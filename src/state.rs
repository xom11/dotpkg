use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
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
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct State(BTreeMap<String, BTreeMap<String, Ownership>>);

// dead_code: unreachable from main() until Task 6 adds src/lib.rs — remove then.
#[allow(dead_code)]
impl State {
    pub fn owns(&self, backend: &str, name: &str) -> bool {
        self.0
            .get(backend)
            .map(|m| m.contains_key(name))
            .unwrap_or(false)
    }

    pub fn set(&mut self, backend: &str, name: &str, o: Ownership) {
        self.0
            .entry(backend.to_string())
            .or_default()
            .insert(name.to_string(), o);
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
    use crate::model::SCOOP;

    #[test]
    fn an_absent_file_yields_a_state_that_owns_nothing() {
        let s = State::load_or_empty(Path::new("/definitely/not/here/state.json")).unwrap();
        assert!(!s.owns(SCOOP, "fzf"));
    }

    #[test]
    fn ownership_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("state.json");

        let mut s = State::default();
        s.set(SCOOP, "fzf", Ownership::Installed);
        s.set(SCOOP, "aichat", Ownership::Adopted);
        s.save(&path).unwrap();

        let back = State::load_or_empty(&path).unwrap();
        assert_eq!(back, s);
        assert!(back.owns(SCOOP, "aichat"));
        assert!(!back.owns(SCOOP, "antigravity"));
    }

    #[test]
    fn the_documented_json_shape_is_what_we_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{ "scoop": { "fzf": "installed", "aichat": "adopted" } }"#)
            .unwrap();

        let s = State::load_or_empty(&path).unwrap();
        assert!(s.owns("scoop", "fzf"));
        assert!(s.owns("scoop", "aichat"));
    }
}
