use crate::model::{fold_map, Name};
use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ownership {
    /// dotpkg installed it.
    Installed,
    /// It was already on the machine and the user ran `dotpkg adopt`.
    Adopted,
}

/// backend -> package -> ownership.
///
/// This is the prune fence. A package absent from here is never touched, which
/// is what makes dotpkg safe to install on a machine full of existing software.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct State(BTreeMap<String, BTreeMap<Name, Ownership>>);

/// Hand-written rather than derived: deserializing straight into
/// `BTreeMap<Name, Ownership>` folds case on the way in and merges `"fzf"`
/// with `"FZF"` into one entry, silently. `owned_count` is the number the
/// mass-prune guard prints and compares against zero, so a merge there
/// understates how much dotpkg is about to remove.
impl<'de> Deserialize<'de> for State {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<State, D::Error> {
        let raw: BTreeMap<String, BTreeMap<String, Ownership>> = BTreeMap::deserialize(d)?;
        let mut out = BTreeMap::new();
        for (backend, packages) in raw {
            let folded = fold_map(packages, &format!("state.json [{backend}]"))
                .map_err(|e| serde::de::Error::custom(format!("{e:#}")))?;
            out.insert(backend, folded);
        }
        Ok(State(out))
    }
}

impl State {
    pub fn owns(&self, backend: &str, name: &Name) -> bool {
        self.0
            .get(backend)
            .map(|m| m.contains_key(name))
            .unwrap_or(false)
    }

    pub fn set(&mut self, backend: &str, name: &Name, o: Ownership) {
        self.0
            .entry(backend.to_string())
            .or_default()
            .insert(name.clone(), o);
    }

    /// How many packages dotpkg owns for one backend. The mass-prune guard
    /// needs the number, not the names.
    pub fn owned_count(&self, backend: &str) -> usize {
        self.0.get(backend).map(|m| m.len()).unwrap_or(0)
    }

    pub fn load_or_empty(path: &Path) -> Result<State> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid state.json", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
    }

    /// `%LOCALAPPDATA%\dotpkg\state.json` on Windows; the XDG-ish equivalent
    /// elsewhere so the test suite and development on macOS work unchanged.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_STATE_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("dotpkg").join("state.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Name, SCOOP};

    #[test]
    fn an_absent_file_yields_a_state_that_owns_nothing() {
        let s = State::load_or_empty(Path::new("/definitely/not/here/state.json")).unwrap();
        assert!(!s.owns(SCOOP, &Name::new("fzf")));
    }

    #[test]
    fn ownership_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("state.json");

        let mut s = State::default();
        s.set(SCOOP, &Name::new("fzf"), Ownership::Installed);
        s.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);
        s.save(&path).unwrap();

        let back = State::load_or_empty(&path).unwrap();
        assert_eq!(back, s);
        assert!(back.owns(SCOOP, &Name::new("aichat")));
        assert!(!back.owns(SCOOP, &Name::new("antigravity")));
    }

    #[test]
    fn the_documented_json_shape_is_what_we_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{ "scoop": { "fzf": "installed", "aichat": "adopted" } }"#,
        )
        .unwrap();

        let s = State::load_or_empty(&path).unwrap();
        assert!(s.owns("scoop", &Name::new("fzf")));
        assert!(s.owns("scoop", &Name::new("aichat")));
    }

    #[test]
    fn ownership_is_case_insensitive_because_the_prune_fence_depends_on_it() {
        // state.json is written by dotpkg and read back to decide what may be
        // uninstalled. A case mismatch here reads as "not owned", which is safe,
        // or as a second entry for the same app, which is not.
        let mut s = State::default();
        s.set(SCOOP, &Name::new("FZF"), Ownership::Installed);
        assert!(s.owns(SCOOP, &Name::new("fzf")));
    }

    #[test]
    fn two_entries_for_one_package_are_refused_rather_than_counted_once() {
        // Measured before this fix: `BTreeMap<Name, Ownership>` merged these
        // into ONE entry, so owned_count() reported 1 -- and owned_count is
        // exactly the number the mass-prune guard prints to tell the user how
        // much is about to be removed.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{ "scoop": { "fzf": "installed", "FZF": "adopted" } }"#,
        )
        .unwrap();

        let err = State::load_or_empty(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("fzf") && msg.contains("FZF"),
            "name both spellings: {msg}"
        );
    }

    #[test]
    fn owned_count_reports_per_backend() {
        let mut s = State::default();
        s.set(SCOOP, &Name::new("fzf"), Ownership::Installed);
        s.set(SCOOP, &Name::new("bat"), Ownership::Adopted);
        assert_eq!(s.owned_count(SCOOP), 2);
        assert_eq!(s.owned_count("winget"), 0);
    }
}
