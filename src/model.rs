use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;
use std::hash::{Hash, Hasher};

/// A package name.
///
/// Scoop and winget both resolve names case-insensitively. Comparing them any
/// other way is how `apply` removes the app it has just installed: `pkg.toml`
/// saying `FZF` against `fzf` on disk plans `Install{FZF}` and `Prune{fzf}`,
/// and prune runs last.
///
/// Equality, ordering and hashing use the folded key; `Display` and
/// serialization keep what the user wrote, because `Git.Git` is what a winget
/// user has to type and `git.git` reads like a mistake.
///
/// `Borrow<str>` is deliberately NOT implemented. It would make
/// `map.get("FZF")` compile and silently miss — the exact bug this type exists
/// to make unrepresentable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub struct Name {
    display: String,
    key: String,
}

impl Name {
    pub fn new(s: impl Into<String>) -> Name {
        let display = s.into();
        // ASCII rather than Unicode folding: scoop names come from filenames in
        // a git repository and are ASCII in practice, while `to_lowercase`
        // carries the Turkish dotless-i hazard. Not a trade worth making in a
        // value that decides whether to uninstall something.
        let key = display.to_ascii_lowercase();
        Name { display, key }
    }

    /// The folded form. Compare against data that is already lowercased —
    /// process names, the helper list — with this.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl From<String> for Name {
    fn from(s: String) -> Name {
        Name::new(s)
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Name {
        Name::new(s)
    }
}

impl From<Name> for String {
    fn from(n: Name) -> String {
        n.display
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Name) -> bool {
        self.key == other.key
    }
}

impl Eq for Name {}

/// Comparing against a literal is safe and keeps assertions readable; it folds
/// case like every other comparison on this type. The hazard this type guards
/// against is map *lookup* by `&str`, which `Borrow` would have allowed.
impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.key == other.to_ascii_lowercase()
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Name) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Name {
    fn cmp(&self, other: &Name) -> Ordering {
        self.key.cmp(&other.key)
    }
}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `f.pad`, not `write_str`: `render.rs` prints `{name:<14}` and a
        // Display impl that ignores the formatter drops the padding silently.
        f.pad(&self.display)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub backend: String,
    pub name: Name,
    pub version: String,
    /// Scoop records this in install.json; winget does not expose it.
    pub arch: Option<String>,
    /// Scoop only.
    pub bucket: Option<String>,
    /// Lowercased, extension-stripped basenames of every executable this
    /// package's manifest names. Populated by the backend's scan in Task 3;
    /// empty for a package whose manifest names none.
    pub bins: Vec<String>,
}

pub const SCOOP: &str = "scoop";
pub const WINGET: &str = "winget";

/// Which packages have a live process. Resolved outside the planner, so
/// `dotpkg status` can say "skipped, running" before anything is attempted.
///
/// Two independent signals, because each covers the other's blind spot.
/// `names` catches a process whose executable path cannot be read — an
/// elevated kanata, from a medium-integrity dotpkg. `dirs` catches a package
/// that names no executable in its manifest at all, which on the author's
/// machine is `nodejs` and `rustup`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Running {
    names: BTreeSet<String>,
    dirs: BTreeSet<Name>,
}

impl Running {
    /// `names` must already be lowercased with any `.exe` suffix removed;
    /// `sys::running_processes` is what produces them.
    pub fn new(names: BTreeSet<String>, dirs: BTreeSet<Name>) -> Running {
        Running { names, dirs }
    }

    /// True if anything belonging to this package is alive. `bins` is the
    /// package's declared executables, as `Installed.bins` will carry them
    /// from Task 3.
    ///
    /// Takes the two values rather than an `&Installed` so that `Running`
    /// does not depend on a type Task 2 is about to change.
    ///
    /// Over-matching is deliberate. A false positive costs one `!` line the
    /// user clears by closing an app; a false negative costs the app.
    ///
    /// `bins` entries must already be lowercased with any known extension
    /// stripped, matching `names` above -- `declared_executables` in
    /// `backend::scoop` is what produces them in that form.
    pub fn covers(&self, name: &Name, bins: &[String]) -> bool {
        self.dirs.contains(name)
            || self.names.contains(name.key())
            || bins.iter().any(|b| self.names.contains(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bins(v: &[&str]) -> Vec<String> {
        v.iter().map(|b| b.to_string()).collect()
    }

    #[test]
    fn names_compare_without_regard_to_case() {
        // pkg.toml saying FZF against fzf on disk planned Install{FZF} and
        // Prune{fzf} -- the same app -- and prune runs last.
        assert_eq!(Name::new("FZF"), Name::new("fzf"));
        let mut m = std::collections::BTreeMap::new();
        m.insert(Name::new("FZF"), 1);
        assert_eq!(m.get(&Name::new("fzf")), Some(&1));
    }

    #[test]
    fn a_name_displays_what_the_user_wrote() {
        assert_eq!(Name::new("Git.Git").to_string(), "Git.Git");
        assert_eq!(format!("{:<10}|", Name::new("fzf")), "fzf       |");
    }

    #[test]
    fn a_name_is_a_toml_map_key() {
        #[derive(serde::Deserialize)]
        struct Doc {
            pkgs: std::collections::BTreeMap<Name, String>,
        }
        let d: Doc = toml::from_str("[pkgs]\nFZF = \"a\"\n").unwrap();
        assert_eq!(d.pkgs.get(&Name::new("fzf")), Some(&"a".to_string()));
    }

    #[test]
    fn a_name_round_trips_through_json_preserving_case() {
        let mut m = std::collections::BTreeMap::new();
        m.insert(Name::new("Git.Git"), 1u8);
        let text = serde_json::to_string(&m).unwrap();
        assert!(text.contains("Git.Git"), "got {text}");
        let back: std::collections::BTreeMap<Name, u8> = serde_json::from_str(&text).unwrap();
        assert_eq!(back.get(&Name::new("git.git")), Some(&1));
    }

    #[test]
    fn a_process_named_after_the_package_is_covered() {
        let r = Running::new(BTreeSet::from(["fzf".to_string()]), BTreeSet::new());
        assert!(r.covers(&Name::new("fzf"), &[]));
    }

    #[test]
    fn a_process_the_manifest_names_is_covered_even_when_the_package_is_not() {
        // neovim's executable is nvim.exe. This is the miss that made a running
        // editor plan a clean upgrade.
        let r = Running::new(BTreeSet::from(["nvim".to_string()]), BTreeSet::new());
        assert!(r.covers(&Name::new("neovim"), &bins(&["nvim", "xxd"])));
    }

    #[test]
    fn a_package_naming_no_executable_is_covered_by_its_directory() {
        // nodejs declares env_add_path and no bin anywhere, so the path is the
        // only signal there is.
        let r = Running::new(BTreeSet::new(), BTreeSet::from([Name::new("nodejs")]));
        assert!(r.covers(&Name::new("nodejs"), &[]));
    }

    #[test]
    fn an_idle_package_is_not_covered() {
        let r = Running::new(BTreeSet::from(["chrome".to_string()]), BTreeSet::new());
        assert!(!r.covers(&Name::new("neovim"), &bins(&["nvim", "xxd"])));
    }

    #[test]
    fn coverage_by_directory_ignores_case_like_the_filesystem() {
        let r = Running::new(BTreeSet::new(), BTreeSet::from([Name::new("NodeJS")]));
        assert!(r.covers(&Name::new("nodejs"), &[]));
    }
}
