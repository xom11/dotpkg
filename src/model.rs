use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
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

/// The one error text every collision uses, wherever the two spellings came
/// from.
///
/// It names both spellings and does **not** claim they "differ only in case":
/// an exact repeat (`["fzf", "fzf"]`) reaches this same path, and telling that
/// user to look for a case difference sends them hunting for something that is
/// not there. The case rule is stated as the rule it is, not as a diagnosis of
/// this particular pair.
fn collision(what: &str, first: &str, second: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{what} names the same package twice: {first:?} and {second:?} \
         (package names are compared without regard to case)"
    )
}

/// Fold raw strings into `Name`s, refusing any two that name the same package.
///
/// `Name` compares case-insensitively, so `fzf` and `FZF` are one package —
/// but a `Vec` keeps both and every declared loop then acts on both. Neither
/// spelling is something a user can see is a duplicate in their own file, so
/// it is rejected here rather than resolved silently.
pub fn fold_names(raw: Vec<String>, what: &str) -> Result<Vec<Name>> {
    let mut seen: BTreeMap<Name, String> = BTreeMap::new();
    let mut out = Vec::with_capacity(raw.len());
    for s in raw {
        let name = Name::new(s.clone());
        if let Some(first) = seen.get(&name) {
            return Err(collision(what, first, &s));
        }
        seen.insert(name.clone(), s);
        out.push(name);
    }
    Ok(out)
}

