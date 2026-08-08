# dotpkg Phase 2b-1 — `apply --prepare` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `dotpkg apply --prepare` — close the three holes that make an executing plan unsafe, recover each locked manifest from its bucket commit, stage it where scoop can install from it, fetch it with hash verification, and stop before changing anything.

**Architecture:** `plan.rs` decides and stays pure; a new `src/apply.rs` reads a `Plan` and acts. Staging runs real `git` and is therefore fully testable on macOS against a real fixture repository; the one thing that needs scoop is isolated behind a pure argv function.

**Tech Stack:** Rust 2021, `serde`, `serde_json`, `toml`, `anyhow`, `clap`, `sysinfo`. No new dependencies. `git` must be on `PATH` to run the test suite.

**Design:** `docs/specs/2026-08-08-phase2b1-prepare-design.md`
**Carried findings:** `docs/phase2b-notes.md`

## Global Constraints

- **Never degrade silently.** A missing lock entry, a missing bucket, or a missing commit is a reported condition, never a fallback to "latest".
- **Never pass `--skip-hash-check`.** Hash verification is scoop's and dotpkg does not opt out of it.
- **The planner stays pure.** `src/plan.rs` is not touched by this plan. `tests/planner.rs::the_planner_source_performs_no_io` is an allowlist over `use` lines (`crate::`, `std::collections`, `super::`) plus a ban on any `std::` path other than `std::collections` anywhere in that file including comments.
- **Prepare changes no installed software.** It writes only to dotpkg's staging directory and scoop's download cache.
- Rust edition 2021, `rust-version = "1.85"`.
- The gate is `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all`.
- **Test layers 1 and 2 run on Linux and macOS.** No test added here may require Windows or a real scoop install. Tests may require `git`, which CI already has.
- **Every new test needs a negative control with recorded evidence.** Break the code the way the task names, paste the red output into the task, restore, confirm green.

## Baseline

`main` at `131414f`, branch `phase2b1-prepare` at `1a6a233`. **84 tests** green: 28 unit + 30 `tests/planner.rs` + 26 `tests/scoop_scan.rs`.

## Measured facts this plan is built on

Do not re-derive these; they cost a machine to establish and are recorded in `docs/phase2b-notes.md`.

