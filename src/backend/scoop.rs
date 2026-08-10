use super::{Backend, ResolveCtx, Scan};
use crate::bucket::{self, BucketChoice};
use crate::lock::Pin;
use crate::model::{Installed, Name, Running, SCOOP};
use crate::sys::{Process, EXECUTABLE_SUFFIXES};
use crate::update::Resolution;
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

/// The rewritten path string, or `None` to keep the original `PathBuf`.
///
/// Split out of `resolve_root` so the decision is observed rather than
/// inferred: the `b.len() == s.len()` arm -- "nothing was stripped, keep
/// `canon` itself rather than rebuilding it from a lossy string, which would
/// corrupt a path that is not valid UTF-8" -- is only reachable through
/// `resolve_root` with a real `\\?\`-prefixed `canonicalize` result (Windows
/// only) or a non-UTF-8 path (a `#[cfg(unix)]` fixture this project does not
/// add, per its own history of losing coverage that way). Through
/// `resolve_root` alone, all three of its mutants survive on every platform a
/// test can run on. As a pure function over `&str`, the same three are killed
/// directly, on every platform, with no `#[cfg]` at all.
fn rewritten(s: &str) -> Option<String> {
    match strip_extended_prefix(s) {
        Cow::Borrowed(b) if b.len() == s.len() => None,
        other => Some(other.into_owned()),
    }
}

