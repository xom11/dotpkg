use super::{Backend, Scan};
use crate::lock::Pin;
use crate::model::{Installed, Name, Running, SCOOP};
use crate::sys::{Process, EXECUTABLE_SUFFIXES};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Every executable this manifest declares, normalised to the form
/// `sysinfo` reports a process under: basename, known extension removed,
/// lowercased.
///
/// This walks for the keys instead of modelling the schema. Measured across
/// the author's thirty installed manifests, `bin` appears as a bare string, a
/// list of strings, a mixed list of strings and `[path, alias]` pairs, and
/// nested under `architecture.<arch>`. A depth-first collect handles all four
/// and cannot be broken by a fifth shape nobody has seen.
///
/// Every architecture branch is collected, not just the installed one:
/// `kanata` declares its executables per architecture, and reading only one
/// branch is how the app that costs you the keyboard goes unguarded.
///
/// `shortcuts` is collected alongside `bin` because for `antigravity` it is
/// the only field in the manifest that names an executable at all.
///
/// Over-collection is the safe direction: a spurious entry can only ever
/// cause a package to be skipped.
fn declared_executables(manifest: &serde_json::Value) -> Vec<String> {
    fn add(v: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match v {
            serde_json::Value::String(s) => {
                // Later elements of a bin tuple can be arguments, not names.
                if s.starts_with('-') {
                    return;
                }
                let base = s.rsplit(['\\', '/']).next().unwrap_or(s);
                let stem = base
                    .rsplit_once('.')
                    .filter(|(_, ext)| {
                        EXECUTABLE_SUFFIXES.contains(&ext.to_ascii_lowercase().as_str())
                    })
                    .map(|(stem, _)| stem)
                    .unwrap_or(base);
                if !stem.is_empty() {
                    out.insert(stem.to_ascii_lowercase());
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|e| add(e, out)),
            _ => {}
        }
    }

    fn walk(v: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, val) in m {
                    if k == "bin" || k == "shortcuts" {
                        add(val, out);
                    } else {
                        walk(val, out);
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|e| walk(e, out)),
            _ => {}
        }
    }

    let mut out = std::collections::BTreeSet::new();
    walk(manifest, &mut out);
    out.into_iter().collect()
}

#[derive(Debug, Default, Deserialize)]
struct Install {
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    architecture: Option<String>,
}