| Fact | Consequence for this plan |
|---|---|
| `scoop install` has no force flag; installing over an app exits 0 and does nothing | every version change is uninstall+install, so prepare exists |
| `scoop download <path>/app.json` verifies hashes | the prefetch step is real, not a rehearsal |
| `scoop install <path>` takes the app name from the **filename** | the staged file must be named for the app, in its own directory |
| `install.json` records the staging **path**, not a bucket | staging is permanent, never `%TEMP%` |
| `scoop.cmd` in `<root>/shims` runs non-interactively | that is the entry point, not `scoop.ps1`, not `PATH` |
| buckets are git repos with manifests under `bucket/<app>.json` | `git -C <bucket> show <commit>:bucket/<app>.json` |

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/config.rs` | parse `pkg.toml`; reject duplicate declared names | Modify (Task 1) |
| `src/backend/scoop.rs` | canonical root; `stage`; `download_argv`; `download` | Modify (Tasks 2, 4, 5) |
| `src/apply.rs` | the apply driver: guard, prepare loop, outcomes | Create (Tasks 3, 6) |
| `src/render.rs` | render the preparation report | Modify (Task 6) |
| `src/main.rs`, `src/lib.rs` | wiring, the `apply --prepare` subcommand | Modify (Tasks 3, 6) |
| `tests/prepare.rs` | staging against a real git fixture repository | Create (Task 4) |
| `docs/dogfood-phase2b1-2026-08-08.md` | dogfood record | Create (Task 7) |

---

### Task 1: Reject two declared names that differ only in case

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Consumes: `crate::model::Name`
- Produces: `Config`, `ScoopSection`, `WingetSection` unchanged in shape; `parse` now errors on a folded-name collision

- [ ] **Step 1: Write the failing tests**

Append to `src/config.rs`'s test module:

```rust
    #[test]
    fn two_declared_names_differing_only_in_case_are_rejected() {
        // Name folds case, so these are one package -- but `packages` is a Vec
        // and the declared loop iterates it twice, producing two Install
        // actions for one app and a change_count of 2. Verified against the
        // merged planner.
        let err = parse("[scoop]\npackages = [\"fzf\", \"FZF\"]\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("fzf") && msg.contains("FZF"), "name both spellings: {msg}");
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
        assert!(msg.contains("python") && msg.contains("Python"), "got: {msg}");
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
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib config`
Expected: FAIL — the three rejection tests pass parsing instead of erroring.

- [ ] **Step 3: Add the raw layer**

`Config` can no longer derive `Deserialize` directly, because the collision is only visible before the fold. Mirror the shape `src/lock.rs` already uses. Replace the four struct definitions and `parse` with:

```rust
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
```

`PkgOpts` and `Arch` are unchanged.

- [ ] **Step 4: Run the whole suite**

Run: `cargo test --all && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: PASS. The four pre-existing config tests must still pass unchanged — in particular `a_misspelled_key_is_an_error_not_a_silent_ignore`, which depends on `deny_unknown_fields` having moved to the raw types, and `parses_the_documented_example`, whose display-fidelity assertions must still hold.

- [ ] **Step 5: Negative control**

In `fold_names`, replace the `bail!` with nothing (drop the collision check).

Run: `cargo test --lib config`
Expected: FAIL — `two_declared_names_differing_only_in_case_are_rejected` and `a_duplicate_winget_name_is_rejected_too`. Paste the output in. Then do the same for `fold_opts` and confirm `a_duplicate_scoop_opts_key_is_rejected_rather_than_silently_clobbered` goes red. Restore both, confirm green.

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "Reject two declared names that differ only in case

Name folds case but a Vec keeps both entries and a map keeps the first
key with the last value. Neither is visible in the user's own file."
```

---

### Task 2: Canonicalise the scoop root

**Files:**
- Modify: `src/backend/scoop.rs`
- Modify: `tests/scoop_scan.rs`

**Interfaces:**
- Produces: `Scoop::new` resolves aliases in its root when the path exists

- [ ] **Step 1: Write the failing test**

Append to `tests/scoop_scan.rs`:

```rust
#[test]
fn a_root_reached_through_a_symlink_still_matches_running_processes() {
    // The hole: sysinfo reports resolved paths. A root reached through a
    // junction, a subst drive or a symlink prefix-compares against the wrong
    // string, running_apps silently returns nothing, and nodejs and rustup --
    // which have no other running signal -- become prunable while running.
    //
    // A symlink is the portable stand-in for a Windows junction.
    let real = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(real.path().join("apps/nodejs/current")).unwrap();

    let link_parent = tempfile::tempdir().unwrap();
    let link = link_parent.path().join("aliased-root");
    #[cfg(unix)]
    std::os::unix::fs::symlink(real.path(), &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(real.path(), &link).unwrap();

    // The process reports the REAL path, as sysinfo would.
    let got = Scoop::new(link).running_apps(&[proc(
        "node",
        Some(real.path().join("apps/nodejs/current/node.exe")),
    )]);
    assert_eq!(got, BTreeSet::from([Name::new("nodejs")]), "aliased root must still match");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test scoop_scan a_root_reached_through_a_symlink`
Expected: FAIL — got an empty set.

- [ ] **Step 3: Implement**

In `src/backend/scoop.rs`, route both constructors through one resolver:

```rust
/// Resolve aliases so path matching compares the string `sysinfo` reports.
///
/// A path that does not exist is kept as given: a machine with no scoop is a
/// valid state, and `canonicalize` would turn it into an error.
///
/// On Windows `canonicalize` returns an extended-length `\\?\C:\...` path,
/// which would break the very comparison this exists to fix, so the prefix is
/// stripped.
fn resolve_root(root: PathBuf) -> PathBuf {
    let Ok(canon) = std::fs::canonicalize(&root) else {
        return root;
    };
    let s = canon.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => canon,
    }
}
```

and use it:

```rust
    pub fn new(root: PathBuf) -> Scoop {
        Scoop { root: resolve_root(root) }
    }
```

`discover()` builds a `PathBuf` and must go through `Scoop::new` rather than constructing the struct literal, so it gets the same treatment.

- [ ] **Step 4: Run the whole suite**

Run: `cargo test --all && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: PASS, 84 + 5 tests.

**Check specifically:** the pre-existing `running_apps` tests build roots like `/tmp/dpk-root` that do not exist, so `resolve_root` returns them unchanged and those tests are unaffected. The `tempfile`-based scan tests use roots that *do* exist, so on macOS the root becomes `/private/var/...`; they only assert on scan results, not on the root, so they should still pass. Confirm both rather than assuming.

- [ ] **Step 5: Negative control**

Make `resolve_root` the identity: `fn resolve_root(root: PathBuf) -> PathBuf { root }`.

Run: `cargo test --test scoop_scan a_root_reached_through_a_symlink`
Expected: FAIL. Paste it in, restore, confirm green.

- [ ] **Step 6: Commit**

```bash
git add src/backend/scoop.rs tests/scoop_scan.rs
git commit -m "Resolve the scoop root so path matching sees what sysinfo sees

An aliased root made running_apps return nothing, silently, and nodejs
and rustup have no other running signal."
```

---

### Task 3: The mass-prune guard

**Files:**
- Create: `src/apply.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `crate::config::Config`, `crate::state::State`, `crate::model::SCOOP`
- Produces: `pub fn mass_prune_guard(declared: &Config, state: &State) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing tests**

Create `src/apply.rs`:

```rust
use crate::config::Config;
use crate::model::SCOOP;
use crate::state::State;
use anyhow::Result;

/// Refuse a plan built from a config that declares nothing while dotpkg owns
/// something.
///
/// An empty or truncated `pkg.toml` parses successfully to zero packages —
/// every field is `#[serde(default)]` — and every owned package then becomes a
/// prune. Verified against the merged planner: five owned packages, empty
/// config, five prunes, no signal of any kind.
///
/// This is checked before anything else happens, and **`--yes` does not bypass
/// it**. `--yes` means "I have read the plan"; an empty config is file
/// corruption, so the plan itself is the thing that cannot be trusted.
/// Overriding takes its own flag.
///
/// Deliberately no ratio or count threshold. A user who genuinely deletes half
/// their `pkg.toml` is shown the plan and asked, which is the protection that
/// already exists.
pub fn mass_prune_guard(declared: &Config, state: &State) -> Result<()> {
    if !declared.scoop.packages.is_empty() {
        return Ok(());
    }
    let owned = state.owned_count(SCOOP);
    anyhow::ensure!(
        owned == 0,
        "pkg.toml declares no scoop packages but dotpkg owns {owned}. \
         Refusing to prune everything. If the file is right, pass --allow-empty-config."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Name;
    use crate::state::Ownership;

    fn owning(names: &[&str]) -> State {
        let mut s = State::default();
        for n in names {
            s.set(SCOOP, &Name::new(*n), Ownership::Installed);
        }
        s
    }

    #[test]
    fn an_empty_config_with_owned_packages_is_refused() {
        let err = mass_prune_guard(
            &crate::config::parse("").unwrap(),
            &owning(&["fzf", "bat", "ripgrep", "neovim", "kanata"]),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains('5'), "the count is the whole point: {msg}");
        assert!(msg.contains("--allow-empty-config"), "say how to override: {msg}");
    }

    #[test]
    fn an_empty_config_on_a_machine_that_owns_nothing_is_fine() {
        // A fresh machine. status should report everything as unmanaged and
        // apply should do nothing -- not error.
        mass_prune_guard(&crate::config::parse("").unwrap(), &State::default()).unwrap();
    }

    #[test]
    fn a_config_that_declares_anything_is_not_the_corruption_case() {
        mass_prune_guard(
            &crate::config::parse("[scoop]\npackages = [\"fzf\"]\n").unwrap(),
            &owning(&["fzf", "bat", "ripgrep"]),
        )
        .unwrap();
    }
}
```

- [ ] **Step 2: Add `State::owned_count`**

`State`'s inner map is private. Add to `src/state.rs`:

```rust
    /// How many packages dotpkg owns for one backend. The mass-prune guard
    /// needs the number, not the names.
    pub fn owned_count(&self, backend: &str) -> usize {
        self.0.get(backend).map(|m| m.len()).unwrap_or(0)
    }
```

with a test in that file:

```rust
    #[test]
    fn owned_count_reports_per_backend() {
        let mut s = State::default();
        s.set(SCOOP, &Name::new("fzf"), Ownership::Installed);
        s.set(SCOOP, &Name::new("bat"), Ownership::Adopted);
        assert_eq!(s.owned_count(SCOOP), 2);
        assert_eq!(s.owned_count("winget"), 0);
    }
```

- [ ] **Step 3: Export the module**

Add `pub mod apply;` to `src/lib.rs`, in alphabetical position.

- [ ] **Step 4: Run**

Run: `cargo test --all && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Negative control**

Change `anyhow::ensure!(owned == 0, ...)` to `anyhow::ensure!(true, ...)`.

Run: `cargo test --lib apply`
Expected: FAIL — `an_empty_config_with_owned_packages_is_refused`. Paste it in, restore, confirm green.

- [ ] **Step 6: Commit**

```bash
git add src/apply.rs src/state.rs src/lib.rs
git commit -m "Refuse to prune everything because pkg.toml went empty

An empty file parses to zero packages and turns every owned package into
a prune, with no signal. --yes does not bypass this: an empty config is
file corruption, so the plan itself is what cannot be trusted."
```

---

### Task 4: Recover a pinned manifest from its bucket commit

The heart of the phase, and the one place the reproducibility claim becomes real. Fully testable on macOS against a real git repository.

**Files:**
- Modify: `src/backend/scoop.rs`
- Create: `tests/prepare.rs`

**Interfaces:**
- Consumes: `crate::lock::Pin`, `crate::model::Name`
- Produces: `pub fn Scoop::stage(&self, staging_root: &Path, app: &Name, pin: &Pin) -> anyhow::Result<PathBuf>`

- [ ] **Step 1: Write the failing tests**

Create `tests/prepare.rs`:

```rust
use dotpkg::backend::scoop::Scoop;
use dotpkg::lock::Pin;
use dotpkg::model::Name;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Build a real git repository shaped like a scoop bucket: manifests under
/// `bucket/`, one commit per version. Returns a commit sha per version, in
/// the order given.
///
/// This is git, not a stand-in for git. `stage` runs the real binary here.
fn bucket_repo(scoop_root: &Path, bucket: &str, manifest_file: &str, versions: &[&str]) -> Vec<String> {
    let dir = scoop_root.join("buckets").join(bucket);
    fs::create_dir_all(dir.join("bucket")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    let mut shas = Vec::new();
    for v in versions {
        fs::write(
            dir.join("bucket").join(manifest_file),
            format!(r#"{{"version":"{v}","bin":"tool.exe"}}"#),
        )
        .unwrap();
        git(&dir, &["add", "-A"]);
        git(
            &dir,
            &[
                "-c", "user.email=t@example.invalid",
                "-c", "user.name=t",
                "commit", "-q", "-m", "bump",
            ],
        );
        shas.push(git(&dir, &["rev-parse", "HEAD"]).trim().to_string());
    }
    shas
}

fn pin(bucket: &str, commit: &str, version: &str) -> Pin {
    Pin::ScoopCommit {
        bucket: bucket.into(),
        commit: commit.into(),
        version: version.into(),
    }
}

#[test]
fn an_old_commit_recovers_the_old_manifest_not_the_current_one() {
    // The whole reproducibility claim in one test: the bucket has moved on to
    // 2.0.0, and the lock still gets 1.0.0.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0", "2.0.0"]);

    let staged = Scoop::new(root.path().to_path_buf())
        .stage(stage_dir.path(), &Name::new("tool"), &pin("main", &shas[0], "1.0.0"))
        .unwrap();

    let text = fs::read_to_string(&staged).unwrap();
    assert!(text.contains("1.0.0"), "got {text}");
    assert!(!text.contains("2.0.0"), "recovered the current manifest, not the pinned one: {text}");
}

#[test]
fn a_commit_the_bucket_does_not_have_fails_and_stages_nothing() {
    // The approved design's second mandatory test. A lock that quietly falls
    // back to latest is worse than no lock, because it makes a guarantee that
    // is not there.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);

    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", "0000000000000000000000000000000000000000", "1.0.0"),
        )
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("0000000"), "name the commit: {msg}");
    assert!(msg.contains("main"), "name the bucket: {msg}");
    assert_eq!(
        fs::read_dir(stage_dir.path()).unwrap().count(),
        0,
        "nothing may be staged when the commit is missing"
    );
}

#[test]
fn the_staged_file_is_named_for_the_buckets_spelling_not_the_users() {
    // scoop takes the installed app name from the FILENAME, so this is what
    // makes the resulting app directory identical to what a plain
    // `scoop install tool` would create.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "Tool.json", &["1.0.0"]);

    let staged = Scoop::new(root.path().to_path_buf())
        .stage(stage_dir.path(), &Name::new("TOOL"), &pin("main", &shas[0], "1.0.0"))
        .unwrap();

    assert_eq!(staged.file_name().unwrap(), "Tool.json", "got {}", staged.display());
}

#[test]
fn a_manifest_whose_version_disagrees_with_the_lock_fails() {
    // The commit is right and the file is there, but the lock says something
    // else. Installing it would install a version nobody asked for.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);

    let err = Scoop::new(root.path().to_path_buf())
        .stage(stage_dir.path(), &Name::new("tool"), &pin("main", &shas[0], "9.9.9"))
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("9.9.9") && msg.contains("1.0.0"), "name both versions: {msg}");
}

#[test]
fn a_missing_bucket_is_named_rather_than_guessed_at() {
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let err = Scoop::new(root.path().to_path_buf())
        .stage(stage_dir.path(), &Name::new("tool"), &pin("extras", "abc123", "1.0.0"))
        .unwrap_err();
    assert!(format!("{err:#}").contains("extras"));
}

#[test]
fn a_spelling_neither_guess_finds_is_resolved_from_the_tree() {
    // `MIXEDCASE` and its folded form `mixedcase` both miss `MixedCase.json`.
    // One tree listing finds the real name -- and uses it, rather than only
    // reporting it. Without this third attempt the two cheap guesses only
    // work when the user's casing happens to match.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "MixedCase.json", &["1.0.0"]);

    let staged = Scoop::new(root.path().to_path_buf())
        .stage(stage_dir.path(), &Name::new("MIXEDCASE"), &pin("main", &shas[0], "1.0.0"))
        .unwrap();
    assert_eq!(staged.file_name().unwrap(), "MixedCase.json", "got {}", staged.display());
}

#[test]
fn an_app_the_bucket_simply_does_not_have_fails() {
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);

    let err = Scoop::new(root.path().to_path_buf())
        .stage(stage_dir.path(), &Name::new("nosuch"), &pin("main", &shas[0], "1.0.0"))
        .unwrap_err();
    assert!(format!("{err:#}").contains("nosuch"), "got {err:#}");
}

#[test]
fn a_winget_pin_in_the_scoop_map_is_an_error_not_a_panic() {
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &Pin::WingetVersion { version: "1.0.0".into() },
        )
        .unwrap_err();
    assert!(format!("{err:#}").contains("inconsistent"));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test prepare`
Expected: FAIL to compile — `no method named stage`.

- [ ] **Step 3: Implement**

Add to `src/backend/scoop.rs`:

```rust
use crate::lock::Pin;
use std::process::Command;

impl Scoop {
    /// Recover the exact manifest a lock entry names and write it where scoop
    /// can install from it. Returns the staged path.
    ///
    /// The staged file is named for the **bucket's** spelling of the app, not
    /// the user's, because `scoop install <path>` takes the installed app name
    /// from the filename — so this is what makes the resulting directory
    /// identical to a plain `scoop install <app>`.
    pub fn stage(&self, staging_root: &Path, app: &Name, pin: &Pin) -> Result<PathBuf> {
        let Pin::ScoopCommit { bucket, commit, version } = pin else {
            anyhow::bail!("{app}: the scoop lock holds a winget pin; the lock is inconsistent");
        };
        let bucket_dir = self.root.join("buckets").join(bucket);
        anyhow::ensure!(
            bucket_dir.join(".git").exists(),
            "{app}: bucket {bucket:?} is not present at {}",
            bucket_dir.display()
        );
        anyhow::ensure!(
            git_ok(&bucket_dir, &["cat-file", "-e", &format!("{commit}^{{commit}}")]),
            "{app}: commit {commit} is not in bucket {bucket:?}"
        );

        // git object paths are case-sensitive; Name is not. Try what the user
        // wrote, then the folded form.
        let mut tried: Vec<String> = Vec::new();
        for spelling in [app.to_string(), app.key().to_string()] {
            if tried.contains(&spelling) {
                continue;
            }
            tried.push(spelling.clone());
            let in_repo = format!("bucket/{spelling}.json");
            let Some(text) = git_show(&bucket_dir, commit, &in_repo)? else {
                continue;
            };
            return stage_text(
                staging_root, app, version, &format!("{spelling}.json"), &in_repo, commit, &text,
            );
        }
        // Neither guess is what the bucket calls it. One tree listing finds
        // the real name -- and uses it, rather than only reporting it.
        if let Some(real) = resolve_spelling(&bucket_dir, commit, app.key()) {
            let in_repo = format!("bucket/{real}");
            if let Some(text) = git_show(&bucket_dir, commit, &in_repo)? {
                return stage_text(staging_root, app, version, &real, &in_repo, commit, &text);
            }
        }
        anyhow::bail!(
            "{app}: bucket {bucket:?} at {commit} has no manifest for {tried:?}"
        );
    }
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `Ok(None)` when the path is absent from that commit; `Err` only when git
/// itself could not be run.
fn git_show(dir: &Path, commit: &str, path_in_repo: &str) -> Result<Option<String>> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["show", &format!("{commit}:{path_in_repo}")])
        .output()
        .with_context(|| format!("cannot run git in {}", dir.display()))?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
}

/// Validate a recovered manifest against the lock and write it out. Shared by
/// both routes into staging so the check cannot drift between them.
fn stage_text(
    staging_root: &Path,
    app: &Name,
    version: &str,
    filename: &str,
    in_repo: &str,
    commit: &str,
    text: &str,
) -> Result<PathBuf> {
    let parsed: serde_json::Value = serde_json::from_str(text)
        .with_context(|| format!("{app}: {in_repo} at {commit} is not valid JSON"))?;
    let got = parsed.get("version").and_then(|v| v.as_str()).unwrap_or("");
    anyhow::ensure!(
        got == version,
        "{app}: the lock says {version:?} but {in_repo} at {commit} is {got:?}"
    );
    let dir = staging_root.join(app.key()).join(version);
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let out = dir.join(filename);
    std::fs::write(&out, text).with_context(|| format!("cannot write {}", out.display()))?;
    Ok(out)
}

/// The bucket's own filename for this app, found case-insensitively.
///
/// Costs one tree listing, and only after the two cheap guesses have missed.
/// Returning the real spelling rather than only naming it is what lets
/// `pkg.toml` say `TOOL` while the bucket file is `Tool.json` — without this,
/// the two guesses only work when the user's casing happens to match.
fn resolve_spelling(dir: &Path, commit: &str, app_key: &str) -> Option<String> {
    let listing = Command::new("git")
        .current_dir(dir)
        .args(["ls-tree", "--name-only", commit, "bucket/"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let wanted = format!("{app_key}.json");
    listing
        .lines()
        .map(|l| l.rsplit('/').next().unwrap_or(l))
        .find(|f| f.to_ascii_lowercase() == wanted)
        .map(str::to_string)
}
```

`Context` must be in scope: the file already imports `anyhow::Result`; add `Context`.

- [ ] **Step 4: Run**

Run: `cargo test --all && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Negative control — the mandatory one**

Delete the `cat-file -e` commit check, so a missing commit falls through to the `git show` loop.

Run: `cargo test --test prepare a_commit_the_bucket_does_not_have`
Expected: FAIL. Record what the failure actually is: with the check gone, `git show` on a nonexistent commit also fails, so the test may still fail for the *right reason with the wrong message* — check whether the assertion that fails is the one naming the commit. **Report exactly which assertion fires**, because "it went red" is not the same as "it went red for the reason claimed".

Then a second, sharper control: make `git_show` return the bucket's current manifest when the pinned one is missing (`Ok(Some(std::fs::read_to_string(dir.join("bucket").join(path_in_repo.rsplit('/').next().unwrap()))?))`), which is the exact "quietly falls back to latest" behaviour the design forbids.

Run: `cargo test --test prepare`
Expected: FAIL — both `a_commit_the_bucket_does_not_have_fails_and_stages_nothing` and `an_old_commit_recovers_the_old_manifest_not_the_current_one`. Paste both. Restore, confirm green.

- [ ] **Step 6: Commit**

```bash
git add src/backend/scoop.rs tests/prepare.rs
git commit -m "Recover a pinned manifest from its bucket commit

This is where the reproducibility claim becomes real. A commit the
bucket does not have is a hard failure that stages nothing -- never a
fall back to the current manifest. Tested against a real git repository,
so it runs on any OS."
```

---

### Task 5: Prefetch, and the one guarantee a test can prove

**Files:**
- Modify: `src/backend/scoop.rs`
- Modify: `tests/prepare.rs`

**Interfaces:**
- Produces:
  - `pub fn Scoop::scoop_exe(&self) -> PathBuf`
  - `pub fn download_argv(manifest: &Path) -> Vec<String>`
  - `pub fn Scoop::download(&self, manifest: &Path) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing tests**

Append to `tests/prepare.rs`:

```rust
use dotpkg::backend::scoop::download_argv;

#[test]
fn the_download_argv_never_skips_hash_verification() {
    // The approved design forbids --skip-hash-check and this is the one place
    // it would be tempting. Hash verification is scoop's, and dotpkg does not
    // opt out of it.
    //
    // This is the whole of what a test in this repository can honestly prove
    // about the download step: what scoop then does with the argv was
    // measured, not asserted, and is covered by the Windows dogfood.
    let argv = download_argv(Path::new("/stage/tool/1.0.0/tool.json"));
    assert!(
        !argv.iter().any(|a| a.contains("skip-hash") || a == "-s"),
        "hash verification must never be skipped: {argv:?}"
    );
}

#[test]
fn the_download_argv_names_the_staged_manifest() {
    let argv = download_argv(Path::new("/stage/tool/1.0.0/tool.json"));
    assert_eq!(argv[0], "download");
    assert!(
        argv.iter().any(|a| a.ends_with("tool.json")),
        "the staged path is the point: {argv:?}"
    );
}

#[test]
fn the_scoop_entry_point_is_the_cmd_shim() {
    // Measured: scoop.ps1 cannot be exec'd by Command, and relying on PATH
    // picks up whatever the user's shell resolves. shims/scoop.cmd runs
    // non-interactively and exits 0.
    let root = tempfile::tempdir().unwrap();
    let exe = Scoop::new(root.path().to_path_buf()).scoop_exe();
    assert_eq!(exe.file_name().unwrap(), "scoop.cmd");
    assert!(exe.starts_with(std::fs::canonicalize(root.path()).unwrap()));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test prepare`
Expected: FAIL to compile — `download_argv` and `scoop_exe` do not exist.

- [ ] **Step 3: Implement**

Add to `src/backend/scoop.rs`:

```rust
/// The exact argv for prefetching a staged manifest.
///
/// Pure, and separate from the call that runs it, because the guarantee worth
/// testing here is a property of the argv — that hash verification is never
/// skipped — and not the behaviour of a subprocess no test on this platform
/// can run.
pub fn download_argv(manifest: &Path) -> Vec<String> {
    vec!["download".to_string(), manifest.to_string_lossy().into_owned()]
}

impl Scoop {
    /// Measured: `scoop.ps1` cannot be exec'd directly and bare `scoop` from
    /// `PATH` is whatever the user's shell resolves. `shims/scoop.cmd` runs
    /// non-interactively.
    pub fn scoop_exe(&self) -> PathBuf {
        self.root.join("shims").join("scoop.cmd")
    }

    /// Fetch and hash-verify the artifact a staged manifest names, without
    /// installing it. Nothing on the machine changes except scoop's cache.
    ///
    /// The exit code is the only signal this phase has: `scoop download` was
    /// not measured for silent-success behaviour the way `install` and `reset`
    /// were, and inventing a cache-path check against an unmeasured assumption
    /// would be worse than saying so.
    pub fn download(&self, manifest: &Path) -> Result<()> {
        let argv = download_argv(manifest);
        let out = Command::new(self.scoop_exe())
            .args(&argv)
            .output()
            .with_context(|| format!("cannot run {}", self.scoop_exe().display()))?;
        anyhow::ensure!(
            out.status.success(),
            "scoop download failed for {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(())
    }
}
```

- [ ] **Step 4: Run**

Run: `cargo test --all && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Negative control**

Add `"-s".to_string()` to the vector `download_argv` returns.

Run: `cargo test --test prepare the_download_argv_never_skips`
Expected: FAIL. Paste it in, restore, confirm green.

- [ ] **Step 6: Commit**

```bash
git add src/backend/scoop.rs tests/prepare.rs
git commit -m "Prefetch a staged manifest, hash verification included

scoop download takes a manifest path and verifies hashes, so a dead URL
or a bad hash is caught before anything is uninstalled. The argv is a
pure function because the one guarantee a test here can prove -- that
--skip-hash-check is never passed -- is a property of the argv."
```

---

### Task 6: The prepare driver, its report, and the CLI

**Files:**
- Modify: `src/apply.rs`, `src/render.rs`, `src/main.rs`
- Create: tests inside `src/apply.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–5
- Produces:
  - `pub enum Outcome { Ready { manifest: Option<PathBuf> }, Failed { why: String }, Skipped { why: String }, NotLocked, Report }`
  - `pub struct Prepared { pub action: Action, pub outcome: Outcome }`
  - `pub struct Preparation { pub prepared: Vec<Prepared> }` with `ready_count`, `failed_count`, `skipped_count`, `not_locked_count`, `is_ok`
  - `pub fn prepare(plan: &Plan, scoop: &Scoop, staging_root: &Path) -> Preparation`
  - `pub fn render_preparation(p: &Preparation) -> String` in `src/render.rs`
  - binary: `dotpkg apply --prepare [--config <path>] [--lock <path>] [--allow-empty-config]`

- [ ] **Step 1: Write the failing tests**

The classification is the part worth testing without a filesystem, so it is a separate pure function. Append to `src/apply.rs`:

```rust
/// How the driver reads one planned action, before any work is attempted.
///
/// `NotLocked` and `Running` are both `Action::Skip` and `status` prints both
/// as `!`, but apply must treat them differently: the user can close a running
/// app and run again, whereas a missing lock entry is something apply may not
/// fix, because resolving a version itself is forbidden.
pub fn classify(action: &Action) -> Intent { /* ... */ }

pub enum Intent {
    /// Needs a manifest staged and fetched.
    NeedsArtifact,
    /// A removal: nothing to prepare, ready by definition.
    NoArtifactNeeded,
    /// Benign; does not fail the run.
    Skip(String),
    /// Fails the run.
    NotLocked,
    /// Informational line, passed through.
    Report,
}
```

with tests:

```rust
    #[test]
    fn a_version_change_needs_an_artifact() {
        for a in [
            Action::Install { backend: SCOOP.into(), name: Name::new("a"), version: "1".into() },
            Action::Upgrade { backend: SCOOP.into(), name: Name::new("a"), from: "1".into(), to: "2".into() },
            Action::Downgrade { backend: SCOOP.into(), name: Name::new("a"), from: "2".into(), to: "1".into() },
        ] {
            assert!(matches!(classify(&a), Intent::NeedsArtifact), "{a:?}");
        }
    }

    #[test]
    fn a_prune_needs_nothing_fetched() {
        assert!(matches!(
            classify(&Action::Prune { backend: SCOOP.into(), name: Name::new("a"), version: "1".into() }),
            Intent::NoArtifactNeeded
        ));
    }

    #[test]
    fn a_running_package_is_a_skip_but_an_unlocked_one_fails_the_run() {
        // The distinction that is easy to collapse: both are Action::Skip and
        // status prints both as `!`.
        assert!(matches!(
            classify(&Action::Skip { backend: SCOOP.into(), name: Name::new("a"), reason: SkipReason::Running }),
            Intent::Skip(_)
        ));
        assert!(matches!(
            classify(&Action::Skip { backend: SCOOP.into(), name: Name::new("a"), reason: SkipReason::NotLocked }),
            Intent::NotLocked
        ));
    }

    #[test]
    fn a_declared_winget_package_does_not_fail_a_scoop_run() {
        // Failing the run because Phase 4 has not happened would make apply
        // unusable for anyone whose pkg.toml has a [winget] section, and the
        // plan already prints a `!` line for it every single run.
        assert!(matches!(
            classify(&Action::Skip {
                backend: WINGET.into(),
                name: Name::new("Git.Git"),
                reason: SkipReason::BackendNotImplemented
            }),
            Intent::Skip(_)
        ));
    }

    #[test]
    fn reports_pass_through_without_affecting_the_verdict() {
        for a in [
            Action::Unmanaged { backend: SCOOP.into(), name: Name::new("a"), version: "1".into() },
            Action::ArchDrift { backend: SCOOP.into(), name: Name::new("a"), have: "64bit".into(), want: "arm64".into() },
        ] {
            assert!(matches!(classify(&a), Intent::Report), "{a:?}");
        }
    }
```

and, for the verdict arithmetic:

```rust
    #[test]
    fn a_preparation_with_a_failure_is_not_ok_and_one_without_is() {
        let ok = Preparation { prepared: vec![Prepared {
            action: Action::Install { backend: SCOOP.into(), name: Name::new("a"), version: "1".into() },
            outcome: Outcome::Ready { manifest: None },
        }]};
        assert!(ok.is_ok());

        let bad = Preparation { prepared: vec![Prepared {
            action: Action::Install { backend: SCOOP.into(), name: Name::new("a"), version: "1".into() },
            outcome: Outcome::Failed { why: "hash mismatch".into() },
        }]};
        assert!(!bad.is_ok());

        let unlocked = Preparation { prepared: vec![Prepared {
            action: Action::Skip { backend: SCOOP.into(), name: Name::new("a"), reason: SkipReason::NotLocked },
            outcome: Outcome::NotLocked,
        }]};
        assert!(!unlocked.is_ok(), "an unlocked package must fail the run");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib apply`
Expected: FAIL to compile.

- [ ] **Step 3: Implement the driver**

`classify` is pure. `prepare` walks the plan, calls `classify`, and for `NeedsArtifact` calls `scoop.stage(...)` then `scoop.download(...)`, turning any error into `Outcome::Failed { why: format!("{e:#}") }` so one package's failure never stops the others. Nothing is mutated at any point.

`Preparation::is_ok()` is `failed_count() == 0 && not_locked_count() == 0`.

The staging root is `%LOCALAPPDATA%\dotpkg\manifests` — reuse `State::default_path()`'s parent so both live under the same `dotpkg` directory. Add:

```rust
/// `%LOCALAPPDATA%\dotpkg\manifests`, beside state.json.
///
/// Permanent, not temporary: `install.json` records this path, so a staging
/// directory that gets cleaned leaves the installed app pointing at a path
/// that no longer exists.
pub fn default_staging_root() -> PathBuf
```

- [ ] **Step 4: Render it**

In `src/render.rs`:

```rust
pub fn render_preparation(p: &Preparation) -> String
```

producing exactly the shape the design specifies:

```
  ready   scoop  ripgrep      14.1.0            (install)
  ready   scoop  bat          0.25.0 -> 0.26.1  (upgrade)
  FAILED  scoop  fzf          commit a28d0c56 is not in bucket main
  FAILED  scoop  neovim       download failed: hash mismatch
  !       scoop  kanata       running -- stop it first
  !       scoop  zellij       no lock entry -- run `dotpkg update`

  2 of 4 changes ready, 2 failed, 1 skipped, 1 not locked.
  Nothing has been changed.
```

`Nothing has been changed.` is printed unconditionally — it is the promise of the whole phase. Test that it appears even for an empty preparation.

- [ ] **Step 5: Wire the CLI**

`src/main.rs` gains an `Apply` subcommand:

```rust
    /// Bring the machine to the state pkg.toml and pkg.lock describe.
    Apply {
        #[arg(long, default_value = "pkg.toml")]
        config: PathBuf,
        #[arg(long, default_value = "pkg.lock")]
        lock: PathBuf,
        /// Stage and fetch everything the plan needs, then stop before
        /// changing anything.
        #[arg(long)]
        prepare: bool,
        /// Proceed even though pkg.toml declares nothing while dotpkg owns
        /// packages. Only pass this if the empty file is deliberate.
        #[arg(long)]
        allow_empty_config: bool,
    },
```

In this phase, `apply` **without** `--prepare` must exit with an error saying the executor lands in Phase 2b-2 — never silently do nothing, and never quietly behave as `--prepare`.

Order of operations: load config, lock and state → `mass_prune_guard` unless `--allow-empty-config` → scan → build `Running` → `plan()` → `prepare()` → render → exit non-zero if `!is_ok()`.

- [ ] **Step 6: Run the whole gate**

Run: `cargo test --all && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 7: Run it against a fabricated tree locally**

```bash
mkdir -p /tmp/dpk2/apps/fzf/current
echo '{"version":"0.74.2"}' > /tmp/dpk2/apps/fzf/current/manifest.json
printf '[scoop]\npackages = ["fzf"]\n' > /tmp/dpk2/pkg.toml
: > /tmp/dpk2/pkg.lock
SCOOP=/tmp/dpk2 cargo run -- apply --prepare --config /tmp/dpk2/pkg.toml --lock /tmp/dpk2/pkg.lock; echo "exit=$?"
```

Expected: a `!` line for `fzf` saying no lock entry, `1 not locked`, `Nothing has been changed.`, and a non-zero exit.

- [ ] **Step 8: Negative controls**

1. Make `is_ok()` ignore `not_locked_count()`.
   Run: `cargo test --lib apply a_preparation_with_a_failure` → FAIL.
2. Make `classify` return `Intent::Skip` for `SkipReason::NotLocked`.
   Run: `cargo test --lib apply a_running_package_is_a_skip` → FAIL.
3. Remove the unconditional `Nothing has been changed.` line.
   Run: the render test → FAIL.

Paste all three, restore, confirm green.

- [ ] **Step 9: Commit**

```bash
git add src tests
git commit -m "Add apply --prepare: stage and fetch everything, change nothing

Reads the plan, recovers each locked manifest from its bucket commit,
stages it where scoop can install from it, and fetches it with hash
verification. A missing lock entry fails the run; a running package does
not. The executor lands in 2b-2."
```

---

### Task 7: Dogfood on a14, against the real `~/scoop`

**Files:**
- Create: `docs/dogfood-phase2b1-2026-08-08.md`

Prepare writes only to dotpkg's staging directory and scoop's download cache. It installs, upgrades and removes nothing, so this runs against the real install with nothing at risk.

- [ ] **Step 1: Build on a14**

Copy `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` — not `target/` or `.git/` — to `C:\Users\kln\dotpkg-build` and `cargo build --release`. Phase 1 established that cross-linking from macOS does not work.

- [ ] **Step 2: Generate a `pkg.lock` with real commits**

No command produces one yet — `update` is Phase 3 — so generate it by script from the installed versions. This is the rehearsal for that command; the script belongs in the dogfood record, not in the crate. For each installed, declared app: find the newest commit of `bucket/<app>.json` in its bucket whose `version` equals the installed version, and write `{bucket, commit, version}`.

Expect this to fail for some apps — a version installed from a bucket that has since rewritten history, or an app installed from a bucket not present. Record which and why; that is data for Phase 3.

- [ ] **Step 3: Run at medium integrity**

Use the scheduled-task technique with the **XML-clone workaround** from `docs/phase2b-notes.md`: building a principal with `New-ScheduledTaskPrincipal -LogonType Interactive -RunLevel Limited` leaves the task stuck at Queued on this machine. Clone the `<Principal>` block of an already-registered task known to fire. Confirm `Medium Mandatory Level` from the task's own `whoami /groups`.

- [ ] **Step 4: Answer four questions, each able to come back "no"**

1. Do all twenty-five declared packages recover their manifests?
2. Does every recovered manifest's version match the lock?
3. Do the downloads verify?
4. Does a deliberately corrupted lock entry — a commit that does not exist — fail loudly and stage nothing?

- [ ] **Step 5: Record what contradicted the plan**

**The prediction to test: some packages will fail because their upstream URL is gone.** Scoop manifests pin a URL and a hash, and old releases get deleted. If that happens it is the phase doing its job — catching upstream rot before an uninstall rather than after. **If nothing fails at all, be suspicious and say so**; a dogfood that confirms every expectation has usually not been read carefully enough.

- [ ] **Step 6: Verify the machine is unchanged**

`~/scoop/apps` app count before and after. No app's `current` version changed. Then remove the staging directory and any scaffolding, verifying each removal rather than assuming it. Note what was left in scoop's download cache — that is a legitimate side effect of `scoop download` and should be recorded, not hidden.

- [ ] **Step 7: Commit**

```bash
git add docs/dogfood-phase2b1-2026-08-08.md
git commit -m "Record the Phase 2b-1 dogfood run against a14"
```

---

## Before merging

Per-task review missed a cross-task bug in both previous phases; only a whole-branch review caught them. After Task 7, review the branch as a whole against the design, looking specifically for:

- A path through `prepare()` that can mutate installed software. There must be none.
- Any place `--skip-hash-check` could reach the argv.
- A `Failed` outcome that does not make the run exit non-zero.
- Whether `stage()` can write outside the staging root for a hostile app name (`..`, an absolute path). `Name` is not validated; a lock is tool-written, but Phase 3 will write it from user input.

## What Phase 2b-1 deliberately leaves out

Carried into 2b-2, each an explicit decision: `scoop uninstall` and `scoop install`; post-mutation state verification, which the measurement made mandatory after every operation rather than only after uninstall; the `state.json` write path; the confirmation prompt and `--yes`; cloning a missing bucket; `--fix-arch`; and the diagnostic for a scoop root that no live process resolves into.

## Self-Review

**Spec coverage.** Each design section has a task: the three holes (Tasks 1 and 2 — the third, `NotLocked` as a failure, is Task 6's `classify`), the mass-prune guard (Task 3), manifest recovery (Task 4), prefetch (Task 5), output and CLI (Task 6), dogfood (Task 7). The design's six mandatory tests map to Tasks 1, 3, 4 and 5.

**The plan's own code was executed before this document was committed.** `stage` and its git fixture were built and run in a throwaway crate against real git — seven cases including a missing commit, a version disagreement, and a case-mismatched filename. That run found a bug in the first draft: the two cheap spelling guesses (`display`, then `key`) only find the manifest when the user's casing happens to match, so `pkg.toml` saying `TOOL` against a bucket file `Tool.json` failed instead of staging. The tree-listing step was promoted from an error-message helper to a resolver, and the validate-and-write block extracted into `stage_text` so the two routes into staging cannot drift. This is the second plan bug this discipline has caught in this project; the first was Task 1 of Phase 2a, which could not have compiled.

**Known gap, accepted.** `Scoop::download` cannot be tested on macOS or Linux, and this plan does not mock it. Task 5 says so in the test's own comment rather than letting a mock imply coverage; the Windows dogfood is what exercises it.

**Type consistency.** `Outcome`, `Prepared`, `Preparation`, `Intent` are defined once in Task 6 and used with the same names. `Scoop::stage`'s signature in Task 4 matches its call site in Task 6. `download_argv` and `Scoop::download` from Task 5 are called only from Task 6. `State::owned_count`, added in Task 3, is used only by `mass_prune_guard`.
