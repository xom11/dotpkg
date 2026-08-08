# dotpkg Phase 2a — a plan you can trust

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `dotpkg status` print a plan that is true — case-insensitive name matching, running-process detection that works for real packages, a prune that consults it, and architecture drift reported — so Phase 2b's `apply` executes a plan already checked against a real machine.

**Architecture:** No new command and no write path. Three model types gain final shape in Task 1 (`Name`, `Running`, `Installed.bins`); Task 2 sweeps them through every layer; Tasks 3–7 populate and use them. The planner stays pure and still receives the running set as an input.

**Tech Stack:** Rust 2021, `serde`, `serde_json`, `toml`, `anyhow`, `clap`, `sysinfo`. No new dependencies.

**Design:** `docs/specs/2026-08-08-phase2a-design.md`

## Global Constraints

Copied from the spec and from `docs/plans/2026-08-08-phase1-status-scoop.md`. Every task's requirements implicitly include these.

- **Never degrade silently.** A missing lock entry, missing bucket, or missing commit is a reported condition, never a fallback to "latest".
- **The planner is pure.** No I/O, no network, no subprocess in `src/plan.rs`. Enforced by `the_planner_source_performs_no_io` in `tests/planner.rs`, which is an **allowlist over `use` lines** (`crate::`, `std::collections`, `super::`) plus a ban on any `std::` path other than `std::collections` *anywhere in the file, including prose*. Do not write `std::fs` in a comment in that file.
- **Subprocesses are for mutation only.** Phase 2a mutates nothing, so it spawns no subprocess at all.
- **Prune only ever touches a package named in `state.json`.**
- **Scoop helper packages are `dark`, `innounp`, `7zip`, `lessmsi`.** Never reported as strays.
- **Test layers 1 and 2 must run on Linux and macOS.** No test added here may require Windows or a real scoop install.
- Rust edition 2021, `rust-version = "1.85"`.
- **Every new test needs a negative control with recorded evidence.** Break the code the way the task names, paste the red output into the task's step, restore, confirm green. Phase 1 shipped three tests that passed for reasons unrelated to their names; this is the step that would have caught all three.
- CI gate is `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all`.

## Baseline