/// Strip the extended-length `\\?\` prefix Windows' `canonicalize` adds.
///
/// Per Microsoft's own documentation this form comes back for essentially
/// any existing directory, not only an aliased one, so this is what keeps
/// `running_apps` matching for a plain, unaliased `$SCOOP` on every real
/// Windows machine -- not just the aliased-root case this task set out to
/// fix.
fn strip_extended_prefix(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

/// Resolve aliases so path matching compares the string `sysinfo` reports.
///
/// A path that does not exist is kept as given: a machine with no scoop is a
/// valid state, and `canonicalize` would turn it into an error.
fn resolve_root(root: PathBuf) -> PathBuf {
    let Ok(canon) = std::fs::canonicalize(&root) else {
        return root;
    };
    let s = canon.to_string_lossy();
    let stripped = strip_extended_prefix(&s);
    if stripped.len() < s.len() {
        PathBuf::from(stripped)
    } else {
        canon
    }
}

pub struct Scoop {
    root: PathBuf,
}

impl Scoop {
    pub fn new(root: PathBuf) -> Scoop {
        Scoop {
            root: resolve_root(root),
        }
    }

    /// `$SCOOP` if set, else `%USERPROFILE%\scoop`, matching scoop's own rule.
    pub fn discover() -> Scoop {
        let root = std::env::var_os("SCOOP")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("scoop")))
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join("scoop")))
            .unwrap_or_else(|| PathBuf::from("scoop"));
        Scoop::new(root)
    }

    /// Which installed apps have a live process running out of their own tree.
    ///
    /// Two roots, not one. `apps/<name>/...` is the obvious place; `persist`
    /// is the one that gets forgotten, and `rustup` puts `cargo.exe` under
    /// `persist/rustup/.cargo/bin/`.
    ///
    /// This is the only signal available for a package whose manifest names no
    /// executable (`nodejs`, `rustup`). It cannot replace name matching: a
    /// process at a higher integrity level reports no path at all, and that is
    /// exactly the case — an elevated kanata — where names still work.
    ///
    /// `shims/` is deliberately not a root. A shim is named for the manifest's
    /// alias, which `declared_executables` already collects.
    pub fn running_apps(&self, procs: &[Process]) -> BTreeSet<Name> {
        fn fold(p: &std::path::Path) -> String {
            p.to_string_lossy().replace('\\', "/").to_ascii_lowercase()
        }

        let mut out = BTreeSet::new();
        for parent in ["apps", "persist"] {
            // The trailing separator is what stops `appsbackup` from reading
            // as the `apps` tree.
            let root = format!("{}/", fold(&self.root.join(parent)));
            for p in procs {
                let Some(exe) = p.exe.as_deref() else {
                    continue;
                };
                let Some(rest) = fold(exe).strip_prefix(&root).map(str::to_string) else {
                    continue;
                };
                if let Some(seg) = rest.split('/').next().filter(|s| !s.is_empty()) {
                    out.insert(Name::new(seg));
                }
            }
        }
        out
    }

    /// The `Running` the planner receives: name matching and path matching,
    /// unioned. Each covers the other's blind spot -- an elevated process
    /// reports no `exe` and is caught only by name; a package naming no
    /// executable at all (`nodejs`) is caught only by path -- so a caller that
    /// drops either input silently loses whatever only that half could see.
    ///
    /// This one-line union used to live in `main.rs`, which is not reachable
    /// from a test at all. Assembling it here instead makes it testable on
    /// any OS with fabricated `Process` values, no real process required.
    pub fn running_set(&self, procs: &[Process]) -> Running {
        Running::new(
            procs.iter().map(|p| p.name.clone()).collect(),
            self.running_apps(procs),
        )
    }
}

impl Backend for Scoop {
    fn name(&self) -> &str {
        SCOOP
    }

    fn scan(&self) -> Result<Scan> {
        let apps = self.root.join("apps");
        let entries = match std::fs::read_dir(&apps) {
            Ok(e) => e,
            // No scoop on this machine is a valid state, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Scan::default()),
            Err(e) => return Err(e.into()),
        };

        let mut out = Scan::default();
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                // Same class as an unreadable manifest four lines down: a
                // directory we were told about and cannot look at is a fact
                // about this machine, not an absence.
                Err(e) => {
                    out.warnings
                        .push(format!("cannot read an entry of {}: {e}", apps.display()));
                    continue;
                }
            };
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // apps/scoop is scoop managing itself. Case-insensitive: this is a
            // raw directory name, not a `Name`, and the filesystem that wrote
            // it does not care about case either.
            if name.eq_ignore_ascii_case(SCOOP) {
                continue;
            }

            let current = entry.path().join("current");
            let manifest_path = current.join("manifest.json");
            let manifest_text = match std::fs::read_to_string(&manifest_path) {
                Ok(t) => t,
                // No manifest yet is the ordinary shape of a half-finished
                // install, or of `current` pointing at a version directory
                // scoop is still unpacking. Nothing to tell the user.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                // Anything else -- a permission denial, a dangling junction --
                // is a fact about this machine. Skipping it silently would make
                // an app the user *does* have look uninstalled, which in Phase 2
                // is an offer to reinstall it. Still not fatal: one unreadable
                // directory must not hide the other forty.
                Err(e) => {
                    out.warnings
                        .push(format!("{name}: cannot read manifest.json: {e}"));
                    continue;
                }
            };
            let manifest: serde_json::Value = match serde_json::from_str(&manifest_text) {
                Ok(m) => m,
                Err(e) => {
                    out.warnings
                        .push(format!("{name}: manifest.json is not usable: {e}"));
                    continue;
                }
            };
            let Some(version) = manifest.get("version").and_then(|v| v.as_str()) else {
                out.warnings
                    .push(format!("{name}: manifest.json has no version"));
                continue;
            };
            let bins = declared_executables(&manifest);