/// The map form of [`fold_names`], for every `BTreeMap<Name, _>` that is built
/// from user- or file-supplied string keys.
///
/// A `BTreeMap<Name, V>` deserialized straight from `String` keys **silently
/// merges** a colliding pair: measured behaviour is one entry, keeping the
/// FIRST key and the LAST value. Every such map in dotpkg decides something
/// destructive — which manifest gets reinstalled (`pkg.lock`), how many
/// packages dotpkg admits to owning (`state.json`), which architecture an app
/// is pinned to (`[scoop.opts]`) — so the merge is refused rather than
/// resolved.
pub fn fold_map<V>(raw: BTreeMap<String, V>, what: &str) -> Result<BTreeMap<Name, V>> {
    let mut spellings: BTreeMap<Name, String> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for (s, value) in raw {
        let name = Name::new(s.clone());
        if let Some(first) = spellings.get(&name) {
            return Err(collision(what, first, &s));
        }
        spellings.insert(name.clone(), s);
        out.insert(name, value);
    }
    Ok(out)
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
    /// `names` must already be lowercased with any suffix in
    /// `sys::EXECUTABLE_SUFFIXES` removed; `sys::running_processes` is what
    /// produces them.
    pub fn new(names: BTreeSet<String>, dirs: BTreeSet<Name>) -> Running {
        Running { names, dirs }
    }

    /// True if anything belonging to this package is alive.
    ///
    /// Takes the whole `Installed` rather than its name and bins separately.
    /// It used to take `(&Name, &[String])`, because `Installed` had not yet
    /// gained a `bins` field when this was written; that field has existed
    /// since Task 2, so the narrower signature no longer earns its keep. It
    /// let a call site copy `cur.name` and quietly forget `cur.bins`, which
    /// compiled and dropped exactly the signal that catches a package whose
    /// live process name differs from its own -- the `neovim` / `nvim.exe`
    /// miss that named this phase. Taking `&Installed` makes that mistake
    /// impossible to write rather than merely absent from today's callers.
    ///
    /// `inst.bins` entries must already be lowercased with any suffix in
    /// `sys::EXECUTABLE_SUFFIXES` removed, matching `names` above --
    /// `declared_executables` in `backend::scoop` is what produces them in
    /// that form.
    ///
    /// Over-matching is deliberate. A false positive costs one `!` line the
    /// user clears by closing an app; a false negative costs the app.
    pub fn covers(&self, inst: &Installed) -> bool {
        self.dirs.contains(&inst.name)
            || self.names.contains(inst.name.key())
            || inst.bins.iter().any(|b| self.names.contains(b))
    }

    /// The name-and-directory halves of `covers`, for a caller that has only a
    /// package name. The `bins` half cannot be consulted here, so this is
    /// strictly weaker: use `covers` wherever an `Installed` is available.
    pub fn covers_name(&self, name: &Name) -> bool {
        self.dirs.contains(name) || self.names.contains(name.key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Name`'s companion trait impls, which the Task 14 mutation run found
    /// were exercised by nothing at all: `PartialEq<&str>`, `PartialOrd` and
    /// `Hash` could each be replaced by a constant and the whole suite stayed
    /// green. `Ord` is the one `BTreeMap` uses and it was already covered.
    ///
    /// Each of these claims to fold case, and that claim is the whole reason
    /// `Name` exists -- so it is pinned here rather than left to the three
    /// doc comments that assert it.
    #[test]
    fn names_fold_case_in_every_comparison_the_type_offers() {
        assert!(Name::new("FZF") == "fzf", "PartialEq<&str> folds case");
        assert!(Name::new("fzf") == "FZF", "in both directions");
        assert!(Name::new("fzf") != "bat");

        // PartialOrd, which `<` uses -- distinct from `Ord::cmp`.
        assert!(
            Name::new("aichat") < Name::new("BAT"),
            "ordering folds case"
        );
        assert!(!(Name::new("BAT") < Name::new("aichat")));
        assert_eq!(
            Name::new("FZF").partial_cmp(&Name::new("fzf")),
            Some(Ordering::Equal),
            "two spellings of one name are neither before nor after each other"
        );

        // Hash, which nothing in the crate uses today -- but a `HashMap<Name,
        // _>` that disagreed with `Eq` about two spellings of one package is
        // exactly the collision `parse` refuses elsewhere.
        use std::collections::hash_map::DefaultHasher;
        let digest = |n: &Name| {
            let mut h = DefaultHasher::new();
            n.hash(&mut h);
            h.finish()
        };
        assert_eq!(
            digest(&Name::new("Git.Git")),
            digest(&Name::new("git.git")),
            "equal names must hash equally"
        );
        assert_ne!(digest(&Name::new("fzf")), digest(&Name::new("bat")));
    }

    fn bins(v: &[&str]) -> Vec<String> {
        v.iter().map(|b| b.to_string()).collect()
    }

    /// An `Installed` with just enough set to exercise `covers`: the fields
    /// it actually reads (`name`, `bins`), plus placeholders for the rest.
    fn installed(name: &str, decl_bins: &[&str]) -> Installed {
        Installed {
            backend: SCOOP.to_string(),
            name: Name::new(name),
            version: "0".to_string(),
            arch: None,
            bucket: None,
            bins: bins(decl_bins),
        }
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
        assert!(r.covers(&installed("fzf", &[])));
    }

    #[test]
    fn a_process_the_manifest_names_is_covered_even_when_the_package_is_not() {
        // neovim's executable is nvim.exe. This is the miss that made a running
        // editor plan a clean upgrade.
        let r = Running::new(BTreeSet::from(["nvim".to_string()]), BTreeSet::new());
        assert!(r.covers(&installed("neovim", &["nvim", "xxd"])));
    }

    #[test]
    fn a_package_naming_no_executable_is_covered_by_its_directory() {
        // nodejs declares env_add_path and no bin anywhere, so the path is the
        // only signal there is.
        let r = Running::new(BTreeSet::new(), BTreeSet::from([Name::new("nodejs")]));
        assert!(r.covers(&installed("nodejs", &[])));
    }

    #[test]
    fn an_idle_package_is_not_covered() {
        let r = Running::new(BTreeSet::from(["chrome".to_string()]), BTreeSet::new());
        assert!(!r.covers(&installed("neovim", &["nvim", "xxd"])));
    }

    #[test]
    fn coverage_by_directory_ignores_case_like_the_filesystem() {
        let r = Running::new(BTreeSet::new(), BTreeSet::from([Name::new("NodeJS")]));
        assert!(r.covers(&installed("nodejs", &[])));
    }

    #[test]
    fn covers_name_checks_both_the_names_half_and_the_dirs_half() {
        // covers_name has no unit test of its own -- dropping either half of
        // its `||` leaves the whole suite green. The `dirs` half is the only
        // signal for a package that names no executable at all (nodejs,
        // rustup on the author's machine), so a caller with only a name (not
        // a full `Installed`) must still be able to see it.
        let by_name = Running::new(BTreeSet::from(["fzf".to_string()]), BTreeSet::new());
        assert!(
            by_name.covers_name(&Name::new("fzf")),
            "the names half must be checked"
        );
        assert!(
            !by_name.covers_name(&Name::new("nodejs")),
            "an unrelated name must not be covered"
        );

        let by_dir = Running::new(BTreeSet::new(), BTreeSet::from([Name::new("nodejs")]));
        assert!(
            by_dir.covers_name(&Name::new("nodejs")),
            "the dirs half must be checked"
        );
        assert!(
            !by_dir.covers_name(&Name::new("fzf")),
            "an unrelated directory must not be covered"
        );
    }
}
