use crate::model::{fold_map, Name};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Deliberately asymmetric: only scoop can be pinned to content. Flattening
/// these into one shape would let a reader believe a winget entry carries the
/// same guarantee as a scoop one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pin {
    ScoopCommit {
        bucket: String,
        commit: String,
        version: String,
    },
    WingetVersion {
        version: String,
    },
}

impl Pin {
    pub fn version(&self) -> &str {
        match self {
            Pin::ScoopCommit { version, .. } => version,
            Pin::WingetVersion { version } => version,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Lock {
    pub scoop: BTreeMap<Name, Pin>,
    pub winget: BTreeMap<Name, Pin>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScoop {
    bucket: String,
    commit: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWinget {
    version: String,
    pin: String,
}

/// Keyed by `String`, not by `Name`, deliberately: a `BTreeMap<Name, _>` folds
/// case on the way in and merges `[scoop.fzf]` with `[scoop.FZF]` into one
/// entry — the first key, the last value — with nothing said. This is the file
/// Phase 2b-2 uninstalls and reinstalls from, so the pair is folded explicitly
/// below and a collision is refused.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLock {
    #[serde(default)]
    scoop: BTreeMap<String, RawScoop>,
    #[serde(default)]
    winget: BTreeMap<String, RawWinget>,
}

pub fn parse(text: &str) -> Result<Lock> {
    let raw: RawLock = toml::from_str(text).context("pkg.lock is not valid")?;
    let raw_scoop = fold_map(raw.scoop, "pkg.lock [scoop]")?;
    let raw_winget = fold_map(raw.winget, "pkg.lock [winget]")?;

    let mut lock = Lock::default();
    for (name, r) in raw_scoop {
        lock.scoop.insert(
            name,
            Pin::ScoopCommit {
                bucket: r.bucket,
                commit: r.commit,
                version: r.version,
            },
        );
    }
    for (name, r) in raw_winget {
        anyhow::ensure!(
            r.pin == "version-only",
            "winget lock entry {name} has pin={:?}; only \"version-only\" is defined",
            r.pin
        );
        lock.winget
            .insert(name, Pin::WingetVersion { version: r.version });
    }
    Ok(lock)
}

/// An absent lock is not an error — it is a machine that has never run
/// `dotpkg update`. The planner reports every declared package as unlocked.
pub fn load_or_empty(path: &Path) -> Result<Lock> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lock::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Render a lock as the TOML `parse` reads back.
///
/// Hand-written rather than `toml::to_string`: the file is committed, so its
/// diff is read by people, and the aligned `bucket  =` / `commit  =` shape is
/// what `docs/specs/2026-08-08-design.md` documents. A serializer would also
/// have to be taught that `Pin` is two shapes on purpose.
///
/// Keys are quoted whenever the name is not a legal bare TOML key. `Git.Git`
/// is such a name -- an unquoted `[winget.Git.Git]` would parse as three
/// nested tables and the entry would vanish on the way back in.
pub fn render(lock: &Lock) -> String {
    fn key(n: &Name) -> String {
        let s = n.to_string();
        let bare = !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if bare {
            s
        } else {
            format!("{s:?}")
        }
    }
    let mut out = String::new();
    for (name, pin) in &lock.scoop {
        let Pin::ScoopCommit {
            bucket,
            commit,
            version,
        } = pin
        else {
            continue;
        };
        out.push_str(&format!(
            "[scoop.{}]\nbucket  = {:?}\ncommit  = {:?}\nversion = {:?}\n\n",
            key(name),
            bucket,
            commit,
            version
        ));
    }
    for (name, pin) in &lock.winget {
        let Pin::WingetVersion { version } = pin else {
            continue;
        };
        out.push_str(&format!(
            "[winget.{}]\nversion = {:?}\npin     = \"version-only\"\n\n",
            key(name),
            version
        ));
    }
    out
}

/// Write the lock so that an interrupted write cannot destroy the old one, and
/// so that a lock `apply` would refuse never reaches disk.
///
/// The guard runs here rather than only at the call site because `update` is
/// the command a user runs to *fix* an unusable lock: writing another one
/// would leave them with no way forward that does not involve editing TOML by
/// hand. Same temp-then-rename discipline as `State::save`, and for the same
/// reason -- except that `pkg.lock` is committed, so a torn write is also a
/// git conflict.
///
/// **No `.bak`, and that asymmetry with `State::save` is the point.** This used
/// to keep the displaced file as `pkg.lock.bak`. It was removed on 2026-08-12
/// for the reason the paragraph above already contains: `pkg.lock` is
/// **committed**. Git is a strictly better backup of a file under version
/// control than a sibling copy is -- it keeps every version, not the last one --
/// and the copy's only durable effect was a permanently dirty `git status` in
/// the user's dotfiles repository, which each of them would have had to diagnose
/// alone. `State::save` keeps its `.bak` because `state.json` is deliberately
/// NOT committed and is the one file no other mechanism can recover.
pub fn save(lock: &Lock, path: &Path) -> Result<()> {
    crate::apply::lock_coherence_guard(lock)
        .context("refusing to write a pkg.lock that `dotpkg apply` would reject")?;

    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
    }
    let text = render(lock);
    let stem = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("pkg.lock");
    let tmp = path.with_file_name(format!("{stem}.tmp{}", std::process::id()));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("cannot create {}", tmp.display()))?;
        f.write_all(text.as_bytes())
            .with_context(|| format!("cannot write {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("cannot flush {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::Error::new(e).context(format!(
            "cannot move {} into place at {}",
            tmp.display(),
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Name;

    #[test]
    fn parses_both_backends_into_distinct_pin_shapes() {
        let lock = parse(
            r#"
[scoop.fzf]
bucket  = "main"
commit  = "a28d0c5648f1e9d3b7c2a41f6e8b9d0c5a7f3e12"
version = "0.74.1"

[winget."Git.Git"]
version = "2.55.0"
pin     = "version-only"
"#,
        )
        .unwrap();

        assert_eq!(
            lock.scoop[&Name::new("fzf")],
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "a28d0c5648f1e9d3b7c2a41f6e8b9d0c5a7f3e12".into(),
                version: "0.74.1".into()
            }
        );
        assert_eq!(
            lock.winget[&Name::new("Git.Git")],
            Pin::WingetVersion {
                version: "2.55.0".into()
            }
        );
        assert_eq!(lock.scoop[&Name::new("fzf")].version(), "0.74.1");

        // The indexing checks above look up by folded key, so they would find
        // the entry even if `parse` had lowercased it on the way in.
        // `get_key_value` returns the key actually stored; `.to_string()` goes
        // through `Display`, which does not fold.
        let (stored_key, _) = lock.winget.get_key_value(&Name::new("Git.Git")).unwrap();
        assert_eq!(stored_key.to_string(), "Git.Git");
    }

    #[test]
    fn parse_accepts_a_commit_the_guards_reject_and_that_split_is_deliberate() {
        // There is no hex check here, on purpose. A lock too broken to run
        // must still be READABLE, or `status` could not explain it and
        // `update` could not tell the user which entries it is replacing.
        // The refusal lives in `apply::lock_coherence_guard` and in
        // `Scoop::stage`, both of which run before anything is staged.
        let lock =
            parse("[scoop.fzf]\nbucket = \"main\"\ncommit = \"main\"\nversion = \"0.74.1\"\n")
                .expect("parse must not be the layer that refuses this");
        assert_eq!(lock.scoop.len(), 1);

        let err = crate::apply::lock_coherence_guard(&lock).unwrap_err();
        assert!(format!("{err:#}").contains("hex"), "got {err:#}");
    }

    #[test]
    fn a_scoop_entry_without_a_commit_is_rejected() {
        // The commit IS the lock. An entry carrying only a version would look
        // locked while guaranteeing nothing.
        //
        // `bucket` is supplied deliberately: omitting both fields would make this
        // pass only because serde reports missing fields in struct declaration
        // order, so a future field reorder would break the test without breaking
        // the guarantee.
        let err = parse("[scoop.fzf]\nbucket = \"main\"\nversion = \"0.74.1\"\n").unwrap_err();
        assert!(format!("{err:#}").contains("commit"), "got: {err:#}");
    }

    #[test]
    fn an_unknown_winget_pin_kind_is_rejected() {
        let err = parse("[winget.\"Git.Git\"]\nversion = \"2.55.0\"\npin = \"content-hash\"\n")
            .unwrap_err();
        assert!(format!("{err:#}").contains("version-only"), "got: {err:#}");
    }

    #[test]
    fn two_lock_entries_for_one_package_are_rejected_rather_than_merged() {
        // Measured before this fix: `BTreeMap<Name, RawScoop>` folded the two
        // keys into ONE entry, keeping the first key and the LAST value -- so
        // the lock silently pinned fzf to 2.0.0 while still calling it `fzf`,
        // and 2b-2 would reinstall from that. Two different commits are used
        // so the merge cannot be dismissed as harmless.
        let err = parse(
            r#"
[scoop.fzf]
bucket  = "main"
commit  = "1111111111111111111111111111111111111111"
version = "1.0.0"

[scoop.FZF]
bucket  = "main"
commit  = "2222222222222222222222222222222222222222"
version = "2.0.0"
"#,
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("fzf") && msg.contains("FZF"),
            "name both spellings: {msg}"
        );
    }

    #[test]
    fn the_same_collision_is_refused_in_the_winget_map() {
        let err = parse(
            r#"
[winget."Git.Git"]
version = "2.55.0"
pin     = "version-only"

[winget."git.git"]
version = "2.40.0"
pin     = "version-only"
"#,
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Git.Git") && msg.contains("git.git"),
            "name both spellings: {msg}"
        );
    }

    #[test]
    fn distinct_lock_entries_are_still_accepted() {
        // The guard must not reject a legitimate lock.
        let lock = parse(
            r#"
[scoop.fzf]
bucket  = "main"
commit  = "1111111111111111111111111111111111111111"
version = "1.0.0"

[scoop.bat]
bucket  = "main"
commit  = "2222222222222222222222222222222222222222"
version = "0.26.1"
"#,
        )
        .unwrap();
        assert_eq!(lock.scoop.len(), 2);
    }

    #[test]
    fn a_missing_lock_file_is_an_empty_lock_not_an_error() {
        let lock = load_or_empty(Path::new("/definitely/not/here/pkg.lock")).unwrap();
        assert_eq!(lock, Lock::default());
    }

    #[test]
    fn a_lock_that_exists_but_cannot_be_read_is_an_error_not_an_empty_lock() {
        // The counterweight to the test above, and the reason the guard names
        // `NotFound` specifically rather than swallowing every read error.
        // Reading as empty would make `update` re-resolve and rewrite every
        // pin, and make `apply` call every declared package NotLocked -- on a
        // machine whose real pins are sitting right there, merely unreadable.
        // Same rule `State::load_or_empty` follows, and for the same reason.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.lock");
        std::fs::create_dir(&path).unwrap();

        let r = load_or_empty(&path);
        assert!(
            r.is_err(),
            "an unreadable pkg.lock must not be reported as an empty one: {r:?}"
        );
        assert!(
            format!("{:#}", r.unwrap_err()).contains("pkg.lock"),
            "name the file that could not be read"
        );
    }

    #[test]
    fn a_save_creates_the_directory_the_lock_was_asked_to_live_in() {
        // `--lock` may name a path whose parent does not exist yet. Without
        // the create_dir_all, `File::create` fails and `update` loses a run's
        // worth of resolution for a directory it could have made.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deeper").join("pkg.lock");

        save(&Lock::default(), &path).unwrap();

        assert!(path.exists(), "the parent directory must have been created");
        assert_eq!(load_or_empty(&path).unwrap(), Lock::default());
    }

    #[test]
    fn a_name_with_a_hyphen_or_an_underscore_is_still_rendered_as_a_bare_key() {
        // `oh-my-posh` and `win32-openssh` are real scoop packages. A quoted
        // key parses too, so nothing would break -- but the committed file's
        // documented shape is the bare key, and this is the only thing that
        // holds the `-` and `_` half of that predicate. `Git.Git` covers the
        // other half (it MUST be quoted) in the round-trip test above.
        let mut lock = Lock::default();
        for n in ["oh-my-posh", "some_tool"] {
            lock.scoop.insert(
                Name::new(n),
                Pin::ScoopCommit {
                    bucket: "main".into(),
                    commit: "a".repeat(40),
                    version: "1.0.0".into(),
                },
            );
        }

        let text = render(&lock);
        assert!(text.contains("[scoop.oh-my-posh]"), "{text}");
        assert!(text.contains("[scoop.some_tool]"), "{text}");
        assert_eq!(parse(&text).unwrap(), lock, "and it still round-trips");
    }

    #[test]
    fn a_written_lock_reads_back_as_the_same_lock() {
        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("fzf"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "a".repeat(40),
                version: "0.74.2".into(),
            },
        );
        lock.winget.insert(
            Name::new("Git.Git"),
            Pin::WingetVersion {
                version: "2.55.0".into(),
            },
        );

        let text = render(&lock);
        assert_eq!(parse(&text).unwrap(), lock, "round trip: {text}");
        // The documented shape, not merely something that round-trips.
        assert!(text.contains("[scoop.fzf]"), "{text}");
        assert!(text.contains("pin     = \"version-only\""), "{text}");
        // Display case is preserved: `Git.Git` is what a winget user types.
        assert!(text.contains("[winget.\"Git.Git\"]"), "{text}");
    }

    #[test]
    fn a_lock_the_guards_would_reject_is_never_written() {
        // The writer validates its own output with the reader's guard. A Pin
        // that makes `apply` refuse the whole run must not reach disk, because
        // `update` is the command a user runs to FIX that state.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.lock");
        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("fzf"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "main".into(),
                version: "0.74.2".into(),
            },
        );

        // Sequenced so that BOTH named assertions below are reachable under
        // the mutation they exist for. Written as `save(..).unwrap_err()`,
        // deleting the guard call from `save` made this test go red at
        // "called `Result::unwrap_err()` on an `Ok` value" -- before either
        // assertion ran -- so a reader was told the wrong thing about what
        // protects this property.
        let r = save(&lock, &path);
        assert!(
            r.is_err(),
            "a lock the reader's guard rejects must not be written: {r:?}"
        );
        assert!(
            !path.exists(),
            "nothing may be written for a lock that fails the guard"
        );
        let err = r.unwrap_err();
        assert!(format!("{err:#}").contains("hex"), "got {err:#}");
    }

    #[test]
    fn a_save_that_replaces_an_existing_lock_leaves_no_bak_beside_it() {
        // **This test was inverted on 2026-08-12, and the reason is the same
        // sentence that used to justify the opposite.** It read "pkg.lock is
        // committed. A torn write is a git conflict on top of a broken tool,
        // and the previous pins are the only way back" -- and then asserted a
        // `.bak`. But *committed* is exactly why the `.bak` was never the way
        // back: git holds every previous version of this file, not merely the
        // displaced one, so `git checkout pkg.lock` recovers strictly more than
        // the copy ever did. What the copy did produce was a permanently dirty
        // `git status` beside a file the user commits.
        //
        // `State::save` keeps its own `.bak` and its test still asserts one:
        // `state.json` is deliberately not committed and nothing else can
        // recover it. The rule is "a `.bak` is for a file nothing else can
        // recover", not "every write leaves one".
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.lock");
        let mut first = Lock::default();
        first.scoop.insert(
            Name::new("fzf"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "a".repeat(40),
                version: "1".into(),
            },
        );
        let mut second = Lock::default();
        second.scoop.insert(
            Name::new("bat"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "b".repeat(40),
                version: "2".into(),
            },
        );

        save(&first, &path).unwrap();
        save(&second, &path).unwrap();

        // The write itself still has to land, or "no .bak" would be satisfied
        // by a save that did nothing at all.
        assert_eq!(load_or_empty(&path).unwrap(), second);
        let bak = path.with_extension("lock.bak");
        assert!(
            !bak.exists(),
            "pkg.lock is committed; git is the backup, and {bak:?} would only \
             dirty the user's git status"
        );
    }

    #[test]
    fn a_save_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.lock");
        save(&Lock::default(), &path).unwrap();
        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "pkg.lock")
            .collect();
        assert!(strays.is_empty(), "left behind: {strays:?}");
    }
}
