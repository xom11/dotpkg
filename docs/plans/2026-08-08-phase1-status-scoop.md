# dotpkg Phase 1 — `status` with the scoop backend

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `dotpkg status` — reads `pkg.toml`, `pkg.lock`, `state.json` and the installed scoop packages, then prints the plan it *would* execute. Changes nothing.

**Architecture:** A pure planner function takes five owned inputs `(declared, lock, installed, state, running)` and returns a `Plan`. Everything that touches the filesystem lives in `config`/`lock`/`state`/`backend::scoop`/`sys`; everything that decides lives in `plan`. Phase 2's `apply` reuses the same planner and only adds an executor.

**Tech Stack:** Rust 2021, `clap` (derive), `serde` + `toml` + `serde_json`, `anyhow`, `sysinfo`.

## Global Constraints

Copied from `docs/specs/2026-08-08-design.md`. Every task's requirements implicitly include these.

- **Never degrade silently.** A missing lock entry, a missing bucket, or a missing commit is a reported condition, never a fallback to "latest".
- **The planner is pure.** No I/O, no network, no subprocess in `src/plan.rs`. Enforced by a test in Task 6.
- **Subprocesses are for mutation only.** Phase 1 mutates nothing, so Phase 1 spawns no subprocess at all. Scoop state is read from disk.
- **Prune only ever touches a package named in `state.json`.** Phase 1 does not prune, but the planner must already classify by ownership.
- **Scoop helper packages are `dark`, `innounp`, `7zip`, `lessmsi`.** Never reported as strays.
- **Test layers 1 and 2 must run on Linux and macOS.** No test in Phase 1 may require Windows or a real scoop install.
- Rust edition 2021, minimum toolchain 1.85 — the highest `rust-version` in the
  resolved tree (hashbrown 1.85.0, with clap 4, indexmap and getrandom at 1.85).
  Anything lower is a claim the crate cannot honour, and `rust-version` is
  metadata a crates.io publish carries.

---

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` | Crate metadata, dependencies |
| `src/main.rs` | CLI parsing (clap), wiring inputs into the planner, exit codes |
| `src/model.rs` | Types shared across layers: `Installed`, `Backend` name constants |
| `src/config.rs` | Parse `pkg.toml` into `Config` |
| `src/lock.rs` | Parse `pkg.lock` into `Lock`, with the per-backend `Pin` enum |
| `src/state.rs` | Read/write `state.json` into `State` |
| `src/sys.rs` | Running-process names (the only OS query in Phase 1) |
| `src/backend/mod.rs` | The `Backend` trait |
| `src/backend/scoop.rs` | `scan()` — read scoop's on-disk state |
| `src/plan.rs` | The pure planner, `Action`, `SkipReason`, `Plan` |
| `src/render.rs` | `Plan` → terminal text |
| `tests/planner.rs` | Layer 1: table-driven planner tests |
| `tests/scoop_scan.rs` | Layer 2: `scan()` against a fabricated directory tree |
| `.github/workflows/ci.yml` | `cargo test` on ubuntu + macos + windows |

`plan.rs` and `backend/scoop.rs` are the two files that will grow most in later
phases; keeping the trait in `backend/mod.rs` from the start is what lets the
winget backend (Phase 4) and choco (v2) land without touching `plan.rs`.

---

### Task 1: Crate scaffold and CI

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `.github/workflows/ci.yml`
- Create: `.gitignore`

**Interfaces:**
- Consumes: nothing
- Produces: a binary named `dotpkg` that runs and exits 0

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "dotpkg"
version = "0.1.0"
edition = "2021"
rust-version = "1.85"
description = "Declarative package management for Windows: winget and scoop from one dotfile, with a real lock file and prune"
repository = "https://github.com/xom11/dotpkg"
license = "MIT"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sysinfo = "0.32"
toml = "0.8"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Create `.gitignore`**

```
/target
```

- [ ] **Step 3: Create a placeholder `src/main.rs`**

```rust
fn main() {
    println!("dotpkg");
}
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build`
Expected: `Finished dev [unoptimized + debuginfo] target(s)`

- [ ] **Step 5: Create `.github/workflows/ci.yml`**

Windows is in the matrix from day one even though no Phase 1 test needs it —
the point is to catch a Windows-only compile break the day it lands, not months
later.

```yaml
name: CI
on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test --all
```

- [ ] **Step 6: Verify formatting and lints pass locally**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: no output, exit 0

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore src/main.rs .github/workflows/ci.yml
git commit -m "Scaffold the crate and CI on three platforms"
```

---

### Task 2: Shared model types

**Files:**
- Create: `src/model.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct Installed { pub backend: String, pub name: String, pub version: String, pub arch: Option<String>, pub bucket: Option<String> }`
  - `pub const SCOOP: &str = "scoop";`
  - `pub const WINGET: &str = "winget";`

- [ ] **Step 1: Write the failing test**

Create `src/model.rs` with the test at the bottom:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub backend: String,
    pub name: String,
    pub version: String,
    /// Scoop records this in install.json; winget does not expose it.
    pub arch: Option<String>,
    /// Scoop only.
    pub bucket: Option<String>,
}