/// Resolve aliases so path matching compares the string `sysinfo` reports.
///
/// A path that does not exist is kept as given: a machine with no scoop is a
/// valid state, and `canonicalize` would turn it into an error.
fn resolve_root(root: PathBuf) -> PathBuf {
    let Ok(canon) = std::fs::canonicalize(&root) else {
        return root;
    };
    match rewritten(&canon.to_string_lossy()) {
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

    /// The resolved scoop root this instance was built against, for a caller
    /// (`execute::execute`) that needs it without reaching into a private
    /// field.
    pub fn root(&self) -> &Path {
        &self.root
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
    fn name(&self) -> &'static str {
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
            // The entry itself is unreadable, so there is no name to carry
            // into `opaque` -- unlike the four checks below, which do have
            // one. This one stays a warning only; `plan()` has nothing to
            // compare it against either way.
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
                    out.opaque.push(Name::new(name));
                    continue;
                }
            };
            let manifest: serde_json::Value = match serde_json::from_str(&manifest_text) {
                Ok(m) => m,
                Err(e) => {
                    out.warnings
                        .push(format!("{name}: manifest.json is not usable: {e}"));
                    out.opaque.push(Name::new(name));
                    continue;
                }
            };
            let Some(version) = manifest.get("version").and_then(|v| v.as_str()) else {
                out.warnings
                    .push(format!("{name}: manifest.json has no version"));
                out.opaque.push(Name::new(name));
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

    /// `update`'s resolver. Moved from `update::run` unchanged (Task 13):
    /// same precedence in `bucket::choose_bucket` (existing lock entry, then
    /// `[scoop.opts]`, then a search), same `bucket::resolve_latest` --
    /// deliberately without `--full-history`, see that function's own doc
    /// comment -- and the same fallback warning when no single commit carries
    /// the manifest's current content, now written through `ctx.warnings`
    /// rather than into a `Vec` `update::run` owned directly (`ResolveCtx`'s
    /// own doc comment explains why a sink rather than a wider `Resolution`).
    fn resolve_latest(&self, name: &Name, ctx: &ResolveCtx) -> Resolution {
        let already = ctx.old.scoop.get(name).and_then(|p| match p {
            Pin::ScoopCommit { bucket, .. } => Some(bucket.as_str()),
            Pin::WingetVersion { .. } => None,
        });
        match bucket::choose_bucket(ctx.scoop_root, ctx.declared, name, already) {
            BucketChoice::Ambiguous { candidates } => {
                let names: Vec<String> = candidates.iter().map(|c| c.to_string()).collect();
                Resolution::Failed {
                    why: format!(
                        "{} declared buckets carry it ({}). Say which with \
                         `[scoop.opts] {name} = {{ bucket = \"...\" }}`.",
                        candidates.len(),
                        names.join(", ")
                    ),
                }
            }
            BucketChoice::NotCloned {
                name: bucket_name,
                dir,
            } => Resolution::Failed {
                why: bucket::not_cloned_why("it", &bucket_name, &dir),
            },
            BucketChoice::Undeclared { name: bucket_name } => Resolution::Failed {
                why: bucket::not_declared_why("it", &bucket_name),
            },
            BucketChoice::NotFound { searched, missing } => Resolution::Failed {
                why: bucket::not_found_why("it", &searched, &missing),
            },
            BucketChoice::Chosen {
                name: bucket_name,
                dir,
                tip,
            } => match bucket::resolve_latest(&dir, name, &tip.rev) {
                Ok(Some(latest)) => {
                    if latest.fell_back_to_tip {
                        ctx.warnings.borrow_mut().push(format!(
                            "{name}: no single commit carries this manifest's current \
                             content, so the bucket tip was pinned instead."
                        ));
                    }
                    Resolution::Resolved {
                        pin: Pin::ScoopCommit {
                            // `key()`, not the display spelling -- see the
                            // identical comment this replaced in
                            // `update::run` for why.
                            bucket: bucket_name.key().to_string(),
                            commit: latest.commit,
                            version: latest.version,
                        },
                    }
                }
                Ok(None) => Resolution::Failed {
                    why: format!("bucket {bucket_name} has no manifest for it"),
                },
                Err(e) => Resolution::Failed {
                    why: format!("{e:#}"),
                },
            },
        }
    }

    /// `adopt`'s pin-liveness check. Moved from `adopt::adopt_one` unchanged
    /// (Task 13): the same bucket hint precedence (an existing lock entry,
    /// then `install.json`'s `bucket` -- a legitimate hint only here, because
    /// `adopt` targets packages dotpkg has never touched), the same
    /// `adopt::resolve_installed` search over the bucket's full history --
    /// deliberately WITH `--full-history`, the opposite of `resolve_latest`
    /// above, see `bucket::history`'s doc comment -- and the same shallow-clone
    /// hint on a search that comes back empty.
    ///
    /// `adopt.rs`'s own call site is untouched by this task (Task 15's job is
    /// routing it, and `update`, through `Backend` generically); this exists
    /// so the trait -- which both methods belong to -- has a real
    /// implementation rather than a stub, ready for that wiring.
    fn resolve_installed(&self, inst: &Installed, ctx: &ResolveCtx) -> Resolution {
        let already = ctx.old.scoop.get(&inst.name).and_then(|p| match p {
            Pin::ScoopCommit { bucket, .. } => Some(bucket.as_str()),
            Pin::WingetVersion { .. } => None,
        });
        let hint = already.or(inst.bucket.as_deref());
        let (bucket_name, dir, rev) =
            match bucket::choose_bucket(ctx.scoop_root, ctx.declared, &inst.name, hint) {
                BucketChoice::Chosen { name, dir, tip } => (name, dir, tip.rev),
                BucketChoice::Ambiguous { candidates } => {
                    let names: Vec<String> = candidates.iter().map(|c| c.to_string()).collect();
                    return Resolution::Failed {
                        why: format!(
                            "{} declared buckets carry {} ({}). Say which with \
                             `[scoop.opts] {} = {{ bucket = \"...\" }}`.",
                            candidates.len(),
                            inst.name,
                            names.join(", "),
                            inst.name
                        ),
                    };
                }
                BucketChoice::NotCloned { name, dir } => {
                    return Resolution::Failed {
                        why: bucket::not_cloned_why(&inst.name.to_string(), &name, &dir),
                    };
                }
                BucketChoice::Undeclared { name } => {
                    return Resolution::Failed {
                        why: bucket::not_declared_why(&inst.name.to_string(), &name),
                    };
                }
                BucketChoice::NotFound { searched, missing } => {
                    return Resolution::Failed {
                        why: bucket::not_found_why(&inst.name.to_string(), &searched, &missing),
                    };
                }
            };

        let manifest_path = ctx
            .scoop_root
            .join("apps")
            .join(inst.name.to_string())
            .join("current")
            .join("manifest.json");
        let installed_manifest = match std::fs::read(&manifest_path) {
            Ok(b) => b,
            Err(e) => {
                return Resolution::Failed {
                    why: format!(
                        "cannot read the installed manifest at {}: {e}. Without it there is \
                         nothing to match against, and matching on the version alone would \
                         report a comparison that never happened.",
                        manifest_path.display()
                    ),
                }
            }
        };

        match crate::adopt::resolve_installed(
            &dir,
            &inst.name,
            &inst.version,
            &installed_manifest,
            &rev,
        ) {
            Ok(Some(found)) => {
                // `adopt::run` reads this back via `ctx.matched.take()` right
                // after this call -- it is the ONLY way "which rule found the
                // commit" survives the trip through `Resolution`, which (like
                // `Resolution::Resolved`'s own doc comment explains for
                // `canonical`) is deliberately just `{ pin }` / `{ why }`.
                *ctx.matched.borrow_mut() = Some(found.matched);
                Resolution::Resolved {
                    pin: Pin::ScoopCommit {
                        bucket: bucket_name.key().to_string(),
                        commit: found.commit,
                        version: found.version,
                    },
                }
            }
            Ok(None) => {
                let shallow = if bucket::is_shallow(&dir) {
                    format!(
                        " -- and bucket {bucket_name} is a SHALLOW clone, so most of its \
                         history is not on this machine. `git -C {} fetch --unshallow` \
                         and try again.",
                        dir.display()
                    )
                } else {
                    String::new()
                };
                Resolution::Failed {
                    why: format!(
                        "no commit in bucket {bucket_name} carries {} {}{}",
                        inst.name, inst.version, shallow
                    ),
                }
            }
            Err(e) => Resolution::Failed {
                why: format!("{e:#}"),
            },
        }
    }
}

/// Refuse a string that is about to become one path component.
///
/// `stage()` composes four of them into filesystem paths: `$SCOOP/buckets/
/// <bucket>` and `<staging_root>/<app>/<version>/<commit>`. All four arrive from
/// `pkg.lock`, and Phase 3's `update` fills that file in verbatim from a scoop
/// bucket — an arbitrary third-party git repository. So these are hostile
/// input in the ordinary case, not only under a hand-edited lock.
///
/// This function covers three of the four. `<commit>` is checked by
/// `ensure_commit_hash` instead, whose own doc comment (below) explains why a
/// stricter rule — 40 or 64 lowercase hex, nothing else — is available there
/// and why it is worth having: it makes `<commit>` safe as a path component
/// for free, which is what lets `stage_text` join it without re-checking.
/// `parse_buckets` reuses this same check for a bucket name straight out of
/// `pkg.toml`, before any lock is even involved.
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
///
/// `source` names where `value` came from (`"pkg.lock"`, `"pkg.toml [scoop]
/// buckets"`) and is stitched into the message rather than hardcoded, because
/// a `pkg.toml` typo reported as "the lock's bucket name" sends the reader to
/// the wrong file.
pub fn ensure_plain_component(app: &Name, source: &str, what: &str, value: &str) -> Result<()> {
    let usable = !value.is_empty()
        && value != "."
        && value != ".."
        && !value.starts_with('-')
        && !value.contains(['/', '\\', ':'])
        && !Path::new(value).is_absolute();
    anyhow::ensure!(
        usable,
        "{app}: {what} {value:?} in {source} cannot be used as a path component -- \
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
        ensure_plain_component(app, "pkg.lock", "bucket", bucket)?;
        ensure_plain_component(app, "pkg.lock", "package name", app.key())?;
        ensure_plain_component(app, "pkg.lock", "version", version)?;
        ensure_commit_hash(app, commit)?;

        let bucket_dir = self.root.join("buckets").join(bucket);
        anyhow::ensure!(
            bucket_dir.join(".git").exists(),
            "{app}: bucket {bucket:?} is not present at {}",
            bucket_dir.display()
        );
        anyhow::ensure!(
            crate::bucket::git_ok(
                &bucket_dir,
                &["cat-file", "-e", &format!("{commit}^{{commit}}")]
            ),
            "{app}: commit {commit} is not in bucket {bucket:?}. \
             Run `git -C {} fetch` if the commit was made on another machine.",
            bucket_dir.display()
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
            let Some(text) = crate::bucket::git_show(&bucket_dir, commit, &in_repo)? else {
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
        if let Some(real) = crate::bucket::resolve_spelling(&bucket_dir, commit, app.key()) {
            let in_repo = format!("bucket/{real}");
            if let Some(text) = crate::bucket::git_show(&bucket_dir, commit, &in_repo)? {
                return stage_text(staging_root, app, version, &real, &in_repo, commit, &text);
            }
        }
        anyhow::bail!("{app}: bucket {bucket:?} at {commit} has no manifest for {tried:?}");
    }
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
    // Keyed on the commit as well as the app and version. `install.json`
    // records this path verbatim, so two commits carrying one version sharing
    // a directory means the second staging silently rewrites the manifest the
    // first install still points at. `ensure_commit_hash` has already run, so
    // this component is 40 or 64 lowercase hex and needs no further check.
    let dir = staging_root.join(app.key()).join(version).join(commit);
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let out = dir.join(filename);
    std::fs::write(&out, text).with_context(|| format!("cannot write {}", out.display()))?;
    Ok(out)
}

/// The exact argv for prefetching a staged manifest.
///
/// Pure, and separate from the call that runs it, because the guarantee worth
/// testing here is a property of the argv — that hash verification is never
/// skipped — and not the behaviour of a subprocess no test on this platform
/// can run.
///
/// `-a` is passed whenever an architecture is known: measured, `scoop
/// download` without it fetches the *default* architecture's artifact — two
/// different files for one version — so a prefetch that omits it warms the
/// wrong artifact and the later install reaches the network from inside the
/// mutation window.
pub fn download_argv(manifest: &Path, arch: Option<&str>) -> Vec<String> {
    let mut argv = vec!["download".to_string()];
    if let Some(a) = arch {
        argv.push("-a".to_string());
        argv.push(a.to_string());
    }
    argv.push(manifest.to_string_lossy().into_owned());
    argv
}

/// The argv for adding a bucket. A declaration with no URL names a bucket
/// scoop already knows by name (`main`, `extras`).
pub fn bucket_add_argv(b: &crate::config::BucketDecl) -> Vec<String> {
    let mut argv = vec![
        "bucket".to_string(),
        "add".to_string(),
        b.name.key().to_string(),
    ];
    if let Some(u) = &b.url {
        argv.push(u.clone());
    }
    argv
}

impl Scoop {
    /// Clone every declared bucket that is not on disk. Returns one entry per
    /// bucket that is still missing afterwards.
    ///
    /// Verified by looking for `.git` again, not by the exit code: measured,
    /// `scoop bucket add` exits 0 on a duplicate and on a failure alike.
    ///
    /// Takes `mutator` rather than calling `self.run` directly -- the same
    /// seam `download` joined for the same reason (`crate::execute::Mutator`'s
    /// own doc comment). Without it, no test on any platform could produce
    /// the silent-success shape this function's `.git` re-check exists to
    /// catch, because that shape requires an `Ok` that a real `self.run`
    /// against a nonexistent `scoop.cmd` can never return.
    pub fn clone_missing_buckets(
        &self,
        declared: &crate::config::Config,
        mutator: &dyn crate::execute::Mutator,
    ) -> Vec<(Name, String)> {
        let mut failed = Vec::new();
        for b in &declared.scoop.buckets {
            let dir = self.root.join("buckets").join(b.name.key());
            if dir.join(".git").exists() {
                continue;
            }
            match mutator.bucket_add(b) {
                Ok(_) if dir.join(".git").exists() => {}
                Ok(r) => failed.push((b.name.clone(), tail(&r.stdout))),
                Err(e) => failed.push((b.name.clone(), format!("{e:#}"))),
            }
        }
        failed
    }
}

/// The exact argv for removing an installed app.
///
/// `app.key()`, not the display form: scoop resolves names case-insensitively
/// and the folded key is the one thing that cannot depend on how the user
/// spelled it in `pkg.toml`.
///
/// **Never `-p`/`--purge`.** Measured: without it, `scoop uninstall` keeps
/// everything under `persist`, so the window this opens risks binaries and
/// shims and not the user's data. Adding it would silently change that.
pub fn uninstall_argv(app: &Name) -> Vec<String> {
    vec!["uninstall".to_string(), app.key().to_string()]
}

/// The exact argv for installing a staged manifest.
///
/// `-u`/`--no-update-scoop` keeps a scoop self-update and a bucket `git pull`
/// out of the window between an uninstall and its install. Measured: it is
/// accepted alongside a manifest path.
///
/// `-a` is passed whenever an architecture is known, because `scoop download`
/// without it fetches the *default* architecture's artifact — measured, two
/// different files for one version — and an install that then wants the other
/// one reaches the network from inside the window.
pub fn install_argv(manifest: &Path, arch: Option<&str>) -> Vec<String> {
    let mut argv = vec!["install".to_string(), "-u".to_string()];
    if let Some(a) = arch {
        argv.push("-a".to_string());
        argv.push(a.to_string());
    }
    argv.push(manifest.to_string_lossy().into_owned());
    argv
}

impl Scoop {
    /// Measured: `scoop.ps1` cannot be exec'd directly and bare `scoop` from
    /// `PATH` is whatever the user's shell resolves. `shims/scoop.cmd` runs
    /// non-interactively.
    pub fn scoop_exe(&self) -> PathBuf {
        self.root.join("shims").join("scoop.cmd")
    }
}

/// Turn `scoop download`'s stdout into a result. Pure, so the whole of the
/// prefetch promise is testable without a subprocess.
///
/// The exit code is not a parameter, deliberately: measured on a14, `scoop
/// download` returns 0 for a hash mismatch and for a dead URL, so a signature
/// that accepted a code would invite someone to consult it.
pub fn download_outcome(stdout: &str, manifest: &Path) -> Result<()> {
    match download_verdict(stdout) {
        FetchVerdict::Verified => Ok(()),
        FetchVerdict::HashFailed => anyhow::bail!(
            "hash check failed for {}: {}",
            manifest.display(),
            tail(stdout)
        ),
        FetchVerdict::UrlDead => anyhow::bail!(
            "the manifest's url is gone for {}: {}",
            manifest.display(),
            tail(stdout)
        ),
        FetchVerdict::Unproven => anyhow::bail!(
            "scoop download did not report a verified hash for {} (it exits 0 either way, \
             so this is treated as a failure): {}",
            manifest.display(),
            tail(stdout)
        ),
    }
}

impl crate::execute::Mutator for Scoop {
    fn uninstall(&self, app: &Name) -> Result<crate::execute::CommandReport> {
        self.run(&uninstall_argv(app))
    }
    fn install(
        &self,
        manifest: &Path,
        arch: Option<&str>,
    ) -> Result<crate::execute::CommandReport> {
        self.run(&install_argv(manifest, arch))
    }
    fn download(
        &self,
        manifest: &Path,
        arch: Option<&str>,
    ) -> Result<crate::execute::CommandReport> {
        self.run(&download_argv(manifest, arch))
    }
    fn bucket_add(
        &self,
        bucket: &crate::config::BucketDecl,
    ) -> Result<crate::execute::CommandReport> {
        self.run(&bucket_add_argv(bucket))
    }
}

impl Scoop {
    /// Run scoop and capture everything it said. The exit code is recorded,
    /// not judged.
    fn run(&self, argv: &[String]) -> Result<crate::execute::CommandReport> {
        let out = Command::new(self.scoop_exe())
            .args(argv)
            .output()
            .with_context(|| format!("cannot run {}", self.scoop_exe().display()))?;
        Ok(crate::execute::CommandReport {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Drop ANSI SGR sequences. scoop colours its output, and a colour code
/// between `ERROR` and the rest of a line would hide a failure marker.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What `scoop download` actually did, read from its stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchVerdict {
    Verified,
    HashFailed,
    UrlDead,
    /// Neither a success marker nor a known failure marker. Fail-closed.
    Unproven,
}

/// `scoop download` exits 0 whatever happens — measured on a14 for a hash
/// mismatch and for a 404 — so the verdict comes from stdout.
///
/// **`'<app>' (<version>) was downloaded successfully!` is printed even when
/// the hash check failed.** It is not a success marker, and treating it as one
/// is the single most dangerous mistake available in this function.
///
/// The only success marker is `Checking hash of … ok.`, and its absence is
/// failure rather than doubt: a manifest that declares no `url`/`hash` prints
/// none of these and is refused. That is a known limitation, and refusing is
/// the direction that cannot lose data.
///
/// stderr is deliberately not consulted. Measured: it is non-empty on a
/// *successful* run, carrying non-fatal `Cannot find path …` noise.
pub fn download_verdict(stdout: &str) -> FetchVerdict {
    let clean = strip_ansi(stdout);
    if clean.contains("ERROR Hash check failed!") {
        return FetchVerdict::HashFailed;
    }
    let dead = clean.lines().any(|l| {
        let t = l.trim();
        t.starts_with("ERROR URL ") && t.ends_with(" is not valid")
    });
    if dead {
        return FetchVerdict::UrlDead;
    }
    // Any unrecognized ERROR line means scoop encountered something that failed.
    // This is the fail-closed direction: report unknown errors as Unproven
    // rather than overlook them.
    let has_unrecognized_error = clean.lines().any(|l| l.trim().starts_with("ERROR"));
    if has_unrecognized_error {
        return FetchVerdict::Unproven;
    }
    let verified = clean.lines().any(|l| {
        l.trim_start().starts_with("Checking hash of ") && l.trim_end().ends_with("... ok.")
    });
    if verified {
        FetchVerdict::Verified
    } else {
        FetchVerdict::Unproven
    }
}

/// The last few lines of scoop's stdout, for an error message.
fn tail(stdout: &str) -> String {
    const TAIL_LINES: usize = 20;
    let clean = strip_ansi(stdout);
    let trimmed = clean.trim();
    if trimmed.is_empty() {
        return "scoop printed nothing at all".to_string();
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    match lines.len().checked_sub(TAIL_LINES) {
        Some(skip) if skip > 0 => format!("(last {TAIL_LINES} lines) {}", lines[skip..].join("\n")),
        _ => trimmed.to_string(),
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

    // -- rewritten ------------------------------------------------------

    #[test]
    fn an_ordinary_unprefixed_path_needs_no_rewrite() {
        // The `b.len() == s.len()` arm: nothing was stripped, so `None`
        // tells `resolve_root` to keep `canon` itself rather than rebuild it
        // from a lossy string -- the branch that would corrupt a path that
        // is not valid UTF-8.
        assert_eq!(rewritten(r"C:\Users\kln\scoop"), None);
    }

    #[test]
    fn a_drive_prefixed_path_is_rewritten_to_its_stripped_form() {
        // The prefix really was stripped, so `b.len() < s.len()`: `Some` is
        // the correct answer here, and this is the input `resolve_root`
        // meets on essentially every real Windows machine.
        assert_eq!(
            rewritten(r"\\?\C:\Users\kln\scoop"),
            Some(r"C:\Users\kln\scoop".to_string())
        );
    }

    #[test]
    fn a_unc_prefixed_path_is_rewritten_to_its_restored_form() {
        // The `Cow::Owned` arm: the two leading backslashes were added back
        // on, not merely stripped, so this can never satisfy the
        // `Cow::Borrowed` guard at all -- `Some` regardless of how that
        // guard is mutated.
        assert_eq!(
            rewritten(r"\\?\UNC\server\share\scoop"),
            Some(r"\\server\share\scoop".to_string())
        );
    }

    // -- resolve_root -------------------------------------------------------

    #[test]
    fn resolve_root_strips_any_extended_prefix_and_still_names_the_same_directory() {
        // A real (not fabricated) `canonicalize` round trip -- `rewritten`'s
        // own tests above already pin the string-level stripping cases
        // directly and on every platform; this one's job is only the
        // integration, that `resolve_root` actually runs `canonicalize` and
        // then `rewritten`, on the real, live behaviour of this platform's
        // filesystem.
        //
        // The previous version of this test asserted `got ==
        // std::fs::canonicalize(d.path())` -- the UNSTRIPPED value -- which
        // only ever held on a platform (macOS, every dev machine this suite
        // had run on) where `canonicalize` never adds a `\\?\` prefix in the
        // first place, so there was nothing to strip and the comparison
        // passed vacuously. On real Windows `canonicalize` DOES add that
        // prefix and `resolve_root` DOES strip it -- correctly, that is the
        // whole point of the function -- so the old assertion was checking
        // the opposite of what `resolve_root` exists to do, and it took a
        // real Windows run (Task 16) to show that rather than merely being
        // unable to discriminate a mutant here, as this test's own comment
        // already suspected.
        //
        // Deliberately NOT `#[cfg(windows)]`/`#[cfg(unix)]`: a platform-gated
        // version of this test would have hidden exactly the same defect --
        // green on macOS because the interesting platform never ran it, gated
        // out on Windows because "not our platform" reads as caution rather
        // than as the coverage hole it is. What follows holds unconditionally
        // on both.
        let d = tempfile::tempdir().unwrap();
        let canon = std::fs::canonicalize(d.path()).unwrap();
        let got = resolve_root(d.path().to_path_buf());

        // 1. No extended-length prefix survives, regardless of whether this
        //    platform's `canonicalize` ever produces one.
        assert!(!got.to_string_lossy().starts_with(r"\\?\"), "got {got:?}");

        // 2. `got` still designates the exact same directory `canonicalize`
        //    did -- checked by re-canonicalizing `got` itself and comparing
        //    against the direct call above, rather than comparing `got`
        //    against `canon` textually (that comparison is exactly what
        //    made the old assertion platform-dependent).
        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            canon,
            "resolve_root's output must still resolve to the same directory"
        );
    }

    // -- strip_ansi -----------------------------------------------------

    #[test]
    fn a_lone_escape_character_not_followed_by_a_bracket_is_kept_not_eaten() {
        // The `&&` means BOTH bytes -- ESC and a following `[` -- must be
        // present before anything is treated as a colour code and dropped.
        // Weakening it to `||` treats a bare ESC as the start of a sequence
        // on its own; the loop that follows then consumes the very next
        // character hunting for a terminator that was never there, silently
        // swallowing real content that happened to sit right after it.
        assert_eq!(strip_ansi("\u{1b}A"), "\u{1b}A");
    }

    // -- download_verdict -------------------------------------------------
    //
    // Every string below is scoop 0.5.3's real output, captured on a14 on
    // 2026-08-08 through System.Diagnostics.Process. All three exited 0.

    const OK_CACHED: &str = "INFO  Downloading 'fzf' [arm64]
Loading fzf-0.74.1-windows_arm64.zip from cache
Checking hash of fzf-0.74.1-windows_arm64.zip ... ok.
'fzf' (0.74.1) was downloaded successfully!
";

    const BAD_HASH: &str = "INFO  Downloading 'badhash' [arm64]
Downloading https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip (1.9 MB)...
Checking hash of fzf-0.74.1-windows_arm64.zip ... ERROR Hash check failed!
App:         badhash
URL:         https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip
First bytes: 50 4B 03 04 14 00 08 00
Expected:    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
Actual:      b688ecafa2d1fdb0af3383f25d6d122866c13ad7cc996e9f735bf90e6c75f83f
ERROR
Please try again or create a new issue by using the following link and paste your console output:
https:////
'badhash' (0.74.1) was downloaded successfully!
";

    const DEAD_URL: &str = "INFO  Downloading 'deadurl' [arm64]
The remote server returned an error: (404) Not Found.
ERROR URL https://github.com/xom11/definitely-not-a-real-repo-9f2a/releases/download/v9.9.9/nothing.zip is not valid
";

    #[test]
    fn the_sentence_scoop_prints_after_a_hash_failure_is_not_a_success_marker() {
        // The trap, in one test. Both of these say "was downloaded
        // successfully!" and only one of them verified anything.
        assert!(BAD_HASH.contains("was downloaded successfully!"));
        assert!(OK_CACHED.contains("was downloaded successfully!"));
        assert_eq!(download_verdict(OK_CACHED), FetchVerdict::Verified);
        assert_eq!(download_verdict(BAD_HASH), FetchVerdict::HashFailed);
    }

    #[test]
    fn a_dead_url_is_told_apart_from_a_bad_hash() {
        assert_eq!(download_verdict(DEAD_URL), FetchVerdict::UrlDead);
    }

    #[test]
    fn the_success_sentence_alone_is_not_a_verified_hash() {
        // Important 4: the function's own doc calls treating "was downloaded
        // successfully!" as the success marker the single most dangerous
        // mistake available here. BAD_HASH and OK_CACHED cannot discriminate
        // that mutant -- BAD_HASH trips the earlier ERROR branch, and
        // OK_CACHED contains both strings. This reaches a real shape: a
        // manifest with a `url` and no `hash` prints exactly this sentence,
        // no hash line, no ERROR at all.
        assert_eq!(
            download_verdict("'fzf' (1.0.0) was downloaded successfully!\n"),
            FetchVerdict::Unproven
        );
    }

    #[test]
    fn silence_is_failure_because_scoop_cannot_signal_it_any_other_way() {
        assert_eq!(download_verdict(""), FetchVerdict::Unproven);
        assert_eq!(
            download_verdict("INFO  Downloading 'x' [arm64]\n"),
            FetchVerdict::Unproven
        );
        assert_eq!(
            download_verdict("WARN  'fzf' (0.74.1) is already installed.\n"),
            FetchVerdict::Unproven
        );
    }

    #[test]
    fn ansi_colour_cannot_hide_a_failure() {
        let coloured = BAD_HASH.replace("ERROR", "\u{1b}[31;1mERROR\u{1b}[0m");
        assert_eq!(download_verdict(&coloured), FetchVerdict::HashFailed);
    }

    #[test]
    fn one_verified_url_does_not_excuse_a_second_that_failed() {
        let mixed = "Checking hash of a.zip ... ok.\n\
                     Checking hash of b.zip ... ERROR Hash check failed!\n";
        assert_eq!(download_verdict(mixed), FetchVerdict::HashFailed);
    }

    #[test]
    fn a_verified_line_does_not_excuse_an_unrecognised_error_on_another_url() {
        // A successful hash check does not mean the entire download succeeded.
        // If scoop reports any unrecognized ERROR line, the download is not
        // complete and verified.
        let mixed = "Checking hash of a.zip ... ok.\n\
                     ERROR Something else entirely went wrong for b.zip\n";
        assert_eq!(download_verdict(mixed), FetchVerdict::Unproven);
    }

    #[test]
    fn an_error_url_line_that_does_not_end_in_is_not_valid_is_not_mistaken_for_a_dead_url() {
        // The `&&` requires BOTH the "ERROR URL " prefix and the " is not
        // valid" suffix on one line. Weakening it to `||` would call this
        // UrlDead on the prefix alone -- and a differently-worded ERROR URL
        // line, sharing the prefix but not the suffix, is exactly the shape
        // an unrecognized scoop error takes. The fail-closed path
        // (`has_unrecognized_error`) is what must catch it instead.
        assert_eq!(
            download_verdict("ERROR URL https://example.com/x.zip could not be reached\n"),
            FetchVerdict::Unproven
        );
    }

    #[test]
    fn a_checking_hash_line_that_does_not_end_ok_is_not_mistaken_for_a_verified_hash() {
        // Same trap on the other `&&`: a line starting "Checking hash of "
        // but ending in anything other than "... ok." must not read as
        // verified. `||` would call this Verified on the strength of the
        // prefix alone, which is the single most dangerous mistake this
        // function's own doc comment calls out.
        assert_eq!(
            download_verdict("Checking hash of x.zip ... failed.\n"),
            FetchVerdict::Unproven
        );
    }

    #[test]
    fn tail_returns_short_input_unchanged() {
        let short = "line 1\nline 2\nline 3";
        assert_eq!(tail(short), short);
    }

    #[test]
    fn tail_keeps_only_the_last_20_lines_of_long_input() {
        let noisy: String = (0..500).map(|i| format!("progress {i}\n")).collect();
        let stdout = format!("{noisy}ERROR the final error\n");
        let result = tail(&stdout);
        assert!(
            result.contains("ERROR the final error"),
            "must keep the tail"
        );
        assert!(!result.contains("progress 0\n"), "must drop the head");
        // "last 20 lines" in the output message + error = max 21 lines
        assert!(
            result.lines().count() <= 21,
            "got {} lines",
            result.lines().count()
        );
    }

    #[test]
    fn exactly_twenty_lines_gets_no_last_lines_prefix_glued_on() {
        // `skip > 0` is false exactly when input is at or under TAIL_LINES.
        // At precisely 20 lines, `checked_sub(20)` yields `Some(0)`, so the
        // guard alone decides whether a "(last 20 lines)" prefix gets glued
        // onto text nothing was ever truncated from. Forcing the guard to
        // `true` (or the `>` to `>=`) adds that prefix to input that fits
        // with room to spare; the existing 500-line test never reaches
        // `skip == 0`, so it cannot see this.
        let exactly_20: String = (0..20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = tail(&exactly_20);
        assert_eq!(result, exactly_20);
        assert!(!result.starts_with("(last"), "got {result:?}");
    }

    #[test]
    fn tail_produces_a_non_empty_message_even_when_input_is_empty() {
        let result = tail("");
        assert!(
            !result.trim().is_empty(),
            "empty input must still say something"
        );
        assert!(
            result.contains("nothing"),
            "should explain what happened: {result}"
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
            ensure_plain_component(&Name::new("tool"), "pkg.lock", "version", good)
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
            let err = ensure_plain_component(&Name::new("tool"), "pkg.lock", "version", bad)
                .expect_err("{bad:?} must be refused");
            let msg = format!("{err:#}");
            assert!(
                msg.contains("path component"),
                "say why it was refused: {msg}"
            );
        }
    }

    #[test]
    fn the_message_names_the_source_it_was_given_rather_than_always_the_lock() {
        // Reused for pkg.toml's bucket names at parse time, before any lock
        // is even involved -- a message that always says "the lock's" sends
        // that reader to the wrong file.
        let err = ensure_plain_component(&Name::new("tool"), "pkg.lock", "version", "..")
            .expect_err("must be refused");
        assert!(format!("{err:#}").contains("pkg.lock"), "{err:#}");

        let err = ensure_plain_component(
            &Name::new("xom11"),
            "pkg.toml [scoop] buckets",
            "bucket name",
            "..",
        )
        .expect_err("must be refused");
        let msg = format!("{err:#}");
        assert!(msg.contains("pkg.toml"), "name the file: {msg}");
        assert!(
            !msg.contains("lock"),
            "must not point at the wrong file: {msg}"
        );
    }

    // -- ensure_commit_hash ------------------------------------------

    #[test]
    fn a_real_sha1_or_sha256_commit_hash_is_accepted() {
        let sha1 = "a".repeat(40);
        let sha256 = "a".repeat(64);
        for good in [sha1.as_str(), sha256.as_str()] {
            ensure_commit_hash(&Name::new("tool"), good)
                .unwrap_or_else(|e| panic!("{good:?} must be accepted: {e:#}"));
        }
    }

    #[test]
    fn every_shape_that_is_not_a_lowercase_hex_hash_is_refused() {
        // Both halves of the predicate covered independently: an all-hex
        // string of the wrong length (abc123, 39 chars, 41 chars), and a
        // string of the right length that is not all-hex (uppercase, a
        // non-hex letter, a trailing space). Measured: deleting either half
        // of `ensure_commit_hash`'s `ok` alone left the full suite green,
        // because every fixture that is all-hex is also 40 or 64 characters
        // and every fixture of the wrong length is also non-hex -- so no
        // single case here may be dropped without losing that independence.
        let len39 = "a".repeat(39);
        let len41 = "a".repeat(41);
        let upper40 = "A".repeat(40);
        let nonhex40 = "z".repeat(40);
        let trailing_space = format!("{} ", "a".repeat(39));
        for bad in [
            "abc123",
            len39.as_str(),
            len41.as_str(),
            upper40.as_str(),
            nonhex40.as_str(),
            trailing_space.as_str(),
        ] {
            let err = ensure_commit_hash(&Name::new("tool"), bad).expect_err("must be refused");
            let msg = format!("{err:#}");
            assert!(
                msg.contains("hex"),
                "say what a commit must look like: {bad:?} -> {msg}"
            );
        }
    }

    // -- uninstall_argv / install_argv ------------------------------------

    #[test]
    fn the_uninstall_argv_is_exactly_this_and_never_purges() {
        // -p/--purge deletes the user's persisted data. It is opt-in in scoop
        // and dotpkg never opts in: the uninstall+install window is supposed
        // to risk binaries and shims, not somebody's config.
        assert_eq!(uninstall_argv(&Name::new("FZF")), vec!["uninstall", "fzf"]);
        let argv = uninstall_argv(&Name::new("fzf"));
        assert!(
            !argv.iter().any(|a| a == "-p" || a == "--purge"),
            "{argv:?}"
        );
        assert!(
            !argv.iter().any(|a| a == "-g" || a == "--global"),
            "{argv:?}"
        );
    }

    #[test]
    fn the_install_argv_names_the_staged_path_and_always_passes_no_update_scoop() {
        let m = Path::new("/stage/fzf/0.74.1/fzf.json");
        assert_eq!(
            install_argv(m, Some("arm64")),
            vec!["install", "-u", "-a", "arm64", "/stage/fzf/0.74.1/fzf.json"]
        );
        assert_eq!(
            install_argv(m, None),
            vec!["install", "-u", "/stage/fzf/0.74.1/fzf.json"]
        );
    }

    #[test]
    fn no_argv_this_crate_builds_ever_skips_hash_checking() {
        let m = Path::new("/stage/fzf/0.74.1/fzf.json");
        for argv in [
            install_argv(m, Some("arm64")),
            install_argv(m, None),
            download_argv(m, None),
            uninstall_argv(&Name::new("fzf")),
        ] {
            assert!(
                !argv.iter().any(|a| a == "-s" || a == "--skip-hash-check"),
                "{argv:?}"
            );
        }
    }

    #[test]
    fn every_scoop_argv_is_built_by_a_named_function() {
        // The argv tests above are only honest if there is exactly one
        // construction site per command. An inline scoop invocation would
        // slip past all of them.
        //
        // Assembled rather than written out, so this test's own source does
        // not contain the needle and inflate the count. Patching the expected
        // number instead of the mechanism was tried first and is fragile: any
        // future comment mentioning the literal changes the answer, and two
        // such edits can cancel a real regression out.
        let needle: String = [".args", "(["].concat();
        let src = include_str!("scoop.rs");
        let inline = src.matches(needle.as_str()).count();
        assert_eq!(
            inline, 0,
            "every git argv moved to src/bucket.rs in Phase 3, so scoop.rs now \
             has none; build every SCOOP argv in a *_argv function so the argv \
             tests cover it"
        );
    }
}
