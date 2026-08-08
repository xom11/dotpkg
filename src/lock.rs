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
    pub scoop: BTreeMap<String, Pin>,
    pub winget: BTreeMap<String, Pin>,
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

    let mut lock = Lock::default();
    for (name, r) in raw.scoop {
        lock.scoop.insert(
            name,
            Pin::ScoopCommit {
                bucket: r.bucket,
                commit: r.commit,
                version: r.version,
            },
        );
    }
    for (name, r) in raw.winget {
        anyhow::ensure!(
            r.pin == "version-only",
            "winget lock entry {name:?} has pin={:?}; only \"version-only\" is defined",
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

    #[test]
    fn parses_both_backends_into_distinct_pin_shapes() {
        let lock = parse(
            r#"
[scoop.fzf]
bucket  = "main"
commit  = "a28d0c5648f1"
version = "0.74.1"

[winget."Git.Git"]
version = "2.55.0"
pin     = "version-only"
"#,
        )
        .unwrap();

        assert_eq!(
            lock.scoop["fzf"],
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "a28d0c5648f1".into(),
                version: "0.74.1".into()
            }
        );
        assert_eq!(
            lock.winget["Git.Git"],
            Pin::WingetVersion {
                version: "2.55.0".into()
            }
        );
        assert_eq!(lock.scoop["fzf"].version(), "0.74.1");
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
    fn a_missing_lock_file_is_an_empty_lock_not_an_error() {
        let lock = load_or_empty(Path::new("/definitely/not/here/pkg.lock")).unwrap();
        assert_eq!(lock, Lock::default());
    }
}
