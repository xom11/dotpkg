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
}