            // install.json is absent on apps installed by older scoop versions.
            let install: Install = std::fs::read_to_string(current.join("install.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();

            out.installed.push(Installed {
                backend: SCOOP.to_string(),
                name: Name::new(name),
                version: version.to_string(),
                arch: install.architecture,
                bucket: install.bucket,
                bins,
            });
        }
        Ok(out)
    }
}

impl Scoop {
    /// Recover the exact manifest a lock entry names and write it where scoop
    /// can install from it. Returns the staged path.
    ///
    /// The staged file is named for the **bucket's** spelling of the app, not
    /// the user's, because `scoop install <path>` takes the installed app name
    /// from the filename — so this is what makes the resulting directory
    /// identical to a plain `scoop install <app>`.
    pub fn stage(&self, staging_root: &Path, app: &Name, pin: &Pin) -> Result<PathBuf> {
        let Pin::ScoopCommit {
            bucket,
            commit,
            version,
        } = pin
        else {
            anyhow::bail!("{app}: the scoop lock holds a winget pin; the lock is inconsistent");
        };
        let bucket_dir = self.root.join("buckets").join(bucket);
        anyhow::ensure!(
            bucket_dir.join(".git").exists(),
            "{app}: bucket {bucket:?} is not present at {}",
            bucket_dir.display()
        );
        anyhow::ensure!(
            git_ok(
                &bucket_dir,
                &["cat-file", "-e", &format!("{commit}^{{commit}}")]
            ),
            "{app}: commit {commit} is not in bucket {bucket:?}"
        );

        // git object paths are case-sensitive; Name is not. Try what the user
        // wrote, then the folded form.
        let mut tried: Vec<String> = Vec::new();
        for spelling in [app.to_string(), app.key().to_string()] {
            if tried.contains(&spelling) {
                continue;
            }
            tried.push(spelling.clone());
            let in_repo = format!("bucket/{spelling}.json");
            let Some(text) = git_show(&bucket_dir, commit, &in_repo)? else {
                continue;
            };
            return stage_text(
                staging_root,
                app,
                version,
                &format!("{spelling}.json"),
                &in_repo,
                commit,
                &text,
            );
        }
        // Neither guess is what the bucket calls it. One tree listing finds
        // the real name -- and uses it, rather than only reporting it.
        if let Some(real) = resolve_spelling(&bucket_dir, commit, app.key()) {
            let in_repo = format!("bucket/{real}");
            if let Some(text) = git_show(&bucket_dir, commit, &in_repo)? {
                return stage_text(staging_root, app, version, &real, &in_repo, commit, &text);
            }
        }
        anyhow::bail!("{app}: bucket {bucket:?} at {commit} has no manifest for {tried:?}");
    }
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `Ok(None)` when the path is absent from that commit; `Err` only when git
/// itself could not be run.
fn git_show(dir: &Path, commit: &str, path_in_repo: &str) -> Result<Option<String>> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["show", &format!("{commit}:{path_in_repo}")])
        .output()
        .with_context(|| format!("cannot run git in {}", dir.display()))?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
}

