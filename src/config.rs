use crate::model::Name;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub scoop: ScoopSection,
    #[serde(default)]
    pub winget: WingetSection,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScoopSection {
    #[serde(default)]
    pub buckets: Vec<String>,
    #[serde(default)]
    pub packages: Vec<Name>,
    #[serde(default)]
    pub opts: BTreeMap<Name, PkgOpts>,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WingetSection {
    #[serde(default)]
    pub packages: Vec<Name>,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PkgOpts {
    /// "64bit", "32bit", "arm64", or "keep" to never change what is installed.
    #[serde(default)]
    pub arch: Option<String>,
}

pub fn parse(text: &str) -> Result<Config> {
    toml::from_str(text).context("pkg.toml is not valid")
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
packages = ["fzf", "bat"]

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
        assert_eq!(
            cfg.scoop.opts[&Name::new("python")].arch.as_deref(),
            Some("64bit")
        );
        assert_eq!(
            cfg.scoop.opts[&Name::new("kanata")].arch.as_deref(),
            Some("keep")
        );
        assert_eq!(cfg.winget.packages, vec!["Git.Git"]);
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
}
