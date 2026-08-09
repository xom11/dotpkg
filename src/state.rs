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

/// The path `save` writes to before the atomic rename, derived from the
/// filename `path` actually names rather than a hardcoded `"state.json"`.
///
/// A free function, not inlined into `save`, so it can be pinned by a test
/// without going through a real filesystem write: two different targets in
/// one directory must not collapse onto one temp name.
fn temp_path_for(path: &Path) -> PathBuf {
    let stem = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("state.json");
    path.with_file_name(format!("{stem}.tmp{}", std::process::id()))
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

    /// Release an entry. Returns whether there was one.
    ///
    /// The prune path calls this only **after** `verdict` confirms the package
    /// is gone from disk. Releasing first would leave a still-installed
    /// package that dotpkg has disowned — recoverable only with `dotpkg
    /// adopt`, which does not exist. Releasing last can leave a ghost, and a
    /// ghost is inert: `plan()` consults `owns` only from inside its loop over
    /// *installed* packages.
    pub fn remove(&mut self, backend: &str, name: &Name) -> bool {
        self.0
            .get_mut(backend)
            .map(|m| m.remove(name).is_some())
            .unwrap_or(false)
    }

    /// How dotpkg came to own this package, if it does.
    ///
    /// Read by the executor so that re-recording a package it upgraded puts
    /// back the variant that was already there. Without this, one careless
    /// `set(.., Installed)` in the upgrade path erases every `adopt` decision
    /// on the machine, with no test, no output and no exit code changing.
    pub fn ownership(&self, backend: &str, name: &Name) -> Option<Ownership> {
        self.0.get(backend).and_then(|m| m.get(name)).copied()
    }

    /// Drop entries naming a package that is not installed, returning them.
    ///
    /// Returns nothing if `present` is empty while the state has entries for this
    /// backend. An empty re-scan is a scan error, not evidence that nothing is
    /// installed — dropping everything in that case would wipe the prune fence
    /// without the caller knowing it happened.
    pub fn reconcile(&mut self, backend: &str, present: &[Name]) -> Vec<Name> {
        let Some(m) = self.0.get_mut(backend) else {
            return Vec::new();
        };
        // Refuse to drop everything if the scan came back empty but we have entries.
        if present.is_empty() && !m.is_empty() {
            return Vec::new();
        }
        let dropped: Vec<Name> = m.keys().filter(|n| !present.contains(n)).cloned().collect();
        for n in &dropped {
            m.remove(n);
        }
        dropped
    }

    /// Write the state so that an interrupted write cannot destroy the old one.
    ///
    /// `fs::write` truncates in place: a crash mid-write leaves a truncated
    /// file, and `load_or_empty` then fails for **every** command, `status`
    /// included, with no way back. Phase 2b-2 is the first phase that writes
    /// this file, and it writes it while uninstalling software.
    ///
    /// The temp file is created in the destination directory, not in the
    /// system temp directory, because `rename` is only atomic within one
    /// filesystem.
    ///
    /// The temp filename is unique per writer (using the process ID) to avoid
    /// truncating in place when two writes race. `File::create` truncates an
    /// existing file's inode, so without uniqueness, concurrent writers can
    /// corrupt the prune fence by interleaving writes to the same inode.
    ///
    /// The temp name is derived from `path`'s own filename (`temp_path_for`),
    /// not the literal `"state.json"`: `--state <path>` lets a caller point
    /// this at any filename, and a hardcoded stem would leave an orphan named
    /// `state.json.tmp<pid>` in a directory whose real file is called
    /// something else if a crash lands between the write and the rename.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        let tmp = temp_path_for(path);
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)
                .with_context(|| format!("cannot create {}", tmp.display()))?;
            f.write_all(text.as_bytes())
                .with_context(|| format!("cannot write {}", tmp.display()))?;
            f.sync_all()
                .with_context(|| format!("cannot flush {}", tmp.display()))?;
        }
        // Keep the displaced file: if the rename below is the thing that goes
        // wrong, the previous ownership record is still readable by hand.
        if path.exists() {
            let bak = path.with_extension("json.bak");
            let _ = std::fs::copy(path, &bak);
        }
        match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e).with_context(|| {
                    format!(
                        "cannot move {} into place at {}",
                        tmp.display(),
                        path.display()
                    )
                })
            }
        }
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

    #[test]
    fn an_entry_can_be_released_and_the_release_is_reported() {
        let mut s = State::default();
        s.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);
        assert!(s.remove(SCOOP, &Name::new("AICHAT")), "release folds case");
        assert!(!s.owns(SCOOP, &Name::new("aichat")));
        assert!(
            !s.remove(SCOOP, &Name::new("aichat")),
            "a second release is a no-op"
        );
    }

    #[test]
    fn the_ownership_variant_is_readable_so_an_upgrade_cannot_silently_erase_adopt() {
        // Ownership was written and never read: making `set` discard its
        // argument left the whole suite green. The executor re-writes entries
        // for packages it upgrades, so it must be able to put back what was
        // there.
        let mut s = State::default();
        s.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);
        s.set(SCOOP, &Name::new("fzf"), Ownership::Installed);
        assert_eq!(
            s.ownership(SCOOP, &Name::new("aichat")),
            Some(Ownership::Adopted)
        );
        assert_eq!(
            s.ownership(SCOOP, &Name::new("fzf")),
            Some(Ownership::Installed)
        );
        assert_eq!(s.ownership(SCOOP, &Name::new("nope")), None);
    }

    #[test]
    fn reconcile_drops_a_ghost_and_leaves_a_live_entry_alone() {
        // A run interrupted between a verified uninstall and the state write
        // leaves an entry with no package. It is inert -- plan() consults
        // `owns` only while iterating installed packages -- but it inflates
        // owned_count, so it is cleaned up at the end of the run that made it.
        let mut s = State::default();
        s.set(SCOOP, &Name::new("fzf"), Ownership::Installed);
        s.set(SCOOP, &Name::new("ghost"), Ownership::Installed);

        let dropped = s.reconcile(SCOOP, &[Name::new("fzf")]);

        assert_eq!(dropped, vec![Name::new("ghost")]);
        assert!(s.owns(SCOOP, &Name::new("fzf")));
        assert_eq!(s.owned_count(SCOOP), 1);
    }

    #[test]
    fn a_save_that_replaces_an_existing_file_keeps_the_previous_one_alongside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dotpkg").join("state.json");

        let mut first = State::default();
        first.set(SCOOP, &Name::new("fzf"), Ownership::Installed);
        first.save(&path).unwrap();

        let mut second = State::default();
        second.set(SCOOP, &Name::new("bat"), Ownership::Adopted);
        second.save(&path).unwrap();

        assert_eq!(State::load_or_empty(&path).unwrap(), second);
        let backup = path.with_extension("json.bak");
        assert!(backup.exists(), "the displaced file is kept as {backup:?}");
        assert_eq!(State::load_or_empty(&backup).unwrap(), first);
    }

    #[test]
    fn a_save_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        State::default().save(&path).unwrap();
        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "state.json")
            .collect();
        assert!(strays.is_empty(), "left behind: {strays:?}");
    }

    #[test]
    fn reconcile_refuses_to_disown_everything_when_the_scan_came_back_empty() {
        // An empty re-scan is a scan error, not evidence that nothing is
        // installed. Dropping everything would wipe the prune fence without the
        // caller knowing it happened.
        let mut s = State::default();
        s.set(SCOOP, &Name::new("fzf"), Ownership::Installed);
        s.set(SCOOP, &Name::new("bat"), Ownership::Installed);

        let dropped = s.reconcile(SCOOP, &[]);

        assert!(dropped.is_empty());
        assert!(s.owns(SCOOP, &Name::new("fzf")));
        assert!(s.owns(SCOOP, &Name::new("bat")));
        assert_eq!(s.owned_count(SCOOP), 2);
    }

    #[test]
    fn the_temp_path_is_derived_from_the_real_target_not_hardcoded() {
        // Task 12 adds `--state <path>`, so `path` is no longer always
        // literally "state.json". Before this fix the temp name was the
        // literal string `"state.json.tmp<pid>"` regardless of `path` --
        // harmless while every caller happened to save to a file named
        // state.json, wrong the moment one doesn't, and silent either way
        // because `with_file_name` still lands in the right directory and
        // the later rename still succeeds.
        // Asserted on `file_name()` and `parent()`, not on the rendered
        // whole path. Windows renders `Path::new("/x").with_file_name(..)`
        // as `/x\name`, so a `starts_with("/x/…")` assertion passes on macOS
        // and Linux and fails on Windows -- which is the one platform this
        // tool actually runs on. Found by building this branch on the
        // dogfood machine, not by CI.
        let a = temp_path_for(Path::new("/x/state.json"));
        let b = temp_path_for(Path::new("/x/custom-name.json"));

        let name_of = |p: &PathBuf| p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name_of(&a).starts_with("state.json.tmp"),
            "the temp name must derive from the target's own file name: {a:?}"
        );
        assert!(
            name_of(&b).starts_with("custom-name.json.tmp"),
            "the temp name must derive from the target's own file name: {b:?}"
        );
        assert_eq!(
            a.parent(),
            Path::new("/x/state.json").parent(),
            "the temp file must stay in the destination directory -- rename is \
             only atomic within one filesystem: {a:?}"
        );
        assert_ne!(
            a, b,
            "two different targets in the same directory must not share one temp name"
        );
    }
}
