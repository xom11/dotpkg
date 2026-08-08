use super::{Backend, Scan};
use crate::lock::Pin;
use crate::model::{Installed, Name, Running, SCOOP};
use crate::sys::{Process, EXECUTABLE_SUFFIXES};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::borrow::Cow;
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

/// Strip the extended-length prefix Windows' `canonicalize` adds, in the two
/// forms it comes in.
///
/// Per Microsoft's own documentation this form comes back for essentially
/// any existing directory, not only an aliased one, so this is what keeps
/// `running_apps` matching for a plain, unaliased `$SCOOP` on every real
/// Windows machine -- not just the aliased-root case this task set out to
/// fix.
///
/// The UNC form is the one that bites. A network root canonicalises to
/// `\\?\UNC\server\share\scoop`, and dropping only `\\?\` leaves
/// `UNC\server\share\scoop` -- a **relative** path. Every later read then
/// resolves against the process's CWD, `scan()` gets `NotFound`, and
/// `NotFound` is deliberately swallowed as "this machine has no scoop": zero
/// installed packages reported, in silence, on a machine full of software.
/// `\\?\UNC\<rest>` is `\\<rest>`, so the two leading backslashes go back on.
fn strip_extended_prefix(path: &str) -> Cow<'_, str> {
    // Exactly the spelling `canonicalize` emits. A drive path can never
    // collide with it: it would have to be `\\?\UNC` with no colon.
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return Cow::Owned(format!(r"\\{rest}"));
    }
    Cow::Borrowed(path.strip_prefix(r"\\?\").unwrap_or(path))
}

/// Resolve aliases so path matching compares the string `sysinfo` reports.
///
/// A path that does not exist is kept as given: a machine with no scoop is a
/// valid state, and `canonicalize` would turn it into an error.
fn resolve_root(root: PathBuf) -> PathBuf {
    let Ok(canon) = std::fs::canonicalize(&root) else {
        return root;
    };
    // `None` means "nothing was stripped": keep `canon` itself rather than
    // rebuilding it from a lossy string, which would corrupt a path that is
    // not valid UTF-8.
    let stripped = {
        let s = canon.to_string_lossy();
        match strip_extended_prefix(&s) {
            Cow::Borrowed(b) if b.len() == s.len() => None,
            other => Some(other.into_owned()),
        }
    };
    match stripped {
        Some(s) => PathBuf::from(s),
        None => canon,
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

/// Refuse a lock-controlled string that is about to become one path component.
///
/// `stage()` composes three of them into filesystem paths: `$SCOOP/buckets/
/// <bucket>` and `<staging_root>/<app>/<version>`. All three arrive from
/// `pkg.lock`, and Phase 3's `update` fills that file in verbatim from a scoop
/// bucket — an arbitrary third-party git repository. So these are hostile
/// input in the ordinary case, not only under a hand-edited lock.
///
/// `..` is the obvious escape, not the dangerous one. **`Path::join` with an
/// absolute component discards everything to its left**, so a single
/// `version = "/tmp/anywhere"` puts the write wherever it likes without a
/// `..` in sight. Measured before this check: `stage()` returned `Ok` and
/// wrote the manifest completely outside `staging_root`.
///
/// A closed rule — accept only a plain single path component — rather than a
/// blocklist of the escapes someone thought of. A leading `-` is refused for a
/// different reason: these strings are also handed to `git` and to `scoop`,
/// where a leading dash reads as an option rather than a name.
pub fn ensure_plain_component(app: &Name, what: &str, value: &str) -> Result<()> {
    let usable = !value.is_empty()
        && value != "."
        && value != ".."
        && !value.starts_with('-')
        && !value.contains(['/', '\\', ':'])
        && !Path::new(value).is_absolute();
    anyhow::ensure!(
        usable,
        "{app}: the lock's {what} {value:?} cannot be used as a path component -- \
         it must not be empty, `.`, `..`, absolute, start with `-`, or contain \
         `/`, `\\` or `:`"
    );
    Ok(())
}

/// Refuse a `commit` that is not a hash.
///
/// Measured against real git: `git cat-file -e <rev>^{commit}` accepts `main`,
/// `HEAD`, `@` and `refs/heads/main` — it resolves any revision expression,
/// not only an object name. So `commit = "main"` passes the existence check,
/// `git show main:bucket/<app>.json` returns the bucket **tip**, and the only
/// remaining backstop is `stage_text`'s version equality — which a same-version
/// URL/hash correction passes. The lock then means "latest", which
/// `docs/specs/2026-08-08-design.md` calls worse than having no lock at all.
///
/// 40 hex characters for SHA-1, 64 for SHA-256, lowercase as git writes them.
pub fn ensure_commit_hash(app: &Name, commit: &str) -> Result<()> {
    let ok = (commit.len() == 40 || commit.len() == 64)
        && commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    anyhow::ensure!(
        ok,
        "{app}: the lock's commit {commit:?} is not a commit hash -- it must be \
         40 (or 64) lowercase hex characters. A branch or tag name resolves to \
         whatever the bucket points at today, which is not a pin."
    );
    Ok(())
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
        // Before the first `join`, and before any directory is created: the
        // load-bearing property of this whole phase is that the only writes
        // land inside `staging_root`, and `apply.rs` states it as a comment.
        // This is the check behind the comment.
        //
        // `app.key()` stands in for both spellings `stage_text` may use: the
        // display form differs from it only by ASCII case, and every rule
        // above is case-blind.
        ensure_plain_component(app, "bucket", bucket)?;
        ensure_plain_component(app, "package name", app.key())?;
        ensure_plain_component(app, "version", version)?;
        ensure_commit_hash(app, commit)?;

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
            download_failure_detail(
                &String::from_utf8_lossy(&out.stderr),
                &String::from_utf8_lossy(&out.stdout),
            )
        );
        Ok(())
    }
}

/// What to print when `scoop download` exits non-zero.
///
/// A dead URL, a hash mismatch and a network failure all arrive here, and this
/// text is the entire "here is why" half of the phase's promise that nothing
/// happened and the user is told why.
///
/// **Which stream carries that text is unmeasured.** The dogfood run never
/// produced a failing download, and scoop is a PowerShell program whose
/// user-facing output mostly goes to stdout -- so forwarding only stderr risks
/// forwarding an empty string in exactly the case the promise is about. Both
/// are read for that reason, and the fallback exists so the message can never
/// trail off after the colon.
fn download_failure_detail(stderr: &str, stdout: &str) -> String {
    /// Enough to carry scoop's error and the line or two of context around
    /// it, without pasting an entire progress log into one anyhow message.
    const TAIL_LINES: usize = 20;

    if !stderr.trim().is_empty() {
        return stderr.trim().to_string();
    }
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return "scoop printed nothing on either stream".to_string();
    }
    let lines: Vec<&str> = stdout.lines().collect();
    match lines.len().checked_sub(TAIL_LINES) {
        Some(skip) if skip > 0 => format!("(last {TAIL_LINES} lines) {}", lines[skip..].join("\n")),
        _ => stdout.to_string(),
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

    #[test]
    fn a_unc_root_keeps_the_two_backslashes_that_make_it_absolute() {
        // `canonicalize` on a network root returns this form. Stripping only
        // `\\?\` left `UNC\server\share\scoop`, which is RELATIVE: scan()
        // then read it against the CWD, got NotFound, and reported "no scoop
        // on this machine" -- zero packages, silently, which is strictly
        // worse than the raw `\\server\share\scoop` this branch replaced.
        let out = strip_extended_prefix(r"\\?\UNC\server\share\scoop");
        assert_eq!(out, r"\\server\share\scoop");
        assert!(
            out.starts_with(r"\\"),
            "a UNC path without its leading backslashes is a relative path: {out}"
        );
    }

    #[test]
    fn a_drive_root_still_loses_the_whole_prefix() {
        // The UNC branch must not fire for the ordinary case, which has no
        // backslashes to restore.
        let out = strip_extended_prefix(r"\\?\C:\Users\kln\scoop");
        assert_eq!(out, r"C:\Users\kln\scoop");
        assert!(!out.starts_with(r"\\"), "got {out}");
    }

    #[test]
    fn a_directory_merely_named_unc_is_not_mistaken_for_a_network_root() {
        assert_eq!(
            strip_extended_prefix(r"\\?\C:\UNC\scoop"),
            r"C:\UNC\scoop",
            "the UNC branch must match the prefix, not the substring"
        );
    }

    // -- download_failure_detail -----------------------------------------

    #[test]
    fn a_failing_download_reports_stderr_when_scoop_uses_it() {
        assert_eq!(
            download_failure_detail("  hash check failed for fzf  \n", "downloading fzf\n"),
            "hash check failed for fzf"
        );
    }

    #[test]
    fn a_failing_download_falls_back_to_stdout_because_powershell_writes_there() {
        // The case the promise depends on: scoop exits non-zero having said
        // everything it had to say on stdout. Forwarding only stderr made the
        // message read "scoop download failed for <path>: " and stop.
        let detail = download_failure_detail("   \n", "ERROR Hash check failed for fzf\n");
        assert!(
            detail.contains("Hash check failed"),
            "stdout must be forwarded when stderr is blank: {detail:?}"
        );
    }

    #[test]
    fn a_failing_download_that_said_nothing_at_all_still_says_something() {
        let detail = download_failure_detail("", "");
        assert!(
            !detail.trim().is_empty(),
            "the message must never trail off after the colon"
        );
    }

    #[test]
    fn a_long_stdout_is_cut_to_its_tail_where_the_error_is() {
        let noisy: String = (0..500).map(|i| format!("progress {i}\n")).collect();
        let stdout = format!("{noisy}ERROR the url is gone\n");
        let detail = download_failure_detail("", &stdout);
        assert!(detail.contains("ERROR the url is gone"), "got {detail}");
        assert!(!detail.contains("progress 0\n"), "the head must be cut");
        assert!(
            detail.lines().count() <= 21,
            "got {} lines",
            detail.lines().count()
        );
    }

    // -- ensure_plain_component ------------------------------------------

    #[test]
    fn the_ordinary_shapes_of_a_bucket_name_and_a_version_are_accepted() {
        // The guard must not reject a legitimate lock: real versions carry
        // dots, dashes, plus signs and dates.
        for good in [
            "main",
            "extras",
            "xom11",
            "fzf",
            "Git.Git",
            "1.0.0",
            "0.74.1",
            "1.0.0-beta.1+2",
            "2026-08-08",
            "v2_1",
        ] {
            ensure_plain_component(&Name::new("tool"), "version", good)
                .unwrap_or_else(|e| panic!("{good:?} must be accepted: {e:#}"));
        }
    }

    #[test]
    fn every_component_that_could_leave_its_directory_is_refused() {
        // An absolute component is the one that needs no `..` at all:
        // `Path::join` throws away everything to the left of it.
        for bad in [
            "",
            ".",
            "..",
            "../escape",
            r"..\escape",
            "/etc",
            "/tmp/escape",
            r"C:\Windows\Temp",
            r"\\server\share",
            r"sub\dir",
            "sub/dir",
            "c:relative",
            "-oops",
            "--upload-pack=touch",
        ] {
            let err = ensure_plain_component(&Name::new("tool"), "version", bad)
                .expect_err("{bad:?} must be refused");
            let msg = format!("{err:#}");
            assert!(
                msg.contains("path component"),
                "say why it was refused: {msg}"
            );
        }
    }
}