/// Validate a recovered manifest against the lock and write it out. Shared by
/// both routes into staging so the check cannot drift between them.
fn stage_text(
    staging_root: &Path,
    app: &Name,
    version: &str,
    filename: &str,
    in_repo: &str,
    commit: &str,
    text: &str,
) -> Result<PathBuf> {
    let parsed: serde_json::Value = serde_json::from_str(text)
        .with_context(|| format!("{app}: {in_repo} at {commit} is not valid JSON"))?;
    let got = parsed.get("version").and_then(|v| v.as_str()).unwrap_or("");
    anyhow::ensure!(
        got == version,
        "{app}: the lock says {version:?} but {in_repo} at {commit} is {got:?}"
    );
    let dir = staging_root.join(app.key()).join(version);
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let out = dir.join(filename);
    std::fs::write(&out, text).with_context(|| format!("cannot write {}", out.display()))?;
    Ok(out)
}

/// The bucket's own filename for this app, found case-insensitively.
///
/// Costs one tree listing, and only after the two cheap guesses have missed.
/// Returning the real spelling rather than only naming it is what lets
/// `pkg.toml` say `TOOL` while the bucket file is `Tool.json` — without this,
/// the two guesses only work when the user's casing happens to match.
fn resolve_spelling(dir: &Path, commit: &str, app_key: &str) -> Option<String> {
    let listing = Command::new("git")
        .current_dir(dir)
        .args(["ls-tree", "--name-only", commit, "bucket/"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let wanted = format!("{app_key}.json");
    listing
        .lines()
        .map(|l| l.rsplit('/').next().unwrap_or(l))
        .find(|f| f.to_ascii_lowercase() == wanted)
        .map(str::to_string)
}

/// The exact argv for prefetching a staged manifest.
///
/// Pure, and separate from the call that runs it, because the guarantee worth
/// testing here is a property of the argv — that hash verification is never
/// skipped — and not the behaviour of a subprocess no test on this platform
/// can run.
pub fn download_argv(manifest: &Path) -> Vec<String> {
    vec![
        "download".to_string(),
        manifest.to_string_lossy().into_owned(),
    ]
}

impl Scoop {
    /// Measured: `scoop.ps1` cannot be exec'd directly and bare `scoop` from
    /// `PATH` is whatever the user's shell resolves. `shims/scoop.cmd` runs
    /// non-interactively.
    pub fn scoop_exe(&self) -> PathBuf {
        self.root.join("shims").join("scoop.cmd")
    }

    /// Fetch and hash-verify the artifact a staged manifest names, without
    /// installing it. Nothing on the machine changes except scoop's cache.
    ///
    /// The exit code is the only signal this phase has: `scoop download` was
    /// not measured for silent-success behaviour the way `install` and `reset`
    /// were, and inventing a cache-path check against an unmeasured assumption
    /// would be worse than saying so.
    pub fn download(&self, manifest: &Path) -> Result<()> {
        let argv = download_argv(manifest);
        let out = Command::new(self.scoop_exe())
            .args(&argv)
            .output()
            .with_context(|| format!("cannot run {}", self.scoop_exe().display()))?;
        anyhow::ensure!(
            out.status.success(),
            "scoop download failed for {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // This is the branch `resolve_root` hits on every real Windows machine,
    // aliased root or not -- `canonicalize` returns this extended-length form
    // for essentially any existing directory -- and until now nothing in the
    // suite ever passed it a `\\?\`-prefixed string: the aliased-root test in
    // `tests/scoop_scan.rs` only reaches this code on unix, where
    // `canonicalize` never produces this prefix, so the branch has run for
    // every contributor and told nobody anything.
    #[test]
    fn the_extended_length_prefix_windows_adds_is_stripped() {
        assert_eq!(
            strip_extended_prefix(r"\\?\C:\Users\kln\scoop"),
            r"C:\Users\kln\scoop"
        );
    }

    #[test]
    fn a_path_with_no_extended_length_prefix_is_returned_unchanged() {
        assert_eq!(
            strip_extended_prefix(r"C:\Users\kln\scoop"),
            r"C:\Users\kln\scoop"
        );
    }

    #[test]
    fn an_empty_string_does_not_panic() {
        assert_eq!(strip_extended_prefix(""), "");
    }
}