`main` at `e798916`, 37 tests green: 15 unit + 16 `tests/planner.rs` + 6 `tests/scoop_scan.rs`.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/model.rs` | Shared types: `Name`, `Installed`, `Running`, backend consts | Modify (Task 1) |
| `src/config.rs` | Parse `pkg.toml`; the `Arch` vocabulary | Modify (Tasks 2, 6) |
| `src/lock.rs` | Parse `pkg.lock` | Modify (Task 2) |
| `src/state.rs` | `state.json`, the prune fence | Modify (Task 2) |
| `src/sys.rs` | Process table: names and executable paths | Modify (Task 4) |
| `src/backend/scoop.rs` | `scan()`, executable extraction, path matching | Modify (Tasks 2, 3, 4, 7) |
| `src/plan.rs` | The pure planner | Modify (Tasks 2, 5, 6) |
| `src/render.rs` | `Plan` → terminal text | Modify (Tasks 2, 6) |
| `src/main.rs` | Wiring | Modify (Tasks 2, 4) |
| `tests/planner.rs` | Layer 1 | Modify (Tasks 2, 5, 6) |
| `tests/scoop_scan.rs` | Layer 2 | Modify (Tasks 2, 3, 4, 7) |
| `tests/fixtures/scoop-manifests/*.json` | Real manifests from a14 | Create (Task 3) |
| `docs/dogfood-phase2a-2026-08-08.md` | Dogfood record | Create (Task 8) |

---

### Task 1: The `Name` and `Running` model types

Both types land in their final shape here, **additively**. `Installed` is not touched in this task: adding a field to it would break `src/backend/scoop.rs`, `src/plan.rs`, `src/render.rs` and both test files in the same commit, and Task 1 could not end green on its own. Task 2 changes `Installed` as part of the sweep that fixes every call site at once.

That is also why `Running::covers` takes a name and a bin list rather than an `&Installed`: the narrower interface keeps `Running` independent of `Installed`, so its signature is final from here and never churns.

**Files:**
- Modify: `src/model.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct Name` with `Name::new(impl Into<String>) -> Name`, `Name::key(&self) -> &str`, `Display`, `Ord`, `Hash`, `PartialEq<&str>`, serde via `String`
  - `pub struct Running` with `Running::new(BTreeSet<String>, BTreeSet<Name>) -> Running` and `Running::covers(&self, name: &Name, bins: &[String]) -> bool`
  - `pub struct Installed`, `pub const SCOOP`, `pub const WINGET` — **unchanged in this task**

- [ ] **Step 1: Write the failing tests**

Append to `src/model.rs`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib model`
Expected: FAIL — `cannot find type Name in this scope`

- [ ] **Step 3: Write the types**

Replace the whole of `src/model.rs` above the test module with:

```rust
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

// `Installed` stays exactly as it is in this task. Task 2 changes its `name`
// to a `Name` and adds `bins`, together with every call site those two edits
// break -- doing it here would leave the crate uncompilable at this commit.

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
    pub fn covers(&self, name: &Name, bins: &[String]) -> bool {
        self.dirs.contains(name)
            || self.names.contains(name.key())
            || bins.iter().any(|b| self.names.contains(b))
    }
}
```

Leave `Installed`, `SCOOP` and `WINGET` in the file exactly as they are.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib model`
Expected: PASS, 9 tests

- [ ] **Step 5: Negative control — prove the case test can fail**

Change `Name::new` to `let key = display.clone();`.

Run: `cargo test --lib model`
Expected: FAIL — `names_compare_without_regard_to_case`, `a_name_is_a_toml_map_key`, `a_name_round_trips_through_json_preserving_case` and `coverage_by_directory_ignores_case_like_the_filesystem` all red. Paste the failure output into this step.

Restore `to_ascii_lowercase()`, re-run, confirm green.

- [ ] **Step 6: Negative control — prove `f.pad` is load-bearing**

Change `Display` to `f.write_str(&self.display)`.

Run: `cargo test --lib model a_name_displays_what_the_user_wrote`
Expected: FAIL — got `fzf|`, wanted `fzf       |`. Paste it in.

Restore `f.pad`, re-run, confirm green.

- [ ] **Step 7: Commit**

```bash
git add src/model.rs
git commit -m "Add Name and Running, the two types the planner was missing"
```

---

### Task 2: Wire `Name` and `Running` through every layer

Mechanical, wide, and the one task where a missed call site is a silent regression. The whole suite must be green at the end.

**Files:**
- Modify: `src/config.rs`, `src/lock.rs`, `src/state.rs`, `src/plan.rs`, `src/render.rs`, `src/main.rs`, `src/backend/scoop.rs`
- Modify: `tests/planner.rs`, `tests/scoop_scan.rs`

**Interfaces:**
- Consumes: `crate::model::{Name, Running}` from Task 1
- Produces:
  - `Installed { backend: String, name: Name, version: String, arch: Option<String>, bucket: Option<String>, bins: Vec<String> }` — this task is where those two field changes land, together with every call site they break
  - `Config.scoop.packages: Vec<Name>`, `Config.scoop.opts: BTreeMap<Name, PkgOpts>`, `Config.winget.packages: Vec<Name>`
  - `Lock.scoop: BTreeMap<Name, Pin>`, `Lock.winget: BTreeMap<Name, Pin>`
  - `State::owns(&self, backend: &str, name: &Name) -> bool`, `State::set(&mut self, backend: &str, name: &Name, o: Ownership)`
  - `pub fn plan(declared: &Config, lock: &Lock, installed: &[Installed], state: &State, running: &Running) -> Plan`
  - Every `Action` variant's `name` field is a `Name`

- [ ] **Step 1: Write the failing test**

Append to `tests/planner.rs`:

```rust
#[test]
fn a_case_difference_between_pkg_toml_and_disk_is_not_two_packages() {
    // Before Name, this planned Install{FZF} then Prune{fzf} -- the same app --
    // and because prune runs last, apply would have uninstalled what it had
    // just installed. Verified against the merged Phase 1 planner.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("fzf"), Ownership::Installed);

    let p = plan(
        &config::parse("[scoop]\npackages = [\"FZF\"]\n").unwrap(),
        &lock::parse("[scoop.FZF]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"0.74.1\"\n").unwrap(),
        &[installed("fzf", "0.74.1")],
        &state,
        &Running::default(),
    );
    assert!(p.actions.is_empty(), "expected no action, got {:?}", p.actions);
}

#[test]
fn a_case_difference_in_scoop_opts_still_finds_the_package() {
    // Second instance of the same bug, in a different map.
    let cfg = config::parse(
        "[scoop]\npackages = [\"python\"]\n\n[scoop.opts]\nPython = { arch = \"64bit\" }\n",
    )
    .unwrap();
    assert!(cfg.scoop.opts.get(&Name::new("python")).is_some());
}
```

Add to that file's imports: `use dotpkg::model::{Installed, Name, Running, SCOOP};`

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test planner a_case_difference`
Expected: FAIL to compile — `unresolved import dotpkg::model::Name` is not the error; `Name` exists. The error is `mismatched types` at `state.set(SCOOP, &Name::new("fzf"), ...)` because `set` still takes `&str`.

- [ ] **Step 3: Change `Installed` in `src/model.rs`**

Everything else in this task follows from these two edits. Expect the crate not to compile again until Step 9 is done; that is the shape of this task.

```rust
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
```

- [ ] **Step 4: Change the type in `src/config.rs`**

```rust
use crate::model::Name;
```

and in the structs:

```rust
pub struct ScoopSection {
    #[serde(default)]
    pub buckets: Vec<String>,
    #[serde(default)]
    pub packages: Vec<Name>,
    #[serde(default)]
    pub opts: BTreeMap<Name, PkgOpts>,
}

pub struct WingetSection {
    #[serde(default)]
    pub packages: Vec<Name>,
}
```

`Config`, `ScoopSection`, `WingetSection` and `PkgOpts` all derive `PartialEq, Eq`; `Name` provides both, so nothing else changes.

- [ ] **Step 5: Change the type in `src/lock.rs`**

```rust
use crate::model::Name;
```

```rust
pub struct Lock {
    pub scoop: BTreeMap<Name, Pin>,
    pub winget: BTreeMap<Name, Pin>,
}

struct RawLock {
    #[serde(default)]
    scoop: BTreeMap<Name, RawScoop>,
    #[serde(default)]
    winget: BTreeMap<Name, RawWinget>,
}
```

In `parse`, the winget loop's error message interpolates `name`, which now goes through `Display`. Change `"winget lock entry {name:?} has pin=..."` to `"winget lock entry {name} has pin=..."` — `Name`'s `Debug` prints both fields and would read as noise.

The existing test `parses_both_backends_into_distinct_pin_shapes` indexes `lock.scoop["fzf"]`. `Index` on `BTreeMap<Name, _>` needs a `&Name`, so change those to `lock.scoop[&Name::new("fzf")]` and `lock.winget[&Name::new("Git.Git")]`, and add `use crate::model::Name;` to the test module.

- [ ] **Step 6: Change the type in `src/state.rs`**

```rust
use crate::model::Name;

pub struct State(BTreeMap<String, BTreeMap<Name, Ownership>>);

impl State {
    pub fn owns(&self, backend: &str, name: &Name) -> bool {
        self.0.get(backend).map(|m| m.contains_key(name)).unwrap_or(false)
    }

    pub fn set(&mut self, backend: &str, name: &Name, o: Ownership) {
        self.0
            .entry(backend.to_string())
            .or_default()
            .insert(name.clone(), o);
    }
    // load_or_empty, save, default_path unchanged
}
```

Update that file's three tests: `s.owns(SCOOP, &Name::new("fzf"))`, `s.set(SCOOP, &Name::new("fzf"), Ownership::Installed)`, and so on. Add `use crate::model::Name;` to the test module.

Add one test that pins the new behaviour:

```rust
#[test]
fn ownership_is_case_insensitive_because_the_prune_fence_depends_on_it() {
    // state.json is written by dotpkg and read back to decide what may be
    // uninstalled. A case mismatch here reads as "not owned", which is safe,
    // or as a second entry for the same app, which is not.
    let mut s = State::default();
    s.set(SCOOP, &Name::new("FZF"), Ownership::Installed);
    assert!(s.owns(SCOOP, &Name::new("fzf")));
}
```

- [ ] **Step 7: Change the types in `src/plan.rs`**

Imports become:

```rust
use crate::config::Config;
use crate::lock::Lock;
use crate::model::{Installed, Name, Running, SCOOP, WINGET};
use crate::state::State;
use std::collections::BTreeSet;
```

Both lines start with `crate::` or `std::collections`, so the purity allowlist is satisfied unchanged.

Every `Action` variant's `name: String` becomes `name: Name`. The signature becomes:

```rust
pub fn plan(
    declared: &Config,
    lock: &Lock,
    installed: &[Installed],
    state: &State,
    running: &Running,
) -> Plan {
```

Inside, four changes:

```rust
    let declared_scoop: BTreeSet<&Name> = declared.scoop.packages.iter().collect();
    // (the `let running: BTreeSet<&str> = ...` line is deleted)
```

```rust
        let current = installed
            .iter()
            .find(|i| i.backend == SCOOP && &i.name == name);
```
stays as written — `&i.name == name` is now `Name == Name` and folds case.

```rust
                if running.covers(&cur.name, &cur.bins) {
```
replaces `if running.contains(name.as_str())`. Note it now consults the **installed** record rather than the declared name, so it sees that record's `bins` as soon as Task 3 fills them.

```rust
        if SCOOP_HELPERS.contains(&inst.name.key()) {
```
replaces `SCOOP_HELPERS.contains(&inst.name.as_str())`. The helper list stays lowercase, so a bucket shipping `7Zip` cannot slip past.

`state.owns(SCOOP, &inst.name)` needs no change; its parameter type moved with it.

- [ ] **Step 8: Change `src/render.rs` and `src/main.rs`**

`render.rs` needs no edit to its body: `{name:<14}` goes through `Display`, which Task 1 implemented with `f.pad`. Its tests use `name: "ripgrep".into()`, which resolves through `From<&str> for Name`.

`main.rs` builds the running set:

```rust
            let procs = dotpkg::sys::running_process_names();
            let running = dotpkg::model::Running::new(procs.into_iter().collect(), Default::default());
```

replacing `let running = dotpkg::sys::running_process_names();`. Task 4 replaces this again once path matching exists.

- [ ] **Step 9: Change `src/backend/scoop.rs`**

One line, in the `out.installed.push(Installed { ... })` literal:

```rust
                name: Name::new(name),
                ...
                bins: Vec::new(),
```

and add `Name` to its import: `use crate::model::{Installed, Name, SCOOP};`. `bins` is filled in Task 3.

- [ ] **Step 10: Update `tests/planner.rs` and `tests/scoop_scan.rs`**

In `tests/planner.rs`:
- imports become `use dotpkg::model::{Installed, Name, Running, SCOOP};`
- the `installed()` helper gains `bins: Vec::new(),`
- every `plan(...)` call's fifth argument `&[]` becomes `&Running::default()`
- the one call passing `&["fzf".into()]` becomes
  `&Running::new(std::collections::BTreeSet::from(["fzf".to_string()]), Default::default())`
- `state.set(SCOOP, "aichat", Ownership::Adopted)` becomes `state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted)`
- `name: "fzf".into()` inside expected `Action`s needs no change

In `tests/scoop_scan.rs`, nothing changes: `assert_eq!(got[0].name, "bat")` works through `PartialEq<&str> for Name`, and `a.name.cmp(&b.name)` works through `Ord`.

- [ ] **Step 11: Run the whole suite**

Run: `cargo test --all`
Expected: PASS — 39 tests (15 unit + 9 new model + 18 planner + 6 scoop_scan, minus overlaps; the exact total is whatever green looks like, and **no test may be deleted to get there**). Confirm the count went up, not down.

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 12: Negative control**

Change `Name::new` to `let key = display.clone();`.

Run: `cargo test --test planner a_case_difference_between_pkg_toml_and_disk_is_not_two_packages`
Expected: FAIL with `expected no action, got [Install { .. name: FZF .. }, Prune { .. name: fzf .. }]` — the original bug, reproduced through the real planner. Paste it in.

Restore, re-run, confirm green.

- [ ] **Step 13: Commit**

```bash
git add src tests
git commit -m "Compare package names case-insensitively everywhere

pkg.toml saying FZF against fzf on disk planned Install{FZF} and
Prune{fzf} for one app, prune last. Fixed as a type rather than at six
call sites, because Phase 3 and 4 each add writers."
```

---

### Task 3: Extract the executables a manifest declares

**Files:**
- Modify: `src/backend/scoop.rs`
- Modify: `tests/scoop_scan.rs`
- Create: `tests/fixtures/scoop-manifests/{fzf,age,python,kanata,neovim,nodejs}.json`

**Interfaces:**
- Consumes: `Installed.bins` from Task 1
- Produces: `Installed.bins` populated by `Scoop::scan()`

- [ ] **Step 1: Create the fixtures**

These are the real manifests from a14, trimmed to the fields `scan()` reads. Verified: trimming does not change what the extractor returns for any of the six.

`tests/fixtures/scoop-manifests/fzf.json` — the common case, a bare string:

```json
{
  "version": "0.74.2",
  "bin": "fzf.exe",
  "architecture": { "64bit": {}, "arm64": {} }
}
```

`tests/fixtures/scoop-manifests/age.json` — a list of strings:

```json
{
  "version": "1.3.1",
  "bin": ["age.exe", "age-inspect.exe", "age-keygen.exe", "age-plugin-batchpass.exe"],
  "architecture": { "64bit": {} }
}
```

`tests/fixtures/scoop-manifests/neovim.json` — backslash paths, and the package name is not the executable name:

```json
{
  "version": "0.12.4",
  "bin": ["bin\\nvim.exe", "bin\\xxd.exe"],
  "architecture": { "64bit": {}, "arm64": {} }
}
```

`tests/fixtures/scoop-manifests/python.json` — a list mixing bare strings with `[path, alias]` pairs:

```json
{
  "version": "3.14.5",
  "bin": [
    ["python.exe", "python3"],
    "Lib\\idlelib\\idle.bat",
    ["Lib\\idlelib\\idle.bat", "idle3"]
  ],
  "architecture": { "64bit": {}, "32bit": {}, "arm64": {} },
  "env_add_path": ["Scripts", "."]
}
```

`tests/fixtures/scoop-manifests/kanata.json` — no top-level `bin` at all; pairs nested under each architecture, plus `shortcuts`:

```json
{
  "version": "1.12.0",
  "architecture": {
    "64bit": {
      "bin": [
        ["kanata_windows_tty_winIOv2_x64.exe", "Kanata"],
        ["kanata_windows_tty_winIOv2_cmd_allowed_x64.exe", "Kanata-cmd"]
      ],
      "shortcuts": [
        ["kanata_windows_gui_winIOv2_x64.exe", "Kanata"],
        ["kanata_windows_gui_winIOv2_cmd_allowed_x64.exe", "Kanata-cmd"]
      ]
    },
    "arm64": {
      "bin": [
        ["kanata_windows_tty_winIOv2_arm64.exe", "Kanata"],
        ["kanata_windows_tty_winIOv2_cmd_allowed_arm64.exe", "Kanata-cmd"]
      ],
      "shortcuts": [
        ["kanata_windows_gui_winIOv2_arm64.exe", "Kanata"],
        ["kanata_windows_gui_winIOv2_cmd_allowed_arm64.exe", "Kanata-cmd"]
      ]
    }
  }
}
```

`tests/fixtures/scoop-manifests/nodejs.json` — names no executable anywhere:

```json
{
  "version": "26.5.1",
  "architecture": { "64bit": {}, "arm64": {} },
  "env_add_path": ["bin", "."]
}
```

- [ ] **Step 2: Write the failing test**

Append to `tests/scoop_scan.rs`:

```rust
/// Install a real manifest from `tests/fixtures/scoop-manifests` as an app.
fn app_from_fixture(root: &Path, name: &str, arch: &str) {
    let dir = root.join("apps").join(name).join("current");
    fs::create_dir_all(&dir).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scoop-manifests")
        .join(format!("{name}.json"));
    fs::copy(&src, dir.join("manifest.json"))
        .unwrap_or_else(|e| panic!("copying {}: {e}", src.display()));
    fs::write(
        dir.join("install.json"),
        format!(r#"{{"bucket":"main","architecture":"{arch}"}}"#),
    )
    .unwrap();
}

fn bins_of(root: &Path, name: &str) -> Vec<String> {
    let scan = Scoop::new(root.to_path_buf()).scan().unwrap();
    assert!(scan.warnings.is_empty(), "got {:?}", scan.warnings);
    let inst = scan
        .installed
        .into_iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("{name} not scanned"));
    inst.bins
}

#[test]
fn a_bare_string_bin_yields_one_executable() {
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "fzf", "arm64");
    assert_eq!(bins_of(dir.path(), "fzf"), vec!["fzf"]);
}

#[test]
fn a_list_of_strings_yields_all_of_them() {
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "age", "64bit");
    assert_eq!(
        bins_of(dir.path(), "age"),
        vec!["age", "age-inspect", "age-keygen", "age-plugin-batchpass"]
    );
}

#[test]
fn a_path_is_reduced_to_its_basename_and_the_package_name_is_not_assumed() {
    // The finding: the package is `neovim`, the process is `nvim.exe`.
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "neovim", "arm64");
    assert_eq!(bins_of(dir.path(), "neovim"), vec!["nvim", "xxd"]);
}

#[test]
fn a_mixed_list_of_strings_and_alias_pairs_yields_both_forms() {
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "python", "64bit");
    assert_eq!(
        bins_of(dir.path(), "python"),
        vec!["idle", "idle3", "python", "python3"]
    );
}

#[test]
fn bins_under_every_architecture_and_shortcuts_are_all_collected() {
    // kanata is why this matters. It declares no top-level bin; its executable
    // is kanata_windows_tty_winIOv2_arm64.exe and only the shim alias is
    // `Kanata`. Reading just the installed architecture, or just `bin`, leaves
    // the keyboard remapper unprotected -- and losing it costs the keyboard on
    // the machine you would need to fix it.
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "kanata", "arm64");
    assert_eq!(
        bins_of(dir.path(), "kanata"),
        vec![
            "kanata",
            "kanata-cmd",
            "kanata_windows_gui_winiov2_arm64",
            "kanata_windows_gui_winiov2_cmd_allowed_arm64",
            "kanata_windows_gui_winiov2_cmd_allowed_x64",
            "kanata_windows_gui_winiov2_x64",
            "kanata_windows_tty_winiov2_arm64",
            "kanata_windows_tty_winiov2_cmd_allowed_arm64",
            "kanata_windows_tty_winiov2_cmd_allowed_x64",
            "kanata_windows_tty_winiov2_x64",
        ]
    );
}

#[test]
fn a_manifest_naming_no_executable_yields_none_rather_than_guessing() {
    // nodejs uses env_add_path. Inventing `nodejs` here would be a guess that
    // never matches the real process, which is `node`.
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "nodejs", "arm64");
    assert_eq!(bins_of(dir.path(), "nodejs"), Vec::<String>::new());
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test --test scoop_scan`
Expected: FAIL — every new test reports `left: []`, because `scan()` still writes `bins: Vec::new()`.

- [ ] **Step 4: Implement the extractor**

In `src/backend/scoop.rs`, delete the `Manifest` struct and add:

```rust
/// Every executable this manifest declares, normalised to the form
/// `sysinfo` reports a process under: basename, known extension removed,
/// lowercased.
///
/// This walks for the keys instead of modelling the schema. Measured across
/// the author's thirty installed manifests, `bin` appears as a bare string, a
/// list of strings, a mixed list of strings and `[path, alias]` pairs, and
/// nested under `architecture.<arch>`. A depth-first collect handles all four
/// and cannot be broken by a fifth shape nobody has seen.
///
/// Every architecture branch is collected, not just the installed one:
/// `kanata` declares its executables per architecture, and reading only one
/// branch is how the app that costs you the keyboard goes unguarded.
///
/// `shortcuts` is collected alongside `bin` because for `antigravity` it is
/// the only field in the manifest that names an executable at all.
///
/// Over-collection is the safe direction: a spurious entry can only ever
/// cause a package to be skipped.
fn declared_executables(manifest: &serde_json::Value) -> Vec<String> {
    const EXECUTABLE_SUFFIXES: &[&str] = &["exe", "cmd", "bat", "ps1", "com"];

    fn add(v: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match v {
            serde_json::Value::String(s) => {
                // Later elements of a bin tuple can be arguments, not names.
                if s.starts_with('-') {
                    return;
                }
                let base = s.rsplit(['\\', '/']).next().unwrap_or(s);
                let stem = base
                    .rsplit_once('.')
                    .filter(|(_, ext)| {
                        EXECUTABLE_SUFFIXES.contains(&ext.to_ascii_lowercase().as_str())
                    })
                    .map(|(stem, _)| stem)
                    .unwrap_or(base);
                if !stem.is_empty() {
                    out.insert(stem.to_ascii_lowercase());
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|e| add(e, out)),
            _ => {}
        }
    }

    fn walk(v: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, val) in m {
                    if k == "bin" || k == "shortcuts" {
                        add(val, out);
                    } else {
                        walk(val, out);
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|e| walk(e, out)),
            _ => {}
        }
    }

    let mut out = std::collections::BTreeSet::new();
    walk(manifest, &mut out);
    out.into_iter().collect()
}
```

In `scan()`, replace the typed parse with a `Value` parse so the same document serves both purposes:

```rust
            let manifest: serde_json::Value = match serde_json::from_str(&manifest_text) {
                Ok(m) => m,
                Err(e) => {
                    out.warnings
                        .push(format!("{name}: manifest.json is not usable: {e}"));
                    continue;
                }
            };
            let Some(version) = manifest.get("version").and_then(|v| v.as_str()) else {
                out.warnings
                    .push(format!("{name}: manifest.json has no version"));
                continue;
            };
            let bins = declared_executables(&manifest);
```

and in the pushed `Installed`: `version: version.to_string(),` and `bins,`.

- [ ] **Step 5: Run the tests**

Run: `cargo test --all`
Expected: PASS. The pre-existing `a_manifest_that_cannot_be_read_is_skipped_with_a_warning_not_in_silence` still passes: its fixture is `{"description":"x"}`, which now trips the `has no version` branch and still produces exactly one warning naming `halfwritten`.

- [ ] **Step 6: Negative control — three separate breaks**

Each must be applied alone, observed red, and restored.

1. In `walk`, drop the recursion: replace `walk(val, out)` with `{}`.
   Run: `cargo test --test scoop_scan bins_under_every_architecture`
   Expected: FAIL, kanata yields `[]`.
2. In `walk`, collect `bin` only: change `if k == "bin" || k == "shortcuts"` to `if k == "bin"`.
   Run: `cargo test --test scoop_scan bins_under_every_architecture`
   Expected: FAIL, the four `..._gui_...` entries are missing.
3. In `add`, drop the basename step: replace the `rsplit` line with `let base = s.as_str();`.
   Run: `cargo test --test scoop_scan a_path_is_reduced_to_its_basename`
   Expected: FAIL, `neovim` yields `["bin\\nvim", "bin\\xxd"]`.

Paste all three failures in.

- [ ] **Step 7: Commit**

```bash
git add src/backend/scoop.rs tests/scoop_scan.rs tests/fixtures
git commit -m "Read the executables a scoop manifest declares

The running-process guard compared package names to process names, so it
never fired for neovim (nvim), ripgrep (rg), 7zip (7z) or kanata. Tested
against six real manifests from a14 covering every bin shape they use."
```

---

### Task 4: Match a running process by its executable path

**Files:**
- Modify: `src/sys.rs`, `src/backend/scoop.rs`, `src/main.rs`
- Modify: `tests/scoop_scan.rs`

**Interfaces:**
- Consumes: `Running` from Task 1
- Produces:
  - `pub struct Process { pub name: String, pub exe: Option<PathBuf> }` in `src/sys.rs`
  - `pub fn running_processes() -> Vec<Process>` replacing `running_process_names`
  - `pub fn Scoop::running_apps(&self, procs: &[sys::Process]) -> BTreeSet<Name>`

- [ ] **Step 1: Write the failing test**

Append to `tests/scoop_scan.rs`:

```rust
use dotpkg::model::Name;
use dotpkg::sys::Process;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn proc(name: &str, exe: Option<PathBuf>) -> Process {
    Process {
        name: name.to_string(),
        exe,
    }
}

#[test]
fn a_process_running_out_of_an_app_directory_names_that_app() {
    // nodejs is why this exists: its manifest names no executable at all, so
    // the path is the only signal there is.
    let root = PathBuf::from("/tmp/dpk-root");
    let got = Scoop::new(root.clone()).running_apps(&[proc(
        "node",
        Some(root.join("apps/nodejs/current/node.exe")),
    )]);
    assert_eq!(got, BTreeSet::from([Name::new("nodejs")]));
}

#[test]
fn the_persist_tree_counts_too_because_rustup_lives_there() {
    // rustup's env_add_path is `.cargo\bin`, which scoop puts under
    // persist/rustup, outside apps entirely.
    let root = PathBuf::from("/tmp/dpk-root");
    let got = Scoop::new(root.clone()).running_apps(&[proc(
        "cargo",
        Some(root.join("persist/rustup/.cargo/bin/cargo.exe")),
    )]);
    assert_eq!(got, BTreeSet::from([Name::new("rustup")]));
}

#[test]
fn a_process_with_no_readable_path_is_not_an_error() {
    // sysinfo reports None for a process at a higher integrity level. That is
    // the case name matching covers, so this must simply contribute nothing.
    let root = PathBuf::from("/tmp/dpk-root");
    let got = Scoop::new(root).running_apps(&[proc("kanata", None)]);
    assert!(got.is_empty());
}

#[test]
fn a_process_outside_the_scoop_tree_names_nothing() {
    let root = PathBuf::from("/tmp/dpk-root");
    let got = Scoop::new(root).running_apps(&[proc(
        "node",
        Some(PathBuf::from("/usr/local/bin/node")),
    )]);
    assert!(got.is_empty());
}

#[test]
fn a_sibling_directory_with_a_shared_prefix_is_not_the_apps_tree() {
    // `.../scoop/appsbackup/x.exe` must not read as app `backup`.
    let root = PathBuf::from("/tmp/dpk-root");
    let got = Scoop::new(root.clone()).running_apps(&[proc(
        "x",
        Some(root.join("appsbackup/backup/x.exe")),
    )]);
    assert!(got.is_empty(), "got {got:?}");
}

#[test]
fn path_matching_folds_case_like_the_filesystem() {
    let root = PathBuf::from("/tmp/DPK-Root");
    let got = Scoop::new(root).running_apps(&[proc(
        "node",
        Some(PathBuf::from("/tmp/dpk-root/Apps/NodeJS/current/node.exe")),
    )]);
    assert_eq!(got, BTreeSet::from([Name::new("nodejs")]));
}

#[test]
fn windows_paths_with_backslashes_and_a_resolved_version_dir_match() {
    // The only shape that occurs on the real machine, and the one a test
    // written on a Mac is most likely to miss: separators are backslashes, and
    // sysinfo may report the version directory the `current` junction resolves
    // to rather than `current` itself. Either way the segment after `apps` is
    // the app name.
    let root = PathBuf::from(r"C:\Users\kln\scoop");
    let got = Scoop::new(root).running_apps(&[proc(
        "nvim",
        Some(PathBuf::from(
            r"C:\Users\kln\scoop\apps\neovim\0.12.4\bin\nvim.exe",
        )),
    )]);
    assert_eq!(got, BTreeSet::from([Name::new("neovim")]));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test scoop_scan running_apps`
Expected: FAIL to compile — `no method named running_apps`, `unresolved import dotpkg::sys::Process`.

- [ ] **Step 3: Rewrite `src/sys.rs`**

```rust
use std::path::PathBuf;
use sysinfo::System;

/// One live process, as much of it as this session is allowed to see.
pub struct Process {
    /// Lowercased base name without a trailing `.exe`: "Kanata.exe" -> "kanata".
    pub name: String,
    /// `None` when the executable path cannot be read — a process at a higher
    /// integrity level, or a kernel process. Name matching is what covers
    /// those, which is why the two signals are kept separate.
    pub exe: Option<PathBuf>,
}

/// The running process table.
///
/// This is an input to the planner rather than something the planner
/// discovers, which is what lets `dotpkg status` say "skipped, running" before
/// anything is attempted.
pub fn running_processes() -> Vec<Process> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .map(|p| {
            let n = p.name().to_string_lossy().to_ascii_lowercase();
            Process {
                name: n.strip_suffix(".exe").unwrap_or(&n).to_string(),
                exe: p.exe().map(|e| e.to_path_buf()),
            }
        })
        .collect()
}
```

`to_ascii_lowercase` rather than `to_lowercase`, to fold the same way `Name::key` does.

- [ ] **Step 4: Add `running_apps` to `src/backend/scoop.rs`**

```rust
use crate::sys::Process;
use std::collections::BTreeSet;

impl Scoop {
    /// Which installed apps have a live process running out of their own tree.
    ///
    /// Two roots, not one. `apps/<name>/...` is the obvious place; `persist`
    /// is the one that gets forgotten, and `rustup` puts `cargo.exe` under
    /// `persist/rustup/.cargo/bin/`.
    ///
    /// This is the only signal available for a package whose manifest names no
    /// executable (`nodejs`, `rustup`). It cannot replace name matching: a
    /// process at a higher integrity level reports no path at all, and that is
    /// exactly the case — an elevated kanata — where names still work.
    ///
    /// `shims/` is deliberately not a root. A shim is named for the manifest's
    /// alias, which `declared_executables` already collects.
    pub fn running_apps(&self, procs: &[Process]) -> BTreeSet<Name> {
        fn fold(p: &std::path::Path) -> String {
            p.to_string_lossy().replace('\\', "/").to_ascii_lowercase()
        }

        let mut out = BTreeSet::new();
        for parent in ["apps", "persist"] {
            // The trailing separator is what stops `appsbackup` from reading
            // as the `apps` tree.
            let root = format!("{}/", fold(&self.root.join(parent)));
            for p in procs {
                let Some(exe) = p.exe.as_deref() else {
                    continue;
                };
                let Some(rest) = fold(exe).strip_prefix(&root).map(str::to_string) else {
                    continue;
                };
                if let Some(seg) = rest.split('/').next().filter(|s| !s.is_empty()) {
                    out.insert(Name::new(seg));
                }
            }
        }
        out
    }
}
```

- [ ] **Step 5: Wire it in `src/main.rs`**

```rust
            let scoop = Scoop::discover();
            let scan = scoop.scan()?;
            let procs = dotpkg::sys::running_processes();
            let running = dotpkg::model::Running::new(
                procs.iter().map(|p| p.name.clone()).collect(),
                scoop.running_apps(&procs),
            );
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --all`
Expected: PASS. Then `cargo fmt --check && cargo clippy --all-targets -- -D warnings`.

- [ ] **Step 7: Negative control — two breaks**

1. Remove `"persist"` from the root list.
   Run: `cargo test --test scoop_scan the_persist_tree_counts_too`
   Expected: FAIL — the only signal rustup has, gone.
2. Drop the trailing separator: `let root = fold(&self.root.join(parent));`
   Run: `cargo test --test scoop_scan a_sibling_directory_with_a_shared_prefix`
   Expected: FAIL — `appsbackup/backup/x.exe` reads as app `backup`.

Paste both in, restore, confirm green.

- [ ] **Step 8: Commit**

```bash
git add src/sys.rs src/backend/scoop.rs src/main.rs tests/scoop_scan.rs
git commit -m "Detect a running package by its executable path as well as its name

nodejs and rustup name no executable in their manifests, so bin
extraction alone cannot see them. Path matching and name matching have
non-overlapping blind spots; both are needed."
```

---

### Task 5: Prune consults the running set

Four lines. The largest hole in the planner and the one `docs/phase2-notes.md` does not record.

**Files:**
- Modify: `src/plan.rs`
- Modify: `tests/planner.rs`

**Interfaces:**
- Consumes: `Running::covers` from Task 1
- Produces: no signature change

- [ ] **Step 1: Write the failing test**

Append to `tests/planner.rs`:

```rust
#[test]
fn a_running_package_is_never_pruned() {
    // The prune loop did not consult `running` at all -- not a mismatched
    // comparison, an absent one. Verified against the merged Phase 1 planner
    // with an exact name match, which had no excuse to miss:
    //   kanata running + owned + removed from pkg.toml  ->  Prune{kanata}
    // Prune is worse than the upgrade case it sits beside: an upgrade puts the
    // app back, a prune does not.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("kanata"), Ownership::Installed);

    let p = plan(
        &config::parse("[scoop]\npackages = []\n").unwrap(),
        &lock::Lock::default(),
        &[installed("kanata", "1.12.0")],
        &state,
        &Running::new(BTreeSet::from(["kanata".to_string()]), Default::default()),
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: SCOOP.into(),
            name: Name::new("kanata"),
            reason: SkipReason::Running
        }],
        "a running package must never turn into a Prune"
    );
}

#[test]
fn a_running_package_is_not_pruned_when_only_its_manifest_names_the_process() {
    // The realistic kanata: the package is `kanata`, the live process is
    // kanata_windows_tty_winIOv2_arm64.exe.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("kanata"), Ownership::Installed);

    let mut inst = installed("kanata", "1.12.0");
    inst.bins = vec!["kanata_windows_tty_winiov2_arm64".to_string()];

    let p = plan(
        &config::parse("[scoop]\npackages = []\n").unwrap(),
        &lock::Lock::default(),
        &[inst],
        &state,
        &Running::new(
            BTreeSet::from(["kanata_windows_tty_winiov2_arm64".to_string()]),
            Default::default(),
        ),
    );
    assert!(
        matches!(p.actions.as_slice(), [Action::Skip { reason: SkipReason::Running, .. }]),
        "got {:?}",
        p.actions
    );
}

#[test]
fn an_idle_owned_undeclared_package_is_still_pruned() {
    // The guard must not turn the prune off altogether.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);

    let p = plan(
        &config::parse("[scoop]\npackages = []\n").unwrap(),
        &lock::Lock::default(),
        &[installed("aichat", "0.30.0")],
        &state,
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Prune {
            backend: SCOOP.into(),
            name: Name::new("aichat"),
            version: "0.30.0".into()
        }]
    );
}
```

Add `use std::collections::BTreeSet;` to that file's imports.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test planner a_running_package_is_never_pruned`
Expected: FAIL — `got [Prune { backend: "scoop", name: kanata, version: "1.12.0" }]`.

- [ ] **Step 3: Implement**

In `src/plan.rs`, in the second loop, between the helper check and the ownership check:

```rust
        if state.owns(SCOOP, &inst.name) {
            // Prune is the one action with no second chance: an interrupted
            // upgrade puts the app back, an uninstall does not. So the running
            // check that guards version changes guards this too.
            if running.covers(&inst.name, &inst.bins) {
                actions.push(Action::Skip {
                    backend: SCOOP.into(),
                    name: inst.name.clone(),
                    reason: SkipReason::Running,
                });
            } else {
                prunes.push(Action::Prune {
                    backend: SCOOP.into(),
                    name: inst.name.clone(),
                    version: inst.version.clone(),
                });
            }
        } else {
```

The `Skip` goes into `actions`, not `prunes`, so it prints with the other skips rather than in the prune block.

- [ ] **Step 4: Run the tests**

Run: `cargo test --all`
Expected: PASS.

- [ ] **Step 5: Negative control**

Delete the `if running.covers(&inst.name, &inst.bins)` branch, leaving the unconditional `prunes.push`.

Run: `cargo test --test planner a_running_package`
Expected: FAIL, both tests. Paste in. Restore, confirm green.

- [ ] **Step 6: Commit**

```bash
git add src/plan.rs tests/planner.rs
git commit -m "Never prune a package that is running

The prune loop did not consult the running set at all. A kanata removed
from pkg.toml while holding a keyboard hook planned a clean uninstall."
```

---

### Task 6: A closed architecture vocabulary, and drift reported

**Files:**
- Modify: `src/config.rs`, `src/plan.rs`, `src/render.rs`
- Modify: `tests/planner.rs`

**Interfaces:**
- Consumes: `Config.scoop.opts` from Task 2
- Produces:
  - `pub enum Arch { X64, X86, Arm64, Keep }` with `Arch::as_scoop(self) -> Option<&'static str>`
  - `PkgOpts { pub arch: Option<Arch> }`
  - `Action::ArchDrift { backend: String, name: Name, have: String, want: String }`
  - `Plan::drift_count(&self) -> usize`

- [ ] **Step 1: Write the failing tests**

Append to `tests/planner.rs`:

```rust
const ARM64_PYTHON: &str =
    "[scoop]\npackages = [\"python\"]\n\n[scoop.opts]\npython = { arch = \"arm64\" }\n";

fn installed_arch(name: &str, version: &str, arch: Option<&str>) -> Installed {
    let mut i = installed(name, version);
    i.arch = arch.map(|a| a.to_string());
    i
}

#[test]
fn a_package_installed_for_the_wrong_architecture_is_reported() {
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &State::default(),
        &Running::default(),
    );
    assert!(
        p.actions.contains(&Action::ArchDrift {
            backend: SCOOP.into(),
            name: Name::new("python"),
            have: "64bit".into(),
            want: "arm64".into(),
        }),
        "got {:?}",
        p.actions
    );
}

#[test]
fn drift_is_reported_even_without_a_lock_entry() {
    // Otherwise the report is invisible on any machine that has not run
    // `dotpkg update` -- which is every machine today, including the one this
    // gets dogfooded on.
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.drift_count(), 1, "got {:?}", p.actions);
}

#[test]
fn an_unknown_installed_architecture_is_not_drift() {
    // install.json only appeared in later scoop versions. Treating unknown as
    // wrong would make dotpkg want to reinstall such apps on every run.
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", None)],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.drift_count(), 0, "got {:?}", p.actions);
}

#[test]
fn keep_means_never_report_whatever_is_installed() {
    let p = plan(
        &config::parse(
            "[scoop]\npackages = [\"rustup\"]\n\n[scoop.opts]\nrustup = { arch = \"keep\" }\n",
        )
        .unwrap(),
        &lock::Lock::default(),
        &[installed_arch("rustup", "1.28.0", Some("64bit"))],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.drift_count(), 0, "got {:?}", p.actions);
}

#[test]
fn an_undeclared_architecture_is_no_opinion_and_no_report() {
    let p = plan(
        &config::parse("[scoop]\npackages = [\"python\"]\n").unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.drift_count(), 0, "got {:?}", p.actions);
}

#[test]
fn drift_is_a_report_not_a_change() {
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.change_count(), 0, "drift must not count as a change");
}

#[test]
fn a_package_can_be_both_an_upgrade_and_a_drift() {
    // Two true facts. Suppressing one would need a rule the reader has to
    // remember, and 2b may well fix the arch by way of the upgrade anyway.
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::parse(
            "[scoop.python]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"3.14.6\"\n",
        )
        .unwrap(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.change_count(), 1);
    assert_eq!(p.drift_count(), 1);
}
```

And in `src/config.rs`'s test module:

```rust
    #[test]
    fn a_misspelled_architecture_is_an_error_not_a_permanent_drift() {
        // `arch = "arm"` used to parse cleanly and mean "always wrong", which
        // in Phase 2b is "reinstall on every run".
        let err = parse("[scoop.opts]\npython = { arch = \"arm\" }\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("arm64"), "the error must list the real values: {msg}");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --all`
Expected: FAIL — `no variant named ArchDrift`, `no method named drift_count`, and the config test passes `arch = "arm"` without error.

- [ ] **Step 3: Add the `Arch` vocabulary in `src/config.rs`**

```rust
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
```

The existing test `parses_the_documented_example` asserts
`cfg.scoop.opts["python"].arch.as_deref() == Some("64bit")`. Change those two
assertions to `assert_eq!(cfg.scoop.opts[&Name::new("python")].arch, Some(Arch::X64));`
and `... [&Name::new("kanata")].arch, Some(Arch::Keep));`.

- [ ] **Step 4: Add the action in `src/plan.rs`**

New variant, after `Unmanaged`:

```rust
    /// Installed for an architecture other than the one declared. Reported in
    /// Phase 2a and not acted on: fixing it means a reinstall, and that
    /// decision waits for the measured picture from a real machine.
    ArchDrift {
        backend: String,
        name: Name,
        have: String,
        want: String,
    },
```

New count, beside the other two:

```rust
    pub fn drift_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, Action::ArchDrift { .. }))
            .count()
    }
```

`change_count` is an allowlist, so it already excludes the new variant. Leave it alone.

In the declared-packages loop, **before** the `let Some(pin) = lock.scoop.get(name) else { ... continue }` line, so that a machine with no lock still reports:

```rust
        // Emitted independently of the version verdict, and before the lock
        // check: architecture is a fact about the machine, true whether or not
        // dotpkg knows which version it wants. A package can be both an
        // Upgrade and an ArchDrift; those are two true facts.
        if let (Some(cur), Some(want)) = (
            current,
            declared
                .scoop
                .opts
                .get(name)
                .and_then(|o| o.arch)
                .and_then(|a| a.as_scoop()),
        ) {
            // A missing install.json means "unknown", not "wrong". Older scoop
            // versions did not write one, and reinstalling those on every run
            // would be a bug, not a fix.
            if let Some(have) = cur.arch.as_deref() {
                if have != want {
                    reports.push(Action::ArchDrift {
                        backend: SCOOP.into(),
                        name: name.clone(),
                        have: have.to_string(),
                        want: want.to_string(),
                    });
                }
            }
        }
```

- [ ] **Step 5: Render it in `src/render.rs`**

New arm in the `match a`:

```rust
            Action::ArchDrift {
                backend,
                name,
                have,
                want,
            } => {
                format!(
                    "  ~ {backend:<6} {name:<14} {:<24} (architecture drift -- reported, not fixed)",
                    format!("{have}, declared {want}")
                )
            }
```

And the summary, replacing the `else` branch's `push_str`:

```rust
        let mut summary = format!(
            "\n  {} change(s), {} skipped",
            plan.change_count(),
            plan.skip_count()
        );
        if plan.drift_count() > 0 {
            summary.push_str(&format!(", {} architecture drift", plan.drift_count()));
        }
        summary.push('\n');
        out.push_str(&summary);
```

Extend the render test `every_action_kind_gets_a_distinct_marker`: add

```rust
                Action::ArchDrift {
                    backend: SCOOP.into(),
                    name: "python".into(),
                    have: "64bit".into(),
                    want: "arm64".into(),
                },
```

to its `actions`, and add the assertions

```rust
        assert!(out.contains("~ scoop  python"));
        assert!(out.contains("64bit, declared arm64"));
        assert!(out.contains("4 change(s), 2 skipped, 1 architecture drift"));
```

replacing the existing `4 change(s), 2 skipped` assertion. This test is the one that historically went stale while claiming to cover every kind; `render`'s `match` is exhaustive, so the compiler forces the arm, but only this step forces the assertion.

- [ ] **Step 6: Run everything**

Run: `cargo test --all && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 7: Negative control — three breaks**

1. Make the comparison always equal: `if have != want` → `if false`.
   Run: `cargo test --test planner a_package_installed_for_the_wrong_architecture`
   Expected: FAIL.
2. Treat unknown as a mismatch: replace `if let Some(have) = cur.arch.as_deref()` with `let have = cur.arch.as_deref().unwrap_or("unknown");`.
   Run: `cargo test --test planner an_unknown_installed_architecture_is_not_drift`
   Expected: FAIL.
3. Move the drift block to *after* the lock check.
   Run: `cargo test --test planner drift_is_reported_even_without_a_lock_entry`
   Expected: FAIL — and note that this is the break that would have made Task 8's dogfood observe nothing.

Paste all three in, restore, confirm green.

- [ ] **Step 8: Commit**

```bash
git add src/config.rs src/plan.rs src/render.rs tests/planner.rs
git commit -m "Close the architecture vocabulary and report drift

arch was Option<String> and accepted anything, so a typo meant
permanently drifted. Drift is reported, not acted on: fixing it means a
reinstall, and that decision waits for a measurement."
```

---

### Task 7: `scan()` stops swallowing two more error classes

**Files:**
- Modify: `src/backend/scoop.rs`
- Modify: `tests/scoop_scan.rs`

**Interfaces:**
- Consumes: `Scan.warnings` from Phase 1
- Produces: no signature change

- [ ] **Step 1: Write the failing test**

Append to `tests/scoop_scan.rs`:

```rust
#[test]
fn a_manifest_that_is_not_a_file_warns_rather_than_vanishing() {
    // The READ branch, as distinct from the parse branch already covered.
    // Reverting that branch to swallow every error left the whole suite green,
    // which is what makes this test worth its lines.
    //
    // Making manifest.json a DIRECTORY is the portable trigger: it yields a
    // non-NotFound error on every platform, unlike a permission denial.
    let dir = tempfile::tempdir().unwrap();
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");
    fs::create_dir_all(
        dir.path()
            .join("apps")
            .join("unreadable")
            .join("current")
            .join("manifest.json"),
    )
    .unwrap();

    let scan = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    assert_eq!(scan.installed.len(), 1, "got {:?}", scan.installed);
    assert_eq!(scan.warnings.len(), 1, "got {:?}", scan.warnings);
    assert!(
        scan.warnings[0].contains("unreadable"),
        "the warning must name the app: {:?}",
        scan.warnings
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test scoop_scan a_manifest_that_is_not_a_file`
Expected: PASS already — the read branch was narrowed in Phase 1; what was missing is the test. **This is the point.** Confirm it passes, then go straight to the negative control in Step 4 to establish that it is testing anything at all.

- [ ] **Step 3: Fix the `read_dir` iteration branch**

In `scan()`, replace `for entry in entries.flatten() {` with:

```rust
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                // Same class as an unreadable manifest four lines down: a
                // directory we were told about and cannot look at is a fact
                // about this machine, not an absence.
                Err(e) => {
                    out.warnings
                        .push(format!("cannot read an entry of {}: {e}", apps.display()));
                    continue;
                }
            };
```

**This branch has no portable test.** Producing a failing `read_dir` iteration requires a directory entry that cannot be stat'd, which cannot be fabricated the same way on macOS, Linux and Windows. Do not invent one, and do not claim coverage: record in the commit message that it is verified by inspection only.

- [ ] **Step 4: Negative control**

Revert the manifest read branch to swallowing everything:

```rust
            let Ok(manifest_text) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
```

Run: `cargo test --test scoop_scan`
Expected: FAIL — `a_manifest_that_is_not_a_file_warns_rather_than_vanishing`, `left: 0, right: 1`. Paste it in.

This is the specific claim `docs/phase2-notes.md` makes — that the read branch could be reverted with the suite staying green. Confirm that it is now false.

Restore, re-run, confirm green.

- [ ] **Step 5: Commit**

```bash
git add src/backend/scoop.rs tests/scoop_scan.rs
git commit -m "Test the scan read branch and stop discarding read_dir errors

The narrowed read branch had no test: reverting it to swallow every I/O
error left the suite green. The read_dir iteration branch is fixed but
has no portable trigger, so it is verified by inspection, not by a test."
```

---

### Task 8: Dogfood on a14, read-only

**Files:**
- Create: `docs/dogfood-phase2a-2026-08-08.md`

**Interfaces:**
- Consumes: the `dotpkg` binary built from Task 7's tree
- Produces: a record of what `status` now says about a real machine, and the architecture measurement Phase 2b needs

`status` still performs no write, no subprocess and no network call, so this run is read-only by construction.

- [ ] **Step 1: Build natively on a14**

Cross-linking to MSVC from macOS needs the Microsoft linker, which the Mac does not have; Phase 1 established that building on a14 is the working path. Copy `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` — **not** `target/` or `.git/` — to `C:\Users\kln\dotpkg-build`, then `cargo build --release`.

- [ ] **Step 2: Write the dogfood `pkg.toml`**

The Phase 1 list, plus one deliberate addition so drift can be observed at all. `stylua` is installed as `64bit` on an ARM64 machine, so declaring `arm64` for it makes the new report fire:

```toml
[scoop]
buckets  = ["main", "extras", "xom11=https://github.com/xom11/scoop-bucket"]
packages = [
  "git", "nodejs", "gh", "bat", "ripgrep", "fzf", "fastfetch", "neovim",
  "tree-sitter", "lazygit", "lazydocker", "yazi", "zellij", "opencode",
  "shfmt", "yamlfmt", "stylua", "actionlint", "kanata", "beckon",
  "python", "go", "rustup", "uv", "age",
]

[scoop.opts]
python = { arch = "64bit" }
rustup = { arch = "keep" }
stylua = { arch = "arm64" }
```

- [ ] **Step 3: Run at medium integrity, not over plain ssh**

`ssh a14` runs at High Mandatory Level with no UAC, and that difference already produced one false finding on this machine. Use the scheduled-task technique from `docs/dogfood-2026-08-08.md`:

```powershell
$principal = New-ScheduledTaskPrincipal -UserId 'kln' -LogonType Interactive -RunLevel Limited
$trigger   = New-ScheduledTaskTrigger -Once -At (Get-Date).AddSeconds(120)
$action    = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument '/c "whoami /groups > out.txt 2>&1 & dotpkg.exe status --config C:\Users\kln\pkg.toml --lock C:\Users\kln\pkg.lock >> out.txt 2>&1"'
Register-ScheduledTask -TaskName DotpkgPhase2a -Action $action -Trigger $trigger -Principal $principal -Force
schtasks /run /tn DotpkgPhase2a
```

Confirm from the task's own `whoami /groups` that it ran at `Medium Mandatory Level`. Send every command as UTF-16LE base64 via `-EncodedCommand`; plain quoting through ssh mangles PowerShell arguments. Write output to a file and `scp` it back — returned output is CLIXML-wrapped and truncated.

- [ ] **Step 4: Answer five questions, each able to come back "no"**

Record the verbatim output, then answer:

1. **Does `! scoop kanata running` appear while kanata is running?** Start kanata if it is not running. If it cannot be started, write down that the check was **not exercised** — do not report a pass. Also record which process names are actually alive (`Kanata.exe` from the shim, the long `kanata_windows_tty_winIOv2_arm64.exe`, or both), since the design predicts the answer depends on how it was launched.
2. **Are `neovim`, `ripgrep` and `7zip` matched by their real executables?** With `nvim`, `rg` or `7z` running and a lock entry forcing a version change, each must print `!` rather than an upgrade or downgrade line.
3. **Are `nodejs` and `rustup` caught by path matching alone?** Run `node` and `cargo`; both name no executable in their manifests, so a `!` line here comes from the path signal and nothing else. This is the case that justified putting path matching in 2a.
4. **Do all thirty apps still scan, with no new warnings** compared with the Phase 1 run?
5. **Does `~ scoop stylua 64bit, declared arm64` appear**, and does the summary line carry `1 architecture drift`?

- [ ] **Step 5: Measure the machine-wide architecture picture**

Not a feature — an input to Phase 2b's decision on whether `apply` should act on drift. Read `architecture` from every `apps/*/current/install.json` and record the counts, with the emulated packages named. The measurement taken while writing the spec was 20 `arm64` and 10 `64bit`, of which `python` is deliberate and `dark`/`innounp` are helpers. Confirm or correct it.

- [ ] **Step 6: Clean up and verify**

Unregister `DotpkgPhase2a`, delete every file the run created, and verify with `Get-ScheduledTask` and `Test-Path` that each is gone. Record in the document which artifacts were authored by the investigator rather than by `dotpkg`, and confirm that `pkg.lock` and `%LOCALAPPDATA%\dotpkg\state.json` are absent before and after.

- [ ] **Step 7: Write the record**

Create `docs/dogfood-phase2a-2026-08-08.md` with the verbatim output, the five answers, the architecture measurement, and — separately — anything that came back different from what this plan predicted. A dogfood that confirms every expectation has usually not been read carefully enough; the Phase 1 run's most valuable output was the finding it went on to refute.

- [ ] **Step 8: Commit**

```bash
git add docs/dogfood-phase2a-2026-08-08.md
git commit -m "Record the Phase 2a dogfood run against a14"
```

---

## Before merging

Per-task review missed a cross-task bug in Phase 1: nine reviews passed while `plan()` silently dropped the `[winget]` section, and only a whole-branch review caught it. So after Task 8, review the branch as a whole against `docs/specs/2026-08-08-phase2a-design.md`, looking specifically for:

- A path through `plan()` that reaches a decision without consulting `Running` — there are now three (version change, prune, and any added later).
- A `Name` that got converted back to `String` anywhere, which would reintroduce case sensitivity at that point.
- Any test whose negative control was skipped or whose recorded failure output does not match what the test claims to check.

Then run the full gate on all three CI platforms.

---

## What Phase 2a deliberately leaves out

Carried into 2b, and listed so each absence is a decision:

- The executor: `install`, `uninstall`, and the `git show` restore path — which, per the spec's Corrections section, belongs to 2b rather than Phase 3.
- `SkipReason::NotLocked` becoming a hard failure.
- The `state.json` write path.
- The mass-prune guard. An empty `pkg.toml` still plans a prune of everything owned, which is the truth and is what `status` should say; the guard belongs at execution and must not be bypassed by `--yes`.
- Post-uninstall verification, per-package failure accumulation, the confirmation prompt, and cloning a missing bucket.
- Whether `apply` acts on architecture drift — decided from Task 8's measurement.
- Moving `SCOOP_HELPERS` from the planner to the backend, and splitting `Lock.scoop` / `Lock.winget` into distinct pin types. Both are Phase 3/4 concerns.

## Self-Review

**Spec coverage.** Each of the six changes in `docs/specs/2026-08-08-phase2a-design.md` has a task: case-insensitive names (Tasks 1–2), bins and shortcuts (Task 3), path matching (Task 4), prune consults running (Task 5), the `Arch` vocabulary and `ArchDrift` (Task 6), the two `scan()` error classes (Task 7), rendering (Task 6), dogfood (Task 8). The spec's testing section is satisfied by the fixtures in Task 3 and by a negative control in every task except Task 8, which is a measurement rather than a code change.

**Known gap, accepted.** The `read_dir` iteration branch fixed in Task 7 has no portable test. Task 7 says so in the step, and its commit message records it, rather than letting the fix look covered.

**The plan's own code was executed, not just written.** Phase 1 shipped a plan containing a task whose code its own test could not pass. To avoid repeating that, the three riskiest pieces of new Rust here were built and run in a throwaway crate before this document was committed: `Name` (19 assertions, including `[scoop.FZF]` as a TOML section key, `state.json`'s nested JSON map with display case preserved, and `{:<14}` padding through `f.pad`), `declared_executables` against all six real manifest shapes with the exact expected values asserted in Task 3, and `running_apps` against all seven path cases in Task 4. All green. What that does **not** establish is that they integrate correctly with the existing code — that is what Task 2's suite run is for.

**Type consistency.** `Name`, `Running`, `Installed.bins`, `Process` and `Arch` are each defined once and used with the same names throughout. `plan()`'s five-argument signature in Task 2 matches every call site in Tasks 5 and 6. `Running::covers(&Name, &[String])` is called with the installed record's own fields at both of its call sites, so it sees `bins` from Task 3 without a signature change — and it does not depend on `Installed`, which is what lets Task 1 compile before Task 2 changes that type.

**One plan bug found and fixed before dispatch.** Task 1 originally added `bins` to `Installed` and changed its `name` to a `Name`, which would have broken `src/backend/scoop.rs`, `src/plan.rs`, `src/render.rs` and both test files at that same commit — Task 1 could not have ended green. Those two field changes moved to Task 2, where the sweep fixes every call site in one commit, and `Running::covers` was narrowed to `(&Name, &[String])` so it no longer depends on a type that was about to change. `Scoop::running_apps` is declared in Task 4 and called only from `main.rs` in that same task.
