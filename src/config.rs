use crate::model::Name;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub scoop: ScoopSection,
    pub winget: WingetSection,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScoopSection {
    pub buckets: Vec<String>,
    pub packages: Vec<Name>,
    pub opts: BTreeMap<Name, PkgOpts>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WingetSection {
    pub packages: Vec<Name>,
}

/// The architectures scoop names in install.json, plus the opt-out.
///
/// A closed set on purpose: `arch = "arm"` used to parse and mean "installed
/// wrong, forever", because nothing ever equals it.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    #[serde(rename = "64bit")]
    X64,
    #[serde(rename = "32bit")]
    X86,
    Arm64,
    /// Never change whatever is installed.
    Keep,
}

impl Arch {
    /// The string scoop writes into install.json. `Keep` names no
    /// architecture: it is the absence of an opinion, not a value.
    pub fn as_scoop(self) -> Option<&'static str> {
        match self {
            Arch::X64 => Some("64bit"),
            Arch::X86 => Some("32bit"),
            Arch::Arm64 => Some("arm64"),
            Arch::Keep => None,
        }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PkgOpts {
    #[serde(default)]
    pub arch: Option<Arch>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    scoop: RawScoopSection,
    #[serde(default)]
    winget: RawWingetSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScoopSection {
    #[serde(default)]
    buckets: Vec<String>,
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    opts: BTreeMap<String, PkgOpts>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWingetSection {
    #[serde(default)]
    packages: Vec<String>,
}

/// Fold raw strings into `Name`s, refusing any two that collide.
///
/// `Name` compares case-insensitively, so `fzf` and `FZF` are one package —
/// but a `Vec` keeps both and the declared loop acts on both, and a map keeps
/// the first key with the last value. Neither is something a user can see in
/// their own file, so it is rejected here rather than resolved silently.
fn fold_names(raw: Vec<String>, what: &str) -> Result<Vec<Name>> {
    let mut seen: BTreeMap<Name, String> = BTreeMap::new();
    let mut out = Vec::with_capacity(raw.len());
    for s in raw {
        let name = Name::new(s.clone());
        if let Some(first) = seen.get(&name) {
            anyhow::bail!(
                "{what} declares the same package twice: {first:?} and {s:?} differ only in case"
            );
        }
        seen.insert(name.clone(), s);
        out.push(name);
    }
    Ok(out)
}

fn fold_opts(raw: BTreeMap<String, PkgOpts>) -> Result<BTreeMap<Name, PkgOpts>> {
    let mut spellings: BTreeMap<Name, String> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for (s, opts) in raw {
        let name = Name::new(s.clone());
        if let Some(first) = spellings.get(&name) {
            anyhow::bail!(
                "[scoop.opts] names the same package twice: {first:?} and {s:?} differ only in case"
            );
        }
        spellings.insert(name.clone(), s);
        out.insert(name, opts);
    }
    Ok(out)
}

pub fn parse(text: &str) -> Result<Config> {
    let raw: RawConfig = toml::from_str(text).context("pkg.toml is not valid")?;
    Ok(Config {
        scoop: ScoopSection {
            buckets: raw.scoop.buckets,
            packages: fold_names(raw.scoop.packages, "[scoop]")?,
            opts: fold_opts(raw.scoop.opts)?,
        },
        winget: WingetSection {
            packages: fold_names(raw.winget.packages, "[winget]")?,
        },
    })
}

pub fn load(path: &Path) -> Result<Config> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_example() {
        let cfg = parse(
            r#"
[scoop]
buckets  = ["main", "extras", "xom11=https://github.com/xom11/scoop-bucket"]
packages = ["fzf", "Bat"]

[scoop.opts]
python = { arch = "64bit" }
kanata = { arch = "keep" }

[winget]
packages = ["Git.Git"]
"#,
        )
        .unwrap();

        assert_eq!(cfg.scoop.packages, vec!["fzf", "bat"]);
        assert_eq!(cfg.scoop.buckets.len(), 3);
        assert_eq!(cfg.scoop.opts[&Name::new("python")].arch, Some(Arch::X64));
        assert_eq!(cfg.scoop.opts[&Name::new("kanata")].arch, Some(Arch::Keep));
        assert_eq!(cfg.winget.packages, vec!["Git.Git"]);

        // The two checks above fold case (`PartialEq<&str> for Name`), so they
        // would not notice `parse` lowercasing a package name on the way in.
        // `.to_string()` goes through `Display`, which does not fold.
        assert_eq!(cfg.scoop.packages[1].to_string(), "Bat");
        assert_eq!(cfg.winget.packages[0].to_string(), "Git.Git");
    }

    #[test]
    fn an_empty_file_is_valid_and_declares_nothing() {
        let cfg = parse("").unwrap();
        assert!(cfg.scoop.packages.is_empty());
        assert!(cfg.winget.packages.is_empty());
    }

    #[test]
    fn a_misspelled_key_is_an_error_not_a_silent_ignore() {
        // deny_unknown_fields: a typo like `packagess` must not read as "you
        // declared nothing", which would make status report every package as a
        // stray and, in Phase 2, offer to remove them.
        let err = parse("[scoop]\npackagess = [\"fzf\"]\n").unwrap_err();
        assert!(
            format!("{err:#}").contains("packagess"),
            "error should name the bad key, got: {err:#}"
        );
    }

    #[test]
    fn a_misspelled_architecture_is_an_error_not_a_permanent_drift() {
        // `arch = "arm"` used to parse cleanly and mean "always wrong", which
        // in Phase 2b is "reinstall on every run".
        let err = parse("[scoop.opts]\npython = { arch = \"arm\" }\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("arm64"),
            "the error must list the real values: {msg}"
        );
    }

    #[test]
    fn two_declared_names_differing_only_in_case_are_rejected() {
        // Name folds case, so these are one package -- but `packages` is a Vec
        // and the declared loop iterates it twice, producing two Install
        // actions for one app and a change_count of 2. Verified against the
        // merged planner.
        let err = parse("[scoop]\npackages = [\"fzf\", \"FZF\"]\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("fzf") && msg.contains("FZF"),
            "name both spellings: {msg}"
        );
    }

    #[test]
    fn a_duplicate_scoop_opts_key_is_rejected_rather_than_silently_clobbered() {
        // TOML cannot express a literal duplicate key, so serde never sees a
        // collision -- the collision is created by Name's folding. Measured
        // behaviour before this fix: one entry, the FIRST key, the LAST value.
        let err =
            parse("[scoop.opts]\npython = { arch = \"64bit\" }\nPython = { arch = \"arm64\" }\n")
                .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("python") && msg.contains("Python"),
            "got: {msg}"
        );
    }

    #[test]
    fn a_duplicate_winget_name_is_rejected_too() {
        let err = parse("[winget]\npackages = [\"Git.Git\", \"git.git\"]\n").unwrap_err();
        assert!(format!("{err:#}").contains("Git.Git"));
    }

    #[test]
    fn distinct_names_are_still_accepted() {
        // The guard must not reject a legitimate config.
        let cfg = parse("[scoop]\npackages = [\"fzf\", \"bat\", \"ripgrep\"]\n").unwrap();
        assert_eq!(cfg.scoop.packages.len(), 3);
    }
}