pub const SCOOP: &str = "scoop";
pub const WINGET: &str = "winget";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_is_comparable_by_value() {
        let a = Installed {
            backend: SCOOP.into(),
            name: "fzf".into(),
            version: "0.74.2".into(),
            arch: Some("arm64".into()),
            bucket: Some("main".into()),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 2: Wire the module in and run the test**

Replace `src/main.rs` with:

```rust
mod model;

fn main() {
    println!("dotpkg");
}
```

Run: `cargo test model`
Expected: PASS, 1 test

- [ ] **Step 3: Commit**

```bash
git add src/model.rs src/main.rs
git commit -m "Add the Installed model shared by every backend"
```

---

### Task 3: Parse `pkg.toml`

**Files:**
- Create: `src/config.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct Config { pub scoop: ScoopSection, pub winget: WingetSection }`
  - `pub struct ScoopSection { pub buckets: Vec<String>, pub packages: Vec<String>, pub opts: BTreeMap<String, PkgOpts> }`
  - `pub struct WingetSection { pub packages: Vec<String> }`
  - `pub struct PkgOpts { pub arch: Option<String> }`
  - `pub fn parse(text: &str) -> anyhow::Result<Config>`
  - `pub fn load(path: &Path) -> anyhow::Result<Config>`

- [ ] **Step 1: Write the failing test**

Create `src/config.rs`:

```rust
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
    pub packages: Vec<String>,
    #[serde(default)]
    pub opts: BTreeMap<String, PkgOpts>,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WingetSection {
    #[serde(default)]
    pub packages: Vec<String>,
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
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
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
        assert_eq!(cfg.scoop.opts["python"].arch.as_deref(), Some("64bit"));
        assert_eq!(cfg.scoop.opts["kanata"].arch.as_deref(), Some("keep"));
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
```

- [ ] **Step 2: Wire the module in**

`src/main.rs`:

```rust
mod config;
mod model;

fn main() {
    println!("dotpkg");
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test config`
Expected: PASS, 3 tests

- [ ] **Step 4: Commit**

```bash
git add src/config.rs src/main.rs
git commit -m "Parse pkg.toml, rejecting unknown keys rather than ignoring them"
```

---

### Task 4: Parse `pkg.lock`

**Files:**
- Create: `src/lock.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub enum Pin { ScoopCommit { bucket: String, commit: String, version: String }, WingetVersion { version: String } }`
  - `pub struct Lock { pub scoop: BTreeMap<String, Pin>, pub winget: BTreeMap<String, Pin> }`
  - `pub fn parse(text: &str) -> anyhow::Result<Lock>`
  - `pub fn load_or_empty(path: &Path) -> anyhow::Result<Lock>`
  - `impl Pin { pub fn version(&self) -> &str }`

- [ ] **Step 1: Write the failing test**

Create `src/lock.rs`:

```rust
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
        let err = parse("[scoop.fzf]\nversion = \"0.74.1\"\n").unwrap_err();
        assert!(format!("{err:#}").contains("commit"), "got: {err:#}");
    }

    #[test]
    fn an_unknown_winget_pin_kind_is_rejected() {
        let err = parse(
            "[winget.\"Git.Git\"]\nversion = \"2.55.0\"\npin = \"content-hash\"\n",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("version-only"), "got: {err:#}");
    }

    #[test]
    fn a_missing_lock_file_is_an_empty_lock_not_an_error() {
        let lock = load_or_empty(Path::new("/definitely/not/here/pkg.lock")).unwrap();
        assert_eq!(lock, Lock::default());
    }
}
```

- [ ] **Step 2: Wire the module in**

`src/main.rs`:

```rust
mod config;
mod lock;
mod model;

fn main() {
    println!("dotpkg");
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test lock`
Expected: PASS, 4 tests

- [ ] **Step 4: Commit**

```bash
git add src/lock.rs src/main.rs
git commit -m "Parse pkg.lock, keeping the scoop and winget pin shapes distinct"
```

---

### Task 5: Read and write `state.json`

**Files:**
- Create: `src/state.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `crate::model::{SCOOP, WINGET}`
- Produces:
  - `pub enum Ownership { Installed, Adopted }`
  - `pub struct State(BTreeMap<String, BTreeMap<String, Ownership>>)`
  - `impl State { pub fn owns(&self, backend: &str, name: &str) -> bool; pub fn set(&mut self, backend: &str, name: &str, o: Ownership); pub fn load_or_empty(path: &Path) -> Result<State>; pub fn save(&self, path: &Path) -> Result<()>; pub fn default_path() -> PathBuf }`

- [ ] **Step 1: Write the failing test**

Create `src/state.rs`:

```rust
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
```

- [ ] **Step 2: Wire the module in**

`src/main.rs`:

```rust
mod config;
mod lock;
mod model;
mod state;

fn main() {
    println!("dotpkg");
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test state`
Expected: PASS, 3 tests

- [ ] **Step 4: Commit**

```bash
git add src/state.rs src/main.rs
git commit -m "Add state.json, the fence that keeps prune off unowned packages"
```

---

### Task 6: The pure planner

This is the heart of the tool. It is also the only file where a bug removes
somebody's software, so it gets the most tests.

**Files:**
- Create: `src/plan.rs`
- Create: `tests/planner.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `crate::config::Config`, `crate::lock::{Lock, Pin}`, `crate::model::Installed`, `crate::state::State`
- Produces:
  - `pub enum SkipReason { Running, NotLocked }`
  - `pub enum Action { Install{..}, Upgrade{..}, Downgrade{..}, Prune{..}, Skip{..}, Unmanaged{..} }`
  - `pub struct Plan { pub actions: Vec<Action> }`
  - `impl Plan { pub fn change_count(&self) -> usize; pub fn skip_count(&self) -> usize }`
  - `pub fn plan(declared: &Config, lock: &Lock, installed: &[Installed], state: &State, running: &[String]) -> Plan`
  - `pub const SCOOP_HELPERS: &[&str]`

- [ ] **Step 1: Write the failing tests**

Create `tests/planner.rs`:

```rust
use dotpkg::config;
use dotpkg::lock;
use dotpkg::model::{Installed, SCOOP};
use dotpkg::plan::{plan, Action, SkipReason};
use dotpkg::state::{Ownership, State};

fn installed(name: &str, version: &str) -> Installed {
    Installed {
        backend: SCOOP.into(),
        name: name.into(),
        version: version.into(),
        arch: Some("arm64".into()),
        bucket: Some("main".into()),
    }
}

const DECLARED_FZF: &str = "[scoop]\npackages = [\"fzf\"]\n";
const LOCK_FZF_741: &str =
    "[scoop.fzf]\nbucket = \"main\"\ncommit = \"a28d0c56\"\nversion = \"0.74.1\"\n";

#[test]
fn a_declared_locked_package_that_is_absent_is_an_install() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Install {
            backend: SCOOP.into(),
            name: "fzf".into(),
            version: "0.74.1".into()
        }]
    );
}

#[test]
fn a_package_already_at_the_locked_version_produces_no_action() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1")],
        &State::default(),
        &[],
    );
    assert!(p.actions.is_empty(), "got {:?}", p.actions);
}

#[test]
fn a_newer_installed_version_is_a_downgrade_because_the_lock_is_authoritative() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.2")],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Downgrade {
            backend: SCOOP.into(),
            name: "fzf".into(),
            from: "0.74.2".into(),
            to: "0.74.1".into()
        }]
    );
}

#[test]
fn an_older_installed_version_is_an_upgrade() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.0")],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Upgrade {
            backend: SCOOP.into(),
            name: "fzf".into(),
            from: "0.74.0".into(),
            to: "0.74.1".into()
        }]
    );
}

#[test]
fn a_declared_package_with_no_lock_entry_is_reported_not_resolved() {
    // Spec: apply must fail here rather than resolve latest itself. Phase 1 is
    // read-only, so the planner surfaces it and Phase 2 turns it fatal.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::Lock::default(),
        &[],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: SCOOP.into(),
            name: "fzf".into(),
            reason: SkipReason::NotLocked
        }]
    );
}

#[test]
fn a_running_package_is_skipped_rather_than_changed() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.2")],
        &State::default(),
        &["fzf".into()],
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: SCOOP.into(),
            name: "fzf".into(),
            reason: SkipReason::Running
        }],
        "a running package must never turn into a Downgrade"
    );
}

#[test]
fn an_undeclared_owned_package_is_a_prune() {
    let mut state = State::default();
    state.set(SCOOP, "aichat", Ownership::Adopted);

    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1"), installed("aichat", "0.30.0")],
        &state,
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Prune {
            backend: SCOOP.into(),
            name: "aichat".into(),
            version: "0.30.0".into()
        }]
    );
}

#[test]
fn an_undeclared_unowned_package_is_reported_but_never_pruned() {
    // The whole reason dotpkg is safe to install on a populated machine.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1"), installed("antigravity", "2.0.6")],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Unmanaged {
            backend: SCOOP.into(),
            name: "antigravity".into(),
            version: "2.0.6".into()
        }]
    );
}

#[test]
fn scoop_helpers_are_never_reported_as_strays() {
    // Measured on a14: without this, 5 differences are reported and only 2 are
    // real — a 60% false-positive rate is how a feature gets switched off.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[
            installed("fzf", "0.74.1"),
            installed("dark", "3.14"),
            installed("innounp", "0.50"),
            installed("7zip", "26.01"),
            installed("lessmsi", "2.1"),
        ],
        &State::default(),
        &[],
    );
    assert!(p.actions.is_empty(), "got {:?}", p.actions);
}

#[test]
fn a_helper_that_the_user_declared_is_managed_normally() {
    let p = plan(
        &config::parse("[scoop]\npackages = [\"7zip\"]\n").unwrap(),
        &lock::parse(
            "[scoop.\"7zip\"]\nbucket = \"main\"\ncommit = \"abc\"\nversion = \"26.02\"\n",
        )
        .unwrap(),
        &[installed("7zip", "26.01")],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Upgrade {
            backend: SCOOP.into(),
            name: "7zip".into(),
            from: "26.01".into(),
            to: "26.02".into()
        }]
    );
}

#[test]
fn actions_are_ordered_installs_then_prunes_then_reports() {
    // Install before uninstall: if a run dies partway, an extra package is
    // easier to live with than a missing one.
    let mut state = State::default();
    state.set(SCOOP, "aichat", Ownership::Adopted);

    let p = plan(
        &config::parse("[scoop]\npackages = [\"fzf\", \"bat\"]\n").unwrap(),
        &lock::parse(
            "[scoop.fzf]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"0.74.1\"\n\
             [scoop.bat]\nbucket=\"main\"\ncommit=\"b\"\nversion=\"0.26.1\"\n",
        )
        .unwrap(),
        &[installed("aichat", "0.30.0"), installed("antigravity", "2.0.6")],
        &state,
        &[],
    );

    let kinds: Vec<&str> = p
        .actions
        .iter()
        .map(|a| match a {
            Action::Install { .. } => "install",
            Action::Upgrade { .. } => "upgrade",
            Action::Downgrade { .. } => "downgrade",
            Action::Prune { .. } => "prune",
            Action::Skip { .. } => "skip",
            Action::Unmanaged { .. } => "unmanaged",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["install", "install", "prune", "unmanaged"],
        "got {:?}",
        p.actions
    );
}

#[test]
fn counts_separate_changes_from_skips_and_reports() {
    let p = plan(
        &config::parse("[scoop]\npackages = [\"fzf\", \"bat\"]\n").unwrap(),
        &lock::parse("[scoop.fzf]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"0.74.1\"\n")
            .unwrap(),
        &[installed("antigravity", "2.0.6")],
        &State::default(),
        &[],
    );
    // fzf install = 1 change; bat unlocked = 1 skip; antigravity = report only.
    assert_eq!(p.change_count(), 1);
    assert_eq!(p.skip_count(), 1);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test planner`
Expected: FAIL — `use of undeclared crate or module dotpkg` / `unresolved import`

- [ ] **Step 3: Add a library target so integration tests can import the crate**

Add to `Cargo.toml`, after the `[package]` block:

```toml
[lib]
name = "dotpkg"
path = "src/lib.rs"

[[bin]]
name = "dotpkg"
path = "src/main.rs"
```

Create `src/lib.rs`:

```rust
pub mod config;
pub mod lock;
pub mod model;
pub mod plan;
pub mod state;
```

Replace `src/main.rs` with:

```rust
fn main() {
    println!("dotpkg");
}
```

- [ ] **Step 4: Write the planner**

Create `src/plan.rs`:

```rust
use crate::config::Config;
use crate::lock::Lock;
use crate::model::{Installed, SCOOP};
use crate::state::State;
use std::collections::BTreeSet;

/// Scoop installs these itself to unpack other packages and does NOT record
/// that it did: install.json for `dark` is shape-identical to a user-requested
/// package's. No installed manifest declares `depends` either, so there is
/// nothing to infer from and this list has to be explicit.
///
/// Update it if scoop gains a new extraction helper.
pub const SCOOP_HELPERS: &[&str] = &["dark", "innounp", "7zip", "lessmsi"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The package's process is alive. Changing it now risks the running app.
    Running,
    /// Declared in pkg.toml with no pkg.lock entry. `apply` must refuse rather
    /// than resolve a version itself.
    NotLocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Install {
        backend: String,
        name: String,
        version: String,
    },
    Upgrade {
        backend: String,
        name: String,
        from: String,
        to: String,
    },
    Downgrade {
        backend: String,
        name: String,
        from: String,
        to: String,
    },
    Prune {
        backend: String,
        name: String,
        version: String,
    },
    Skip {
        backend: String,
        name: String,
        reason: SkipReason,
    },
    /// Installed, undeclared, and not owned by dotpkg. Reported, never touched.
    Unmanaged {
        backend: String,
        name: String,
        version: String,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<Action>,
}

impl Plan {
    pub fn change_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    Action::Install { .. }
                        | Action::Upgrade { .. }
                        | Action::Downgrade { .. }
                        | Action::Prune { .. }
                )
            })
            .count()
    }

    pub fn skip_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, Action::Skip { .. }))
            .count()
    }
}

/// Pure. No I/O, no network, no subprocess — every input is passed in, which is
/// what lets the whole decision layer be tested on any OS.
pub fn plan(
    declared: &Config,
    lock: &Lock,
    installed: &[Installed],
    state: &State,
    running: &[String],
) -> Plan {
    let mut actions = Vec::new();
    let mut prunes = Vec::new();
    let mut reports = Vec::new();

    let declared_scoop: BTreeSet<&str> =
        declared.scoop.packages.iter().map(String::as_str).collect();
    let running: BTreeSet<&str> = running.iter().map(String::as_str).collect();

    // Declared packages: install / upgrade / downgrade / skip.
    for name in &declared.scoop.packages {
        let current = installed
            .iter()
            .find(|i| i.backend == SCOOP && &i.name == name);

        let Some(pin) = lock.scoop.get(name) else {
            actions.push(Action::Skip {
                backend: SCOOP.into(),
                name: name.clone(),
                reason: SkipReason::NotLocked,
            });
            continue;
        };
        let want = pin.version();

        match current {
            None => actions.push(Action::Install {
                backend: SCOOP.into(),
                name: name.clone(),
                version: want.to_string(),
            }),
            Some(cur) if cur.version == want => {}
            Some(cur) => {
                // Checked only once a change is actually called for, so a
                // healthy running package produces no line at all.
                if running.contains(name.as_str()) {
                    actions.push(Action::Skip {
                        backend: SCOOP.into(),
                        name: name.clone(),
                        reason: SkipReason::Running,
                    });
                } else if is_older(&cur.version, want) {
                    actions.push(Action::Upgrade {
                        backend: SCOOP.into(),
                        name: name.clone(),
                        from: cur.version.clone(),
                        to: want.to_string(),
                    });
                } else {
                    actions.push(Action::Downgrade {
                        backend: SCOOP.into(),
                        name: name.clone(),
                        from: cur.version.clone(),
                        to: want.to_string(),
                    });
                }
            }
        }
    }

    // Installed but undeclared: prune if owned, report if not, ignore helpers.
    for inst in installed.iter().filter(|i| i.backend == SCOOP) {
        if declared_scoop.contains(inst.name.as_str()) {
            continue;
        }
        if SCOOP_HELPERS.contains(&inst.name.as_str()) {
            continue;
        }
        if state.owns(SCOOP, &inst.name) {
            prunes.push(Action::Prune {
                backend: SCOOP.into(),
                name: inst.name.clone(),
                version: inst.version.clone(),
            });
        } else {
            reports.push(Action::Unmanaged {
                backend: SCOOP.into(),
                name: inst.name.clone(),
                version: inst.version.clone(),
            });
        }
    }

    // Install before uninstall: a run that dies partway should leave an extra
    // package rather than a missing one.
    actions.extend(prunes);
    actions.extend(reports);
    Plan { actions }
}

/// Dotted numeric comparison, falling back to string order for anything that
/// is not purely numeric. Deliberately not semver: scoop versions include
/// shapes like `26.01` and `2026.07.15.08.55` that semver rejects, and getting
/// the direction wrong only changes the arrow shown in the plan, never whether
/// a change happens.
fn is_older(a: &str, b: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    if pa.is_empty() || pb.is_empty() {
        return a < b;
    }
    pa < pb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering_handles_the_shapes_scoop_actually_uses() {
        assert!(is_older("0.74.1", "0.74.2"));
        assert!(!is_older("0.74.2", "0.74.1"));
        // Numeric, not lexical: "0.74.10" is newer than "0.74.9".
        assert!(is_older("0.74.9", "0.74.10"));
        assert!(is_older("26.01", "26.02"));
        assert!(is_older("2026.07.15", "2026.07.29"));
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test`
Expected: PASS — 12 tests in `tests/planner.rs`, 1 in `plan::tests`, plus the earlier module tests

- [ ] **Step 6: Add the purity guard**

Append to `tests/planner.rs`:

```rust
#[test]
fn the_planner_source_performs_no_io() {
    // The planner being pure is what lets layer-1 tests run on any OS. A stray
    // subprocess or file read here would quietly make the suite Windows-only.
    let src = include_str!("../src/plan.rs");
    for forbidden in [
        "std::process",
        "Command::",
        "std::fs",
        "File::",
        "reqwest",
        "std::net",
    ] {
        assert!(
            !src.contains(forbidden),
            "src/plan.rs must stay pure but mentions {forbidden}"
        );
    }
}
```

- [ ] **Step 7: Run the purity guard and confirm it can fail**

Run: `cargo test --test planner the_planner_source_performs_no_io`
Expected: PASS

Now the negative control. Temporarily add `// std::fs` as a comment on the
first line of `src/plan.rs`, re-run the same command, and confirm it FAILS with
`must stay pure but mentions std::fs`. Remove the comment and confirm it passes
again. An assertion that has never failed has not been shown to test anything.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml src/lib.rs src/main.rs src/plan.rs tests/planner.rs
git commit -m "Add the pure planner, with a guard that keeps it free of I/O"
```

---

### Task 7: Scan scoop's on-disk state

**Files:**
- Create: `src/backend/mod.rs`
- Create: `src/backend/scoop.rs`
- Create: `tests/scoop_scan.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `crate::model::{Installed, SCOOP}`
- Produces:
  - `pub trait Backend { fn name(&self) -> &str; fn scan(&self) -> anyhow::Result<Vec<Installed>>; }`
  - `pub struct Scoop { root: PathBuf }`
  - `impl Scoop { pub fn new(root: PathBuf) -> Scoop; pub fn discover() -> Scoop }`

- [ ] **Step 1: Write the failing test**

Create `tests/scoop_scan.rs`:

```rust
use dotpkg::backend::scoop::Scoop;
use dotpkg::backend::Backend;
use std::fs;
use std::path::Path;

/// Build the parts of a scoop install that `scan` reads. Mirrors the real
/// layout: apps/<name>/current/{manifest,install}.json
fn app(root: &Path, name: &str, version: &str, arch: &str, bucket: &str) {
    let dir = root.join("apps").join(name).join("current");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("manifest.json"),
        format!(r#"{{"version":"{version}","description":"x"}}"#),
    )
    .unwrap();
    fs::write(
        dir.join("install.json"),
        format!(r#"{{"bucket":"{bucket}","architecture":"{arch}"}}"#),
    )
    .unwrap();
}

#[test]
fn reads_name_version_arch_and_bucket_for_each_app() {
    let dir = tempfile::tempdir().unwrap();
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");
    app(dir.path(), "bat", "0.26.1", "64bit", "main");

    let mut got = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    got.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(got.len(), 2);
    assert_eq!(got[0].name, "bat");
    assert_eq!(got[0].version, "0.26.1");
    assert_eq!(got[0].arch.as_deref(), Some("64bit"));
    assert_eq!(got[1].name, "fzf");
    assert_eq!(got[1].bucket.as_deref(), Some("main"));
    assert!(got.iter().all(|i| i.backend == "scoop"));
}

#[test]
fn skips_the_scoop_directory_itself() {
    // ~/scoop/apps/scoop is scoop managing itself, not a package.
    let dir = tempfile::tempdir().unwrap();
    app(dir.path(), "scoop", "0.5.3", "64bit", "main");
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");

    let got = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "fzf");
}

#[test]
fn an_app_installed_by_an_older_scoop_has_no_install_json_and_still_scans() {
    // install.json only appeared in later scoop versions. Treating "unknown
    // architecture" as "wrong architecture" would make dotpkg want to reinstall
    // it on every run.
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("apps").join("old").join("current");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("manifest.json"), r#"{"version":"1.0"}"#).unwrap();

    let got = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "old");
    assert_eq!(got[0].arch, None);
    assert_eq!(got[0].bucket, None);
}

#[test]
fn a_directory_with_no_manifest_is_ignored_rather_than_failing_the_scan() {
    // A half-finished install must not take the whole run down.
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("apps").join("broken").join("current")).unwrap();
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");

    let got = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "fzf");
}

#[test]
fn a_missing_scoop_root_scans_to_nothing() {
    let got = Scoop::new("/definitely/not/here".into()).scan().unwrap();
    assert!(got.is_empty());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test scoop_scan`
Expected: FAIL — `unresolved import dotpkg::backend`

- [ ] **Step 3: Write the trait**

Create `src/backend/mod.rs`:

```rust
pub mod scoop;

use crate::model::Installed;
use anyhow::Result;

/// One package manager. `scan` reads state that is already on disk or already
/// known; nothing here reaches the network. Mutating methods arrive in Phase 2.
pub trait Backend {
    fn name(&self) -> &str;
    fn scan(&self) -> Result<Vec<Installed>>;
}
```

- [ ] **Step 4: Write the scoop backend**

Create `src/backend/scoop.rs`:

```rust
use super::Backend;
use crate::model::{Installed, SCOOP};
use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
}

#[derive(Debug, Default, Deserialize)]
struct Install {
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    architecture: Option<String>,
}

pub struct Scoop {
    root: PathBuf,
}

impl Scoop {
    pub fn new(root: PathBuf) -> Scoop {
        Scoop { root }
    }

    /// `$SCOOP` if set, else `%USERPROFILE%\scoop`, matching scoop's own rule.
    pub fn discover() -> Scoop {
        let root = std::env::var_os("SCOOP")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("scoop")))
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join("scoop")))
            .unwrap_or_else(|| PathBuf::from("scoop"));
        Scoop { root }
    }
}

impl Backend for Scoop {
    fn name(&self) -> &str {
        SCOOP
    }

    fn scan(&self) -> Result<Vec<Installed>> {
        let apps = self.root.join("apps");
        let entries = match std::fs::read_dir(&apps) {
            Ok(e) => e,
            // No scoop on this machine is a valid state, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut out = Vec::new();
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // apps/scoop is scoop managing itself.
            if name == SCOOP {
                continue;
            }

            let current = entry.path().join("current");
            let Ok(manifest_text) = std::fs::read_to_string(current.join("manifest.json")) else {
                // A half-finished or broken install must not fail the whole scan.
                continue;
            };
            let Ok(manifest) = serde_json::from_str::<Manifest>(&manifest_text) else {
                continue;
            };

            // install.json is absent on apps installed by older scoop versions.
            let install: Install = std::fs::read_to_string(current.join("install.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();

            out.push(Installed {
                backend: SCOOP.to_string(),
                name,
                version: manifest.version,
                arch: install.architecture,
                bucket: install.bucket,
            });
        }
        Ok(out)
    }
}
```

- [ ] **Step 5: Export the module**

`src/lib.rs`:

```rust
pub mod backend;
pub mod config;
pub mod lock;
pub mod model;
pub mod plan;
pub mod state;
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --test scoop_scan`
Expected: PASS, 5 tests

- [ ] **Step 7: Commit**

```bash
git add src/backend src/lib.rs tests/scoop_scan.rs
git commit -m "Read scoop state from disk instead of shelling out to scoop"
```

---

### Task 8: Render the plan and wire up `dotpkg status`

**Files:**
- Create: `src/render.rs`
- Create: `src/sys.rs`
- Modify: `src/lib.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `crate::plan::{Plan, Action, SkipReason}`, `crate::backend::scoop::Scoop`
- Produces:
  - `pub fn render(plan: &Plan) -> String`
  - `pub fn running_process_names() -> Vec<String>`
  - binary: `dotpkg status [--config <path>] [--lock <path>]`

- [ ] **Step 1: Write the failing test**

Create `src/render.rs`:

```rust
use crate::plan::{Action, Plan, SkipReason};

/// The plan is the product here: `status` is this and nothing else, and in
/// Phase 2 `apply` prints exactly this before asking for confirmation.
pub fn render(plan: &Plan) -> String {
    let mut out = String::new();
    for a in &plan.actions {
        let line = match a {
            Action::Install { backend, name, version } => {
                format!("  + {backend:<6} {name:<14} {version:<24} (install)")
            }
            Action::Upgrade { backend, name, from, to } => {
                format!("  ^ {backend:<6} {name:<14} {:<24} (upgrade)", format!("{from} -> {to}"))
            }
            Action::Downgrade { backend, name, from, to } => {
                format!("  v {backend:<6} {name:<14} {:<24} (downgrade, from lock)", format!("{from} -> {to}"))
            }
            Action::Prune { backend, name, version } => {
                format!("  - {backend:<6} {name:<14} {version:<24} (prune, owned)")
            }
            Action::Skip { backend, name, reason } => {
                let why = match reason {
                    SkipReason::Running => "running -- stop it first",
                    SkipReason::NotLocked => "no lock entry -- run `dotpkg update`",
                };
                format!("  ! {backend:<6} {name:<14} {why}")
            }
            Action::Unmanaged { backend, name, version } => {
                format!("  ? {backend:<6} {name:<14} {version:<24} (unmanaged -- no action)")
            }
        };
        out.push_str(&line);
        out.push('\n');
    }

    if plan.actions.is_empty() {
        out.push_str("  nothing to do\n");
    } else {
        out.push_str(&format!(
            "\n  {} change(s), {} skipped\n",
            plan.change_count(),
            plan.skip_count()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SCOOP;

    #[test]
    fn an_empty_plan_says_so_rather_than_printing_nothing() {
        assert!(render(&Plan::default()).contains("nothing to do"));
    }

    #[test]
    fn every_action_kind_gets_a_distinct_marker() {
        let plan = Plan {
            actions: vec![
                Action::Install { backend: SCOOP.into(), name: "ripgrep".into(), version: "14.1.0".into() },
                Action::Downgrade { backend: SCOOP.into(), name: "fzf".into(), from: "0.74.2".into(), to: "0.74.1".into() },
                Action::Prune { backend: SCOOP.into(), name: "aichat".into(), version: "0.30.0".into() },
                Action::Skip { backend: SCOOP.into(), name: "kanata".into(), reason: SkipReason::Running },
                Action::Unmanaged { backend: SCOOP.into(), name: "antigravity".into(), version: "2.0.6".into() },
            ],
        };
        let out = render(&plan);
        assert!(out.contains("+ scoop  ripgrep"));
        assert!(out.contains("v scoop  fzf"));
        assert!(out.contains("- scoop  aichat"));
        assert!(out.contains("! scoop  kanata"));
        assert!(out.contains("? scoop  antigravity"));
        assert!(out.contains("3 change(s), 1 skipped"));
    }

    #[test]
    fn a_skip_says_what_to_do_about_it() {
        let plan = Plan {
            actions: vec![Action::Skip {
                backend: SCOOP.into(),
                name: "bat".into(),
                reason: SkipReason::NotLocked,
            }],
        };
        assert!(render(&plan).contains("dotpkg update"));
    }
}
```

- [ ] **Step 2: Write the process lookup**

Create `src/sys.rs`:

```rust
use sysinfo::System;

/// Lowercased process base names, without extension: "kanata.exe" -> "kanata".
///
/// This is an input to the planner rather than something the planner discovers,
/// which is what lets `dotpkg status` say "skipped, running" before anything is
/// attempted.
pub fn running_process_names() -> Vec<String> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut names: Vec<String> = sys
        .processes()
        .values()
        .map(|p| {
            let n = p.name().to_string_lossy().to_lowercase();
            n.strip_suffix(".exe").unwrap_or(&n).to_string()
        })
        .collect();
    names.sort();
    names.dedup();
    names
}
```

- [ ] **Step 3: Wire up the CLI**

`src/lib.rs`:

```rust
pub mod backend;
pub mod config;
pub mod lock;
pub mod model;
pub mod plan;
pub mod render;
pub mod state;
pub mod sys;
```

`src/main.rs`:

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};
use dotpkg::backend::{scoop::Scoop, Backend};
use dotpkg::state::State;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "dotpkg", version, about = "Declarative package management for Windows")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print what `apply` would do. Changes nothing.
    Status {
        #[arg(long, default_value = "pkg.toml")]
        config: PathBuf,
        #[arg(long, default_value = "pkg.lock")]
        lock: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status { config, lock } => {
            let declared = dotpkg::config::load(&config)?;
            let locked = dotpkg::lock::load_or_empty(&lock)?;
            let state = State::load_or_empty(&State::default_path())?;
            let installed = Scoop::discover().scan()?;
            let running = dotpkg::sys::running_process_names();

            let plan = dotpkg::plan::plan(&declared, &locked, &installed, &state, &running);
            print!("{}", dotpkg::render::render(&plan));
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run the whole suite**

Run: `cargo test --all`
Expected: PASS, all tests

- [ ] **Step 5: Run it against a fabricated tree locally**

```bash
mkdir -p /tmp/dpk/apps/fzf/current
echo '{"version":"0.74.2"}' > /tmp/dpk/apps/fzf/current/manifest.json
echo '{"bucket":"main","architecture":"arm64"}' > /tmp/dpk/apps/fzf/current/install.json
printf '[scoop]\npackages = ["fzf"]\n' > /tmp/dpk/pkg.toml
printf '[scoop.fzf]\nbucket = "main"\ncommit = "a28d0c56"\nversion = "0.74.1"\n' > /tmp/dpk/pkg.lock

SCOOP=/tmp/dpk cargo run -- status --config /tmp/dpk/pkg.toml --lock /tmp/dpk/pkg.lock
```

Expected output:

```
  v scoop  fzf            0.74.2 -> 0.74.1       (downgrade, from lock)

  1 change(s), 0 skipped
```

- [ ] **Step 6: Commit**

```bash
git add src/render.rs src/sys.rs src/lib.rs src/main.rs
git commit -m "Add dotpkg status: render the plan and wire the CLI"
```

---

### Task 9: Dogfood on a14

The first contact with a real machine. Read-only, so nothing is at risk.

**Files:**
- Create: `docs/dogfood-2026-08-08.md`

**Interfaces:**
- Consumes: the `dotpkg` binary from Task 8
- Produces: a `pkg.toml` reflecting the real machine, and a record of what
  `status` got wrong

- [ ] **Step 1: Build for Windows ARM64**

From the mac, cross-compiling avoids installing a Rust toolchain on a14:

```bash
rustup target add aarch64-pc-windows-msvc
cargo build --release --target aarch64-pc-windows-msvc
```

If cross-linking fails (it needs the MSVC linker), build on a14 instead:
`rustup` is already installed there via scoop.

- [ ] **Step 2: Copy the binary to a14**

```bash
scp target/aarch64-pc-windows-msvc/release/dotpkg.exe a14:C:/Users/kln/dotpkg.exe
```

- [ ] **Step 3: Write a `pkg.toml` from the repo's existing scoop list**

The 25 names in `~/.nix/windows/modules/packages/scoop/module.ps1` become the
first config. Write it to `C:\Users\kln\pkg.toml` on a14:

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
```

- [ ] **Step 4: Run `status` with no lock file**

Over SSH, using `-EncodedCommand` — plain quoting through ssh mangles PowerShell
arguments:

```bash
printf '%s' 'C:\Users\kln\dotpkg.exe status --config C:\Users\kln\pkg.toml --lock C:\Users\kln\pkg.lock' \
  | python3 -c "import sys,base64;print(base64.b64encode(sys.stdin.read().encode('utf-16-le')).decode())" \
  | xargs -I{} ssh a14 "powershell -NoProfile -EncodedCommand {}"
```

Expected: every declared package reported `! ... no lock entry`, plus `?` lines
for `aichat` and `antigravity`, and **no** lines for `dark`, `innounp`, `7zip`,
`lessmsi`.

- [ ] **Step 5: Record what it got wrong**

Create `docs/dogfood-2026-08-08.md` with the actual output and a list of every
discrepancy between it and the truth. Specifically check:

- Is `beckon` found? It comes from the `xom11` bucket, not `main`.
- Are exactly `aichat` and `antigravity` reported as unmanaged?
- Are all four helpers silent?
- Does any package appear that is not in `~/scoop/apps`?

- [ ] **Step 6: Commit**

```bash
git add docs/dogfood-2026-08-08.md
git commit -m "Record the first dogfood run of status against a14"
```

---

## What Phase 1 deliberately leaves out

Each gets its own plan, in this order:

- **Phase 2 — `apply`:** the executor, the `state.json` write path, turning
  `SkipReason::NotLocked` into a hard error, post-uninstall verification.
- **Phase 3 — `update` and `adopt`:** bucket commit resolution and the
  `git show <commit>:bucket/<app>.json` restore path. The reproducibility claim
  only becomes real here.
- **Phase 4 — winget backend:** `scan` via a once-per-run `winget list`.
- **Phase 5 — `add`, docs, release** through the existing scoop bucket.

---

## Self-Review

**Spec coverage.** Every Phase 1 item in the spec's phasing — config, lock and
state types, scoop `scan()`, the pure planner, plan rendering — has a task.
Spec rules enforced by a test in this plan: the helper list (Task 6), the
`state.json` prune fence (Task 6), planner purity (Task 6, with a negative
control), "declared but unlocked is reported, never resolved" (Task 6), and the
asymmetric lock shape (Task 4). Spec items intentionally deferred are listed
above under "What Phase 1 deliberately leaves out".

**Known gap, accepted for Phase 1.** `PkgOpts.arch` is parsed (Task 3) but the
planner ignores it — architecture drift produces no action yet. This is correct
for a read-only phase, since acting on drift means a reinstall. Phase 2 must add
it, and the Phase 2 plan opens with that task.

**Type consistency.** `Installed`, `Pin`, `State`, `Action`, `SkipReason` and
`Plan` are defined once and used with the same field names throughout;
`plan()`'s five-argument signature in Task 6 matches its call site in Task 8;
`Backend::scan` is declared in Task 7 and called in Task 8.
