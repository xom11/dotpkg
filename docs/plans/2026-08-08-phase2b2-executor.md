# Phase 2b-2 — the `apply` executor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `dotpkg apply` an executor that uninstalls and installs scoop
packages, proves on disk that each mutation actually happened, records
ownership, and asks before it starts.

**Architecture:** A `Mutator` trait is the only faked seam; everything else is
verified against a real directory tree. `verdict()` compares the installed
manifest byte-for-byte against the staged one, because scoop reports failure
through neither its exit code nor a version change. The run is ordered
installs → replacements → removals, with `git` and the extraction helpers last.

**Tech Stack:** Rust 2021, `rust-version = "1.85"`, `anyhow`, `clap`, `serde`,
`serde_json`, `sysinfo`, `toml`; `tempfile` as the only dev-dependency.

**Spec:** `docs/specs/2026-08-08-phase2b2-executor-design.md`. Read it before
Task 1. The measurement table at its top is the reason for almost every
decision below.

## Global Constraints

- **Never trust a scoop exit code.** Measured on a14, scoop 0.5.3: a hash
  mismatch, a dead URL, an install over a nonexistent manifest path and an
  uninstall of an app that is not installed all exit **0**. Only an unknown
  subcommand exits 1. Any `status.success()` check on a scoop invocation is a
  bug.
- **`'<app>' (<version>) was downloaded successfully!` is printed even when the
  hash check failed.** It is not a success marker.
- **No new dependencies.** `tempfile` stays the only dev-dependency.
- Everything in this plan must build and test on macOS and Linux. No test may
  require Windows, and no test may create a file at `Scoop::scoop_exe()`'s path.
- CI runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test --all`. All three must pass at every commit.
- Baseline before Task 1: **147 tests passing** — verified, as
  72 (lib) + 3 (cli) + 30 (planner) + 15 (prepare) + 27 (scoop_scan). The
  per-task counts below are expected *deltas* from that and are a guide, not
  an assertion: if your total differs because you added a test the plan did
  not name, that is fine — say so in the task's notes rather than deleting
  the test to match the number.
- Package names are compared with `Name`, never with `String`. `Name`'s
  `PartialEq` is hand-written to compare the folded key only; **deriving it
  breaks `verdict` silently** — this was reproduced in the prototype.
- Every negative control listed in a task must actually be run, and the
  **assertion that fired** recorded in the commit message or the task's review
  notes. A negative test that only asserts a substring of an error message has
  been measured to survive the "always fail with the right words" mutation; it
  must be paired with a call-count assertion or a positive-control sibling.

## Prototype findings this plan already incorporates

Built and run in a scratch crate before this plan was written; 19 tests, and
seven mutations each proven able to turn them red.

1. `verdict` must locate `apps/<app>` by reading the directory and folding case
   with `Name`, not by `join(app.key())`. macOS and Linux are case-sensitive
   and Windows is not, so a path join makes the test fixture diverge from
   production. This failed for real in the prototype.
2. `verdict` returns a `Disagreement` **enum**, not a string. The retry gate
   needs to distinguish "nothing there at all" from "half-installed", and a
   string cannot carry that.
3. Under a `verdict`-always-fails mutation, `why.contains("install did not
   happen")` stayed green. What caught it was an assertion on the number of
   `Mutator` calls plus the positive-control tests.

---

## File structure

| File | Responsibility |
|---|---|
| `src/verify.rs` **(new)** | `Expected`, `Disagreement`, `verdict`. Pure over the filesystem; no subprocess, no network. |
| `src/execute.rs` **(new)** | `Mutator`, `Step`, `order`, `run_step`, `execute`, `Execution`. The executor loop. |
| `src/backend/scoop.rs` | Adds `uninstall_argv`, `install_argv`, the real `Mutator` impl, `download_verdict`, and the hex-commit check. |
| `src/apply.rs` | Adds `lock_coherence_guard` and `plan_to_steps`. Keeps `prepare` unchanged. |
| `src/state.rs` | Adds `remove`, `entries`, atomic `save`, `reconcile`. |
| `src/config.rs` | Parses `buckets` into `BucketDecl { name, url }`. |
| `src/plan.rs` | Architecture on the three change actions; helper-shadow fix; `is_older` doc fix. |
| `src/render.rs` | Renders an `Execution`. |
| `src/main.rs` | One extracted driver, the prompt, the flags, the three exit codes. |
| `tests/cli.rs` | `Snapshot` hashes content; new end-to-end refusal tests. |
| `tests/execute.rs` **(new)** | The executor against real trees and a lying fake. |

---

## Task 1: Make `Snapshot` see file content

Nothing else in this plan may be tested until this lands. Measured: injecting
the exact `state.json` write Phase 2b-2 adds left `cargo test --test cli` at
3/3 green while the file's content was replaced.

**Files:**
- Modify: `tests/cli.rs:83-105` (`Snapshot`), `tests/cli.rs:20` (doc comment)

**Interfaces:**
- Produces: `Snapshot` now compares `(path, content-hash)` pairs. Every later
  task's `assert_nothing_was_touched` depends on this.

- [ ] **Step 1: Write the failing test**

Add to `tests/cli.rs`:

```rust
#[test]
fn the_snapshot_notices_a_file_whose_content_changed_under_the_same_name() {
    // The property the whole "nothing was touched" assertion rests on.
    // Measured before this fix: Snapshot recorded path strings only, so an
    // in-place rewrite of state.json -- exactly what the executor adds -- was
    // invisible and the suite stayed green.
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let f = dir.path().join("state.json");
    fs::write(&f, r#"{"scoop":{"fzf":"installed"}}"#).unwrap();
    let before = Snapshot::of(dir.path(), other.path());

    fs::write(&f, r#"{"scoop":{"PWNED":"adopted"}}"#).unwrap();
    let after = Snapshot::of(dir.path(), other.path());

    assert_ne!(
        before, after,
        "a rewrite of an existing file must show up as a change"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test --test cli the_snapshot_notices -- --exact --nocapture`
Expected: FAIL, `assertion `left != right` failed: a rewrite of an existing
file must show up as a change`.

- [ ] **Step 3: Hash the bytes**

Replace `Snapshot` in `tests/cli.rs`:

```rust
/// Every path under the scoop root and the state directory, each paired with
/// a hash of its content, sorted. An install, an uninstall, or an in-place
/// rewrite of an existing file all show up as a change in this list.
///
/// The content hash is the point. Recording path names alone was measured to
/// miss a full rewrite of `state.json` -- the exact write the executor adds.
/// `DefaultHasher` is not cryptographic and does not need to be: this detects
/// an accidental write, not an adversarial one, and it adds no dependency.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot(Vec<(String, u64)>);

impl Snapshot {
    fn of(scoop: &Path, local: &Path) -> Snapshot {
        use std::hash::{Hash, Hasher};

        fn digest(path: &Path) -> u64 {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            match fs::read(path) {
                Ok(bytes) => bytes.hash(&mut h),
                // A directory, or a file we cannot read: the path itself is
                // still recorded above, so this only has to be stable.
                Err(e) => e.kind().hash(&mut h),
            }
            h.finish()
        }

        fn walk(dir: &Path, out: &mut Vec<(String, u64)>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let p = e.path();
                out.push((p.to_string_lossy().into_owned(), digest(&p)));
                if p.is_dir() {
                    walk(&p, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(scoop, &mut out);
        walk(local, &mut out);
        out.sort();
        Snapshot(out)
    }
}
```

- [ ] **Step 4: Run the whole file and the whole suite**

Run: `cargo test --test cli` — expected: 4 passed.
Run: `cargo test --all` — expected: 148 passed, 0 failed.

- [ ] **Step 5: Enforce the no-fake-scoop constraint where it cannot false-positive**

The spec asks for a source scan over `tests/` forbidding the literal `shims`.
**That scan does not work:** `tests/prepare.rs:496` already contains `shims` in
a comment, so it would fail on arrival and be deleted rather than fixed.
Enforce the actual property instead — that no fixture ever puts a file where
`Scoop::scoop_exe()` looks — as the first lines of `Fixture::run`:

```rust
    fn run(&self, args: &[&str]) -> Output {
        // A `#!/bin/sh` file at this path would buy a green "end-to-end" test
        // on macOS, where `execve` ignores the `.cmd`, that means something
        // entirely different on a Windows runner. Checked here rather than by
        // scanning the test sources, which cannot tell a comment from a call.
        assert!(
            !self.scoop.path().join("shims").join("scoop.cmd").exists(),
            "no test may provide a fake scoop binary"
        );
        // ... the existing body, unchanged
```

- [ ] **Step 6: Negative controls — prove both additions can fail**

1. Revert `digest` to `fn digest(_: &Path) -> u64 { 0 }`, run
   `cargo test --test cli the_snapshot_notices -- --exact`. Record that it
   fails with `a rewrite of an existing file must show up as a change`.
   Restore.
2. In one test, create `<scoop root>/shims/scoop.cmd` before calling `run`.
   Record that the new assertion fires with `no test may provide a fake scoop
   binary`. Remove the file and the temporary line.

- [ ] **Step 7: Commit**

```bash
git add tests/cli.rs
git commit -m "Make the cli Snapshot compare content, and forbid a fake scoop binary

Measured: injecting the state.json write Phase 2b-2 adds left the cli
suite 3/3 green while the file's content was replaced. Every later
'nothing was touched' assertion in this phase depends on this.

The no-fake-scoop rule is enforced in Fixture::run rather than by the
source scan the spec suggested: tests/prepare.rs already contains the
literal 'shims' in a comment, so that scan would fail on arrival."
```

---

## Task 2: Close two planner debts

`plan.rs:226` skips extraction helpers **above** both the prune branch and the
unmanaged report, so an owned, undeclared helper produces no line at all —
dotpkg acquires software it can never release and never mentions. And
`plan.rs:269-270`'s doc comment states a falsehood about its own function.

**Files:**
- Modify: `src/plan.rs:221-253`, `src/plan.rs:262-290`
- Test: `tests/planner.rs`

**Interfaces:**
- Produces: no signature changes. `plan()` gains one `Action::Prune` case it
  previously swallowed.

- [ ] **Step 1: Write the two failing tests**

Add to `tests/planner.rs`, using that file's existing fixture helpers:

```rust
#[test]
fn an_owned_undeclared_helper_is_pruned_rather_than_silently_kept_forever() {
    // The helper list exists to stop dotpkg reporting scoop's own extraction
    // tools as strays. It must not also stop dotpkg releasing a helper it
    // installed itself: `plan.rs`'s skip sat above the ownership check, so an
    // owned, undeclared 7zip produced no line of any kind.
    let declared = config::parse("[scoop]\npackages = [\"fzf\"]\n").unwrap();
    let lock = lock::parse("").unwrap();
    let installed = vec![installed("7zip", "26.01"), installed("dark", "3.14.1")];
    let mut state = State::default();
    state.set(SCOOP, &Name::new("7zip"), Ownership::Installed);

    let p = plan(&declared, &lock, &installed, &state, &Running::default());

    assert!(
        p.actions.iter().any(|a| matches!(
            a, Action::Prune { name, .. } if *name == Name::new("7zip")
        )),
        "an owned helper must be prunable: {:?}",
        p.actions
    );
    assert!(
        !p.actions.iter().any(|a| matches!(
            a, Action::Prune { name, .. } | Action::Unmanaged { name, .. }
                if *name == Name::new("dark")
        )),
        "an unowned helper must still be invisible: {:?}",
        p.actions
    );
}

#[test]
fn a_prerelease_suffix_does_not_reduce_to_the_release_version() {
    // src/plan.rs's own doc comment claimed 1.0.0-rc1 and 1.0.0 reduce to the
    // same [1,0,0] and compare equal. `parts` keeps every numeric run, so rc1
    // becomes [1,0,0,1], and the displayed arrow was inverted for every
    // suffixed version.
    let declared = config::parse("[scoop]\npackages = [\"tool\"]\n").unwrap();
    let lock = lock::parse(
        "[scoop.tool]\nbucket = \"main\"\ncommit = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let installed = vec![installed("tool", "1.0.0-rc1")];

    let p = plan(&declared, &lock, &installed, &State::default(), &Running::default());

    assert!(
        matches!(p.actions.first(), Some(Action::Downgrade { .. })),
        "1.0.0-rc1 -> 1.0.0 is [1,0,0,1] -> [1,0,0], which this function calls a \
         downgrade; the comment claiming they compare equal was wrong: {:?}",
        p.actions
    );
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --test planner an_owned_undeclared_helper -- --exact`
Expected: FAIL, `an owned helper must be prunable: []`.

Run: `cargo test --test planner a_prerelease_suffix -- --exact`
Expected: PASS. This one pins existing behaviour that the comment described
wrongly; it is a regression pin, not a bug fix. Record that it passed on the
first run and why.

- [ ] **Step 3: Move the helper skip below the ownership check**

In `src/plan.rs`, replace the body of the installed-but-undeclared loop:

```rust
    for inst in installed.iter().filter(|i| i.backend == SCOOP) {
        if declared_scoop.contains(&inst.name) {
            continue;
        }
        if state.owns(SCOOP, &inst.name) {
            // Ownership outranks the helper list. The list exists to stop a
            // helper scoop installed for itself being reported as a stray; a
            // helper *dotpkg* installed is dotpkg's to release, and skipping
            // it here left it unreleasable and unmentioned forever.
            if running.covers(inst) {
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
        } else if !SCOOP_HELPERS.contains(&inst.name.key()) {
            reports.push(Action::Unmanaged {
                backend: SCOOP.into(),
                name: inst.name.clone(),
                version: inst.version.clone(),
            });
        }
    }
```

- [ ] **Step 4: Fix the doc comment**

In `src/plan.rs`, replace the false sentence in `is_older`'s doc comment:

```rust
/// So its edge cases are cosmetic today — but they are not the edge cases this
/// comment used to claim. `parts` keeps **every** numeric run, so `1.0.0-rc1`
/// reduces to `[1,0,0,1]`, not `[1,0,0]`: a prerelease sorts *after* its own
/// release, and the displayed arrow is therefore inverted for suffixed
/// versions. `tests/planner.rs` pins this as a fact rather than leaving it as
/// a claim.
```

- [ ] **Step 5: Run the suite**

Run: `cargo test --all` — expected: 150 passed.

- [ ] **Step 6: Negative control**

Move the `SCOOP_HELPERS` check back above the `state.owns` check. Run
`cargo test --test planner an_owned_undeclared_helper -- --exact`; record that
it fails with `an owned helper must be prunable: []`. Restore.

- [ ] **Step 7: Commit**

```bash
git add src/plan.rs tests/planner.rs
git commit -m "Let dotpkg release a helper it installed itself

The SCOOP_HELPERS skip sat above the ownership check, so an owned,
undeclared 7zip produced no plan line at all -- neither a prune nor an
unmanaged report. Also corrects is_older's doc comment, which claimed
1.0.0-rc1 and 1.0.0 reduce to the same [1,0,0]; they do not, and the
displayed arrow is inverted for suffixed versions."
```

---

## Task 3: `State` gains release, durability, and a readable `Ownership`

`State::save` is `fs::write`; a torn write makes even `status` exit 1. There is
no way to release an entry. And `Ownership` is never read anywhere — making
`State::set` discard its argument leaves the suite green.

**Files:**
- Modify: `src/state.rs`
- Test: `src/state.rs`'s own `mod tests`

**Interfaces:**
- Produces:
  - `State::remove(&mut self, backend: &str, name: &Name) -> bool`
  - `State::ownership(&self, backend: &str, name: &Name) -> Option<Ownership>`
  - `State::names(&self, backend: &str) -> Vec<Name>`
  - `State::reconcile(&mut self, backend: &str, present: &[Name]) -> Vec<Name>`
    — drops entries with no matching installed package and returns what it
    dropped.
  - `State::save` becomes atomic (temp + rename, `.bak` kept).

- [ ] **Step 1: Write the failing tests**

Add to `src/state.rs`'s `mod tests`:

```rust
    #[test]
    fn an_entry_can_be_released_and_the_release_is_reported() {
        let mut s = State::default();
        s.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);
        assert!(s.remove(SCOOP, &Name::new("AICHAT")), "release folds case");
        assert!(!s.owns(SCOOP, &Name::new("aichat")));
        assert!(!s.remove(SCOOP, &Name::new("aichat")), "a second release is a no-op");
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
        assert_eq!(s.ownership(SCOOP, &Name::new("aichat")), Some(Ownership::Adopted));
        assert_eq!(s.ownership(SCOOP, &Name::new("fzf")), Some(Ownership::Installed));
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
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --lib state`
Expected: FAIL to compile — `no method named `remove` found`, `ownership`,
`reconcile`.

- [ ] **Step 3: Implement**

Add to `impl State` in `src/state.rs`:

```rust
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

    /// Every package dotpkg owns for one backend.
    pub fn names(&self, backend: &str) -> Vec<Name> {
        self.0
            .get(backend)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Drop entries naming a package that is not installed, returning them.
    pub fn reconcile(&mut self, backend: &str, present: &[Name]) -> Vec<Name> {
        let Some(m) = self.0.get_mut(backend) else {
            return Vec::new();
        };
        let dropped: Vec<Name> = m
            .keys()
            .filter(|n| !present.contains(n))
            .cloned()
            .collect();
        for n in &dropped {
            m.remove(n);
        }
        dropped
    }
```

Replace `save`:

```rust
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
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
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
        std::fs::rename(&tmp, path).with_context(|| {
            format!("cannot move {} into place at {}", tmp.display(), path.display())
        })
    }
```

- [ ] **Step 4: Run**

Run: `cargo test --lib state` — expected: 10 passed.
Run: `cargo test --all` — expected: 155 passed.

- [ ] **Step 5: Negative control**

Make `State::set` discard its `Ownership` argument and always store
`Ownership::Installed`. Run `cargo test --lib state`. Record that
`the_ownership_variant_is_readable_so_an_upgrade_cannot_silently_erase_adopt`
fails with `assertion left == right failed; left: Some(Installed), right:
Some(Adopted)` — and note that **before this task the same mutation left all
147 tests green**. Restore.

- [ ] **Step 6: Commit**

```bash
git add src/state.rs
git commit -m "Give State a release path, a durable save, and a readable Ownership

save() was fs::write: a torn write made every command fail, status
included. There was no way to release an entry, so a prune could never
give one back. And Ownership was written and never read -- discarding it
in set() left all 147 tests green."
```

---

## Task 4: A `commit` must be a hash, checked in two places

Verified against real git: `git cat-file -e` rejects `-oops` but accepts
`main`, `HEAD`, `@` and `refs/heads/main`. A lock saying `commit = "main"`
passes every guard, `git show main:bucket/<app>.json` returns the tip, and
`stage_text`'s version check passes whenever the tip carries the same version —
which a URL/hash correction does. The lock silently degrades to latest.

**Files:**
- Modify: `src/backend/scoop.rs` (near `ensure_plain_component`, and in `stage`)
- Modify: `src/apply.rs` (new `lock_coherence_guard` beside `mass_prune_guard`)
- Test: `tests/prepare.rs`, `src/apply.rs`'s `mod tests`

**Interfaces:**
- Produces:
  - `pub fn ensure_commit_hash(app: &Name, commit: &str) -> anyhow::Result<()>`
    in `src/backend/scoop.rs`
  - `pub fn lock_coherence_guard(declared: &Config, lock: &Lock) -> anyhow::Result<()>`
    in `src/apply.rs`
- Consumes: `Pin::ScoopCommit`, `ensure_plain_component` (both existing).

- [ ] **Step 1: Write the failing tests**

Add to `tests/prepare.rs`. It already has `bucket_repo(scoop_root, bucket,
manifest_file, versions) -> Vec<String>` (a real git repo, one commit per
version, returning the SHAs) and `pin(bucket, commit, version) -> Pin`; use
those, do not invent a new fixture.

```rust
#[test]
fn a_lock_naming_a_branch_instead_of_a_hash_is_refused_and_stages_nothing() {
    // Measured against real git: `cat-file -e main^{commit}` accepts `main`,
    // `HEAD`, `@` and `refs/heads/main` -- it resolves any revision, not only
    // an object name -- and `git show main:bucket/tool.json` then returns the
    // TIP. When the tip carries the same version (a url/hash correction),
    // stage_text's version check passes too and the pin silently means latest.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0", "2.0.0"]);
    let scoop = Scoop::new(root.path().to_path_buf());

    for rev in ["main", "HEAD", "@", "refs/heads/main"] {
        let Err(err) = scoop.stage(stage_dir.path(), &Name::new("tool"), &pin("main", rev, "2.0.0"))
        else {
            panic!("{rev:?} must not be accepted as a pin");
        };
        let msg = format!("{err:#}");
        assert!(msg.contains(rev), "name the offending value: {msg}");
        assert!(msg.contains("hex"), "say what a commit must look like: {msg}");
        // The neighbouring failure this must NOT be confused with. Without
        // this line, deleting the hex check leaves the test green whenever the
        // revision also happens to be missing from the bucket -- which is the
        // shape of negative control that has burned this project twice.
        assert!(
            !msg.contains("is not in bucket"),
            "refused for its shape, not for being absent: {msg}"
        );
    }
    assert!(
        !stage_dir.path().join("tool").exists(),
        "nothing may be staged for a refused pin"
    );
    let _ = shas;
}
```

```rust
#[test]
fn a_real_commit_hash_is_still_accepted() {
    // The positive control. Without it, `ensure_commit_hash` returning Err
    // unconditionally passes the test above.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0", "2.0.0"]);
    let scoop = Scoop::new(root.path().to_path_buf());

    let staged = scoop
        .stage(stage_dir.path(), &Name::new("tool"), &pin("main", &shas[0], "1.0.0"))
        .expect("a real 40-hex commit must still work");
    assert!(staged.exists());
}
```

Add to `src/apply.rs`'s `mod tests`:

```rust
    #[test]
    fn the_lock_coherence_guard_refuses_every_shape_that_is_decidable_without_io() {
        use crate::lock::Pin;

        let declared = crate::config::parse(
            "[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n",
        )
        .unwrap();

        let bad_commit = {
            let mut l = Lock::default();
            l.scoop.insert(
                Name::new("tool"),
                Pin::ScoopCommit {
                    bucket: "main".into(),
                    commit: "main".into(),
                    version: "1.0.0".into(),
                },
            );
            l
        };
        let msg = format!("{:#}", lock_coherence_guard(&declared, &bad_commit).unwrap_err());
        assert!(msg.contains("tool") && msg.contains("main"), "{msg}");
        assert!(msg.contains("dotpkg update"), "say how to fix it: {msg}");

        let winget_pin_in_scoop_map = {
            let mut l = Lock::default();
            l.scoop
                .insert(Name::new("tool"), Pin::WingetVersion { version: "1".into() });
            l
        };
        assert!(lock_coherence_guard(&declared, &winget_pin_in_scoop_map).is_err());

        let undeclared_bucket = {
            let mut l = Lock::default();
            l.scoop.insert(
                Name::new("tool"),
                Pin::ScoopCommit {
                    bucket: "nowhere".into(),
                    commit: "a".repeat(40),
                    version: "1.0.0".into(),
                },
            );
            l
        };
        let msg = format!(
            "{:#}",
            lock_coherence_guard(&declared, &undeclared_bucket).unwrap_err()
        );
        assert!(msg.contains("nowhere"), "name the bucket: {msg}");
    }

    #[test]
    fn a_coherent_lock_passes_the_guard() {
        // Positive control: without it, a guard that always errors passes the
        // test above.
        use crate::lock::Pin;
        let declared =
            crate::config::parse("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n").unwrap();
        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("tool"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "a".repeat(40),
                version: "1.0.0".into(),
            },
        );
        lock_coherence_guard(&declared, &lock).unwrap();
    }
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --test prepare a_lock_naming_a_branch -- --exact`
Expected: FAIL — `a branch name must not be accepted as a pin: ... Ok(...)`.

Run: `cargo test --lib apply::tests::the_lock_coherence_guard`
Expected: FAIL to compile — `cannot find function `lock_coherence_guard``.

- [ ] **Step 3: Add the hex check to the backend**

In `src/backend/scoop.rs`, beside `ensure_plain_component`:

```rust
/// Refuse a `commit` that is not a hash.
///
/// Measured against real git: `git cat-file -e <rev>^{commit}` accepts `main`,
/// `HEAD`, `@` and `refs/heads/main` — it resolves any revision expression,
/// not only an object name. So `commit = "main"` passes the existence check,
/// `git show main:bucket/<app>.json` returns the bucket **tip**, and the only
/// remaining backstop is `stage_text`'s version equality — which a same-version
/// URL/hash correction passes. The lock then means "latest", which
/// `docs/specs/2026-08-08-design.md` calls worse than having no lock at all.
///
/// 40 hex characters for SHA-1, 64 for SHA-256, lowercase as git writes them.
pub fn ensure_commit_hash(app: &Name, commit: &str) -> Result<()> {
    let ok = (commit.len() == 40 || commit.len() == 64)
        && commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    anyhow::ensure!(
        ok,
        "{app}: the lock's commit {commit:?} is not a commit hash -- it must be \
         40 (or 64) lowercase hex characters. A branch or tag name resolves to \
         whatever the bucket points at today, which is not a pin."
    );
    Ok(())
}
```

Call it in `stage`, immediately after the three `ensure_plain_component` calls
and **before** `bucket_dir` is built:

```rust
        ensure_commit_hash(app, commit)?;
```

- [ ] **Step 4: Add the guard**

In `src/apply.rs`, beside `mass_prune_guard`:

```rust
/// Refuse a lock that is incoherent in a way decidable without touching the
/// disk, before the plan is built and before anything is staged.
///
/// `Scoop::stage` re-checks the same rules, deliberately: this guard gives a
/// good whole-run message, and `stage` is a public API that Phase 3 will call
/// from somewhere else. Neither is allowed to be the only one.
pub fn lock_coherence_guard(declared: &Config, lock: &Lock) -> Result<()> {
    let buckets: BTreeSet<&str> = declared
        .scoop
        .buckets
        .iter()
        .map(|b| b.split_once('=').map(|(n, _)| n).unwrap_or(b.as_str()))
        .collect();

    for (name, pin) in &lock.scoop {
        let Pin::ScoopCommit {
            bucket,
            commit,
            version,
        } = pin
        else {
            anyhow::bail!(
                "pkg.lock [scoop.{name}] holds a winget pin. Run `dotpkg update` to rewrite it."
            );
        };
        crate::backend::scoop::ensure_plain_component(name, "bucket", bucket)
            .and_then(|()| crate::backend::scoop::ensure_plain_component(name, "version", version))
            .and_then(|()| {
                crate::backend::scoop::ensure_plain_component(name, "package name", name.key())
            })
            .and_then(|()| crate::backend::scoop::ensure_commit_hash(name, commit))
            .map_err(|e| e.context("pkg.lock is not usable. Run `dotpkg update` to rewrite it."))?;
        anyhow::ensure!(
            buckets.contains(bucket.as_str()),
            "pkg.lock [scoop.{name}] names bucket {bucket:?}, which pkg.toml does not declare. \
             Add it to [scoop] buckets, or run `dotpkg update`."
        );
    }
    Ok(())
}
```

`ensure_plain_component` becomes `pub` for this. Add
`use std::collections::BTreeSet;` and `use crate::lock::Pin;` to `src/apply.rs`.

- [ ] **Step 5: Call the guard from `main.rs`**

In `src/main.rs`'s Apply arm, immediately after the `mass_prune_guard` block:

```rust
            dotpkg::apply::lock_coherence_guard(&declared, &locked)?;
```

It is not behind `--allow-empty-config`: that flag says an empty `pkg.toml` is
deliberate, which says nothing about whether the lock is readable.

- [ ] **Step 6: Run**

Run: `cargo test --all` — expected: 159 passed.

- [ ] **Step 7: Negative controls — both sites, independently**

1. Delete the `ensure_commit_hash(app, commit)?` line from `stage`. Run
   `cargo test --test prepare a_lock_naming_a_branch -- --exact`; record the
   failure. Restore.
2. Delete the `.and_then(|()| ...ensure_commit_hash...)` from
   `lock_coherence_guard`. Run
   `cargo test --lib apply::tests::the_lock_coherence_guard`; record the
   failure. Restore.

Both must be shown individually. If only one control is run, the two call
sites are not independently covered.

- [ ] **Step 8: Commit**

```bash
git add src/backend/scoop.rs src/apply.rs src/main.rs tests/prepare.rs
git commit -m "Refuse a pkg.lock commit that is not a hash

Verified against real git: cat-file -e rejects a leading dash -- which is
what the carried notes relied on -- and accepts main, HEAD, @ and
refs/heads/main. commit = \"main\" passed every guard and staged the
bucket tip; when the tip carries the same version, stage_text's version
check passes too and the pin silently means latest.

Checked in stage() and in a new lock_coherence_guard, with a negative
control run for each site separately."
```

---

## Task 5: `verdict` — the only evidence a mutation happened

**Files:**
- Create: `src/verify.rs`
- Modify: `src/lib.rs` (add `pub mod verify;`)
- Test: `src/verify.rs`'s own `mod tests`

**Interfaces:**
- Produces:
  - `pub enum Expected { Absent, Present { staged: PathBuf } }`
  - `pub enum Disagreement { NotInstalled, HalfInstalled { leftover: PathBuf }, ContentDiffers, LineEndingsDiffer, StillPresent { leftover: PathBuf }, Unreadable(String) }`
  - `pub fn verdict(root: &Path, app: &Name, want: &Expected) -> Result<(), Disagreement>`
  - `impl std::fmt::Display for Disagreement`
- Consumes: `crate::model::Name`.

- [ ] **Step 1: Write the failing tests**

Create `src/verify.rs` containing only the test module first, so the tests fail
to compile against absent items — then fill in the implementation in Step 3.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const BODY_A: &str = r#"{"version":"1.0.0","url":"https://good/v1.zip","hash":"aaaa"}"#;
    const BODY_B: &str = r#"{"version":"1.0.0","url":"https://evil/v1.zip","hash":"bbbb"}"#;

    struct Tree(tempfile::TempDir);
    impl Tree {
        fn new() -> Tree {
            Tree(tempfile::tempdir().unwrap())
        }
        fn root(&self) -> &Path {
            self.0.path()
        }
        fn stage(&self, app: &str, version: &str, body: &str) -> PathBuf {
            let d = self.root().join("stage").join(app).join(version);
            std::fs::create_dir_all(&d).unwrap();
            let p = d.join(format!("{app}.json"));
            std::fs::write(&p, body).unwrap();
            p
        }
        /// A clean install: `current/manifest.json`, byte-identical to the
        /// staged file. Measured on a14 -- scoop copies the manifest verbatim.
        fn install(&self, dir_name: &str, body: &str) {
            let cur = self.root().join("apps").join(dir_name).join("current");
            std::fs::create_dir_all(&cur).unwrap();
            std::fs::write(cur.join("manifest.json"), body).unwrap();
        }
        /// The measured residue of a failed install: `apps/<app>/<version>/`
        /// holding only the archive, no `current`, no manifest.
        fn half_install(&self, dir_name: &str, version: &str) {
            let d = self.root().join("apps").join(dir_name).join(version);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("thing.zip"), b"PK\x03\x04").unwrap();
        }
        fn empty_apps(&self) {
            std::fs::create_dir_all(self.root().join("apps")).unwrap();
        }
    }

    #[test]
    fn a_clean_install_agrees() {
        let t = Tree::new();
        let staged = t.stage("fzf", "1.0.0", BODY_A);
        t.install("fzf", BODY_A);
        assert_eq!(verdict(t.root(), &Name::new("fzf"), &Expected::Present { staged }), Ok(()));
    }

    #[test]
    fn a_same_version_content_swap_is_caught_where_a_version_check_would_not_be() {
        // The `commit = "main"` hole: both manifests say version 1.0.0 and
        // only the url and hash differ. This is why the comparison is bytes.
        let t = Tree::new();
        let staged = t.stage("tool", "1.0.0", BODY_A);
        t.install("tool", BODY_B);
        assert_eq!(
            verdict(t.root(), &Name::new("tool"), &Expected::Present { staged }),
            Err(Disagreement::ContentDiffers)
        );
    }

    #[test]
    fn the_silent_no_op_install_scoop_was_measured_doing_is_caught() {
        let t = Tree::new();
        let staged = t.stage("fzf", "0.74.2", r#"{"version":"0.74.2"}"#);
        t.install("fzf", r#"{"version":"0.74.1"}"#);
        assert_eq!(
            verdict(t.root(), &Name::new("fzf"), &Expected::Present { staged }),
            Err(Disagreement::ContentDiffers)
        );
    }

    #[test]
    fn the_measured_failed_install_residue_is_its_own_diagnosis() {
        let t = Tree::new();
        let staged = t.stage("badhash", "0.74.1", BODY_A);
        t.half_install("badhash", "0.74.1");
        assert_eq!(
            verdict(t.root(), &Name::new("badhash"), &Expected::Present { staged }),
            Err(Disagreement::HalfInstalled {
                leftover: t.root().join("apps").join("badhash")
            })
        );
    }

    #[test]
    fn nothing_at_all_is_not_installed() {
        let t = Tree::new();
        let staged = t.stage("fzf", "1.0.0", BODY_A);
        t.empty_apps();
        assert_eq!(
            verdict(t.root(), &Name::new("fzf"), &Expected::Present { staged }),
            Err(Disagreement::NotInstalled)
        );
    }

    #[test]
    fn absent_means_absent_and_a_leftover_is_named() {
        let t = Tree::new();
        t.empty_apps();
        assert_eq!(verdict(t.root(), &Name::new("fzf"), &Expected::Absent), Ok(()));
        t.half_install("fzf", "1.0.0");
        assert_eq!(
            verdict(t.root(), &Name::new("fzf"), &Expected::Absent),
            Err(Disagreement::StillPresent {
                leftover: t.root().join("apps").join("fzf")
            })
        );
    }

    #[test]
    fn the_app_directory_is_found_by_folding_case_not_by_the_platforms_rules() {
        // scoop names the directory after the BUCKET's spelling. Windows finds
        // `Tool` when asked for `tool`; macOS and Linux do not, so a path join
        // would make this fixture diverge from production. Found by a real
        // failure while prototyping.
        let t = Tree::new();
        let staged = t.stage("tool", "1.0.0", BODY_A);
        t.install("Tool", BODY_A);
        assert_eq!(
            verdict(t.root(), &Name::new("tool"), &Expected::Present { staged }),
            Ok(())
        );
        assert!(matches!(
            verdict(t.root(), &Name::new("TOOL"), &Expected::Absent),
            Err(Disagreement::StillPresent { .. })
        ));
    }

    #[test]
    fn a_line_ending_difference_is_reported_as_itself() {
        let t = Tree::new();
        let staged = t.stage("tool", "1.0.0", "{\n  \"version\": \"1.0.0\"\n}");
        t.install("tool", "{\r\n  \"version\": \"1.0.0\"\r\n}");
        assert_eq!(
            verdict(t.root(), &Name::new("tool"), &Expected::Present { staged }),
            Err(Disagreement::LineEndingsDiffer)
        );
    }

    #[test]
    fn a_machine_with_no_apps_directory_is_absent_not_an_error() {
        let t = Tree::new();
        assert_eq!(verdict(t.root(), &Name::new("fzf"), &Expected::Absent), Ok(()));
    }

    #[test]
    fn every_disagreement_says_something_a_user_can_act_on() {
        for d in [
            Disagreement::NotInstalled,
            Disagreement::HalfInstalled { leftover: PathBuf::from("/a/b") },
            Disagreement::ContentDiffers,
            Disagreement::LineEndingsDiffer,
            Disagreement::StillPresent { leftover: PathBuf::from("/a/b") },
            Disagreement::Unreadable("boom".into()),
        ] {
            let s = d.to_string();
            assert!(!s.trim().is_empty(), "{d:?} renders empty");
            assert!(s.len() > 10, "{d:?} renders as {s:?}");
        }
    }
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo test --lib verify`
Expected: FAIL to compile — `cannot find type `Expected` in this scope`.

- [ ] **Step 3: Implement**

Put this above the test module in `src/verify.rs`:

```rust
//! Did the mutation actually happen?
//!
//! Measured on a14, scoop 0.5.3: `scoop` exits **0** for a hash mismatch, a
//! dead URL, an install over a nonexistent manifest path, and an uninstall of
//! an app that is not installed. Only an unknown subcommand exits 1 — and this
//! is not the `.cmd` shim: `scoop.ps1` invoked directly reports
//! `$LASTEXITCODE=0` too.
//!
//! So this module is not a second safety net. It is the only signal there is.

use crate::model::Name;
use std::path::{Path, PathBuf};

/// What the executor asked scoop to make true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// After an uninstall.
    Absent,
    /// After an install: the app's manifest must be the one that was staged.
    Present { staged: PathBuf },
}

/// How the disk disagrees. An enum rather than a string, because the retry
/// gate has to tell "nothing there" apart from "half-installed": retrying over
/// a half-install gets `WARN … is already installed`, exit 0, and no change —
/// manufacturing exactly the silent success this module exists to catch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disagreement {
    NotInstalled,
    HalfInstalled { leftover: PathBuf },
    ContentDiffers,
    LineEndingsDiffer,
    StillPresent { leftover: PathBuf },
    Unreadable(String),
}

impl std::fmt::Display for Disagreement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Disagreement::NotInstalled => {
                write!(f, "the app directory is not there at all")
            }
            Disagreement::HalfInstalled { leftover } => write!(
                f,
                "a partial install is left at {} -- there is no current/manifest.json",
                leftover.display()
            ),
            Disagreement::ContentDiffers => write!(
                f,
                "the installed manifest is not the one that was staged"
            ),
            Disagreement::LineEndingsDiffer => write!(
                f,
                "the installed manifest matches the staged one except for line endings"
            ),
            Disagreement::StillPresent { leftover } => {
                write!(f, "it is still on disk at {}", leftover.display())
            }
            Disagreement::Unreadable(why) => write!(f, "could not look: {why}"),
        }
    }
}

/// Find `<root>/apps/<app>` by folding case, the way the filesystem that wrote
/// it does.
///
/// Not `join(app.key())`: scoop names the directory after the **bucket's**
/// spelling, Windows resolves `apps/tool` to `Tool` and macOS does not, so a
/// path join makes every fixture on this developer's machine mean something
/// different from production. Reproduced while prototyping.
fn app_dir(root: &Path, app: &Name) -> Result<Option<PathBuf>, String> {
    let apps = root.join("apps");
    let entries = match std::fs::read_dir(&apps) {
        Ok(e) => e,
        // A machine with no scoop is a valid state, and so is one mid-setup.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", apps.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read an entry of {}: {e}", apps.display()))?;
        if Name::new(entry.file_name().to_string_lossy().to_string()) == *app {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

/// Collapse CRLF and drop trailing newlines, for telling a line-ending
/// difference apart from a content difference. Never used to *accept* a
/// mismatch — only to describe one.
fn normalise(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    while out.last() == Some(&b'\n') {
        out.pop();
    }
    out
}

/// Compare what is on disk against what was asked for. No subprocess, no
/// network, no exit code.
pub fn verdict(root: &Path, app: &Name, want: &Expected) -> Result<(), Disagreement> {
    let dir = app_dir(root, app).map_err(Disagreement::Unreadable)?;
    match want {
        Expected::Absent => match dir {
            None => Ok(()),
            Some(leftover) => Err(Disagreement::StillPresent { leftover }),
        },
        Expected::Present { staged } => {
            let Some(dir) = dir else {
                return Err(Disagreement::NotInstalled);
            };
            let observed = dir.join("current").join("manifest.json");
            let got = match std::fs::read(&observed) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(Disagreement::HalfInstalled { leftover: dir })
                }
                Err(e) => {
                    return Err(Disagreement::Unreadable(format!(
                        "cannot read {}: {e}",
                        observed.display()
                    )))
                }
            };
            let want_bytes = std::fs::read(staged).map_err(|e| {
                Disagreement::Unreadable(format!("cannot read {}: {e}", staged.display()))
            })?;
            if got == want_bytes {
                Ok(())
            } else if normalise(&got) == normalise(&want_bytes) {
                Err(Disagreement::LineEndingsDiffer)
            } else {
                Err(Disagreement::ContentDiffers)
            }
        }
    }
}
```

Add `pub mod verify;` to `src/lib.rs`, keeping the list alphabetical (after
`pub mod sys;`).

- [ ] **Step 4: Run**

Run: `cargo test --lib verify` — expected: 10 passed.
Run: `cargo test --all` — expected: 169 passed.

- [ ] **Step 5: Negative controls — three, each proven separately**

1. Replace the byte comparison with a version comparison: parse both as JSON
   and compare only the `version` field. Run `cargo test --lib verify`; record
   that `a_same_version_content_swap_is_caught_where_a_version_check_would_not_be`
   fails. This is the control that justifies comparing bytes at all. Restore.
2. Replace `app_dir` with `Ok(Some(root.join("apps").join(app.key())))`. Run
   `cargo test --lib verify`; record that
   `the_app_directory_is_found_by_folding_case_not_by_the_platforms_rules`
   fails on macOS. Restore.
3. Make `verdict` return `Ok(())` unconditionally. Run `cargo test --lib
   verify`; record how many fail (expect all but
   `every_disagreement_says_something_a_user_can_act_on`). Restore.

- [ ] **Step 6: Commit**

```bash
git add src/verify.rs src/lib.rs
git commit -m "Add verify::verdict -- the only evidence a mutation happened

scoop exits 0 for a hash mismatch, a dead URL, an install over a
nonexistent path, and an uninstall of an app that is not installed. The
comparison is bytes, not versions, because commit = \"main\" installs the
bucket tip under the pinned version number and a version check cannot see
it. The app directory is found by folding case rather than by joining a
path: scoop names it after the bucket's spelling, and macOS is
case-sensitive where Windows is not."
```

---

## Task 6: Read `scoop download`'s verdict from its stdout

`Scoop::download`'s `ensure!(out.status.success(), …)` can never fire. Phase
2b-1's promise that a bad fetch becomes "nothing happened, here is why" is not
implemented.

**Files:**
- Modify: `src/backend/scoop.rs` (`download`, `download_failure_detail`)
- Test: `src/backend/scoop.rs`'s `mod tests`

**Interfaces:**
- Produces:
  - `pub enum FetchVerdict { Verified, HashFailed, UrlDead, Unproven }`
  - `pub fn download_verdict(stdout: &str) -> FetchVerdict`
  - `pub fn strip_ansi(s: &str) -> String`
- `Scoop::download` keeps its signature `(&self, manifest: &Path) -> Result<()>`
  until **Task 7** adds `arch`.

- [ ] **Step 1: Write the failing tests**

Add to `src/backend/scoop.rs`'s `mod tests`, with the strings copied verbatim
from the a14 run:

```rust
    // -- download_verdict -------------------------------------------------
    //
    // Every string below is scoop 0.5.3's real output, captured on a14 on
    // 2026-08-08 through System.Diagnostics.Process. All three exited 0.

    const OK_CACHED: &str = "INFO  Downloading 'fzf' [arm64]
Loading fzf-0.74.1-windows_arm64.zip from cache
Checking hash of fzf-0.74.1-windows_arm64.zip ... ok.
'fzf' (0.74.1) was downloaded successfully!
";

    const BAD_HASH: &str = "INFO  Downloading 'badhash' [arm64]
Downloading https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip (1.9 MB)...
Checking hash of fzf-0.74.1-windows_arm64.zip ... ERROR Hash check failed!
App:         badhash
URL:         https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip
First bytes: 50 4B 03 04 14 00 08 00
Expected:    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
Actual:      b688ecafa2d1fdb0af3383f25d6d122866c13ad7cc996e9f735bf90e6c75f83f
ERROR
Please try again or create a new issue by using the following link and paste your console output:
https:////
'badhash' (0.74.1) was downloaded successfully!
";

    const DEAD_URL: &str = "INFO  Downloading 'deadurl' [arm64]
The remote server returned an error: (404) Not Found.
ERROR URL https://github.com/xom11/definitely-not-a-real-repo-9f2a/releases/download/v9.9.9/nothing.zip is not valid
";

    #[test]
    fn the_sentence_scoop_prints_after_a_hash_failure_is_not_a_success_marker() {
        // The trap, in one test. Both of these say "was downloaded
        // successfully!" and only one of them verified anything.
        assert!(BAD_HASH.contains("was downloaded successfully!"));
        assert!(OK_CACHED.contains("was downloaded successfully!"));
        assert_eq!(download_verdict(OK_CACHED), FetchVerdict::Verified);
        assert_eq!(download_verdict(BAD_HASH), FetchVerdict::HashFailed);
    }

    #[test]
    fn a_dead_url_is_told_apart_from_a_bad_hash() {
        assert_eq!(download_verdict(DEAD_URL), FetchVerdict::UrlDead);
    }

    #[test]
    fn silence_is_failure_because_scoop_cannot_signal_it_any_other_way() {
        assert_eq!(download_verdict(""), FetchVerdict::Unproven);
        assert_eq!(
            download_verdict("INFO  Downloading 'x' [arm64]\n"),
            FetchVerdict::Unproven
        );
        assert_eq!(
            download_verdict("WARN  'fzf' (0.74.1) is already installed.\n"),
            FetchVerdict::Unproven
        );
    }

    #[test]
    fn ansi_colour_cannot_hide_a_failure() {
        let coloured = BAD_HASH.replace("ERROR", "\u{1b}[31;1mERROR\u{1b}[0m");
        assert_eq!(download_verdict(&coloured), FetchVerdict::HashFailed);
    }

    #[test]
    fn one_verified_url_does_not_excuse_a_second_that_failed() {
        let mixed = "Checking hash of a.zip ... ok.\n\
                     Checking hash of b.zip ... ERROR Hash check failed!\n";
        assert_eq!(download_verdict(mixed), FetchVerdict::HashFailed);
    }
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --lib backend::scoop::tests::the_sentence_scoop_prints`
Expected: FAIL to compile — `cannot find function `download_verdict``.

- [ ] **Step 3: Implement**

Replace `download_failure_detail` and `Scoop::download` in
`src/backend/scoop.rs`:

```rust
/// Drop ANSI SGR sequences. scoop colours its output, and a colour code
/// between `ERROR` and the rest of a line would hide a failure marker.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What `scoop download` actually did, read from its stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchVerdict {
    Verified,
    HashFailed,
    UrlDead,
    /// Neither a success marker nor a known failure marker. Fail-closed.
    Unproven,
}

/// `scoop download` exits 0 whatever happens — measured on a14 for a hash
/// mismatch and for a 404 — so the verdict comes from stdout.
///
/// **`'<app>' (<version>) was downloaded successfully!` is printed even when
/// the hash check failed.** It is not a success marker, and treating it as one
/// is the single most dangerous mistake available in this function.
///
/// The only success marker is `Checking hash of … ok.`, and its absence is
/// failure rather than doubt: a manifest that declares no `url`/`hash` prints
/// none of these and is refused. That is a known limitation, and refusing is
/// the direction that cannot lose data.
///
/// stderr is deliberately not consulted. Measured: it is non-empty on a
/// *successful* run, carrying non-fatal `Cannot find path …` noise.
pub fn download_verdict(stdout: &str) -> FetchVerdict {
    let clean = strip_ansi(stdout);
    if clean.contains("ERROR Hash check failed!") {
        return FetchVerdict::HashFailed;
    }
    let dead = clean.lines().any(|l| {
        let t = l.trim();
        t.starts_with("ERROR URL ") && t.ends_with(" is not valid")
    });
    if dead {
        return FetchVerdict::UrlDead;
    }
    let verified = clean.lines().any(|l| {
        l.trim_start().starts_with("Checking hash of ") && l.trim_end().ends_with("... ok.")
    });
    if verified {
        FetchVerdict::Verified
    } else {
        FetchVerdict::Unproven
    }
}

/// The last few lines of scoop's stdout, for an error message.
fn tail(stdout: &str) -> String {
    const TAIL_LINES: usize = 20;
    let clean = strip_ansi(stdout);
    let trimmed = clean.trim();
    if trimmed.is_empty() {
        return "scoop printed nothing at all".to_string();
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    match lines.len().checked_sub(TAIL_LINES) {
        Some(skip) if skip > 0 => format!("(last {TAIL_LINES} lines) {}", lines[skip..].join("\n")),
        _ => trimmed.to_string(),
    }
}

impl Scoop {
    /// Fetch and hash-verify the artifact a staged manifest names.
    ///
    /// The exit code is read and ignored: measured, `scoop download` returns 0
    /// for a hash mismatch and for a dead URL. `download_verdict` reads what
    /// scoop actually said.
    pub fn download(&self, manifest: &Path) -> Result<()> {
        let argv = download_argv(manifest);
        let out = Command::new(self.scoop_exe())
            .args(&argv)
            .output()
            .with_context(|| format!("cannot run {}", self.scoop_exe().display()))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        match download_verdict(&stdout) {
            FetchVerdict::Verified => Ok(()),
            FetchVerdict::HashFailed => anyhow::bail!(
                "hash check failed for {}: {}",
                manifest.display(),
                tail(&stdout)
            ),
            FetchVerdict::UrlDead => anyhow::bail!(
                "the manifest's url is gone for {}: {}",
                manifest.display(),
                tail(&stdout)
            ),
            FetchVerdict::Unproven => anyhow::bail!(
                "scoop download did not report a verified hash for {} (it exits 0 either way, \
                 so this is treated as a failure): {}",
                manifest.display(),
                tail(&stdout)
            ),
        }
    }
}
```

Delete the four `download_failure_detail` tests; they cover a removed function.

- [ ] **Step 4: Run**

Run: `cargo test --all` — expected: 170 passed (169 − 4 removed + 5 added).

- [ ] **Step 5: Negative control**

Make `download_verdict` return `FetchVerdict::Verified` unconditionally. Run
`cargo test --lib backend::scoop`; record that five tests fail, naming
`the_sentence_scoop_prints_after_a_hash_failure_is_not_a_success_marker`.
Restore.

Then a second control that matters more: change the success marker to
`was downloaded successfully!`. Record that
`the_sentence_scoop_prints_after_a_hash_failure_is_not_a_success_marker` fails
and the others pass — this is the mistake the test exists to prevent. Restore.

- [ ] **Step 6: Commit**

```bash
git add src/backend/scoop.rs
git commit -m "Read scoop download's verdict from stdout, not from its exit code

ensure!(status.success()) could never fire: measured, scoop download
exits 0 for a hash mismatch and for a dead URL. Phase 2b-1's promise that
a bad fetch becomes 'nothing happened, here is why' was not implemented.

The trap is that scoop prints \"'<app>' (<v>) was downloaded
successfully!\" after a failed hash check, so the only usable success
marker is 'Checking hash of ... ok.' and its absence is failure."
```

---

## Task 7: `Mutator`, and the argv for uninstall and install

**Files:**
- Modify: `src/backend/scoop.rs`
- Create: `src/execute.rs` (the trait only; the loop arrives in Task 9)
- Modify: `src/lib.rs`
- Test: `src/backend/scoop.rs`'s `mod tests`

**Interfaces:**
- Produces:
  - `pub struct CommandReport { pub code: Option<i32>, pub stdout: String, pub stderr: String }`
  - `pub trait Mutator { fn uninstall(&self, app: &Name) -> anyhow::Result<CommandReport>; fn install(&self, manifest: &Path, arch: Option<&str>) -> anyhow::Result<CommandReport>; }`
    in `src/execute.rs`
  - `pub fn uninstall_argv(app: &Name) -> Vec<String>` in `src/backend/scoop.rs`
  - `pub fn install_argv(manifest: &Path, arch: Option<&str>) -> Vec<String>`
  - `impl Mutator for Scoop`

- [ ] **Step 1: Write the failing tests**

Add to `src/backend/scoop.rs`'s `mod tests`:

```rust
    #[test]
    fn the_uninstall_argv_is_exactly_this_and_never_purges() {
        // -p/--purge deletes the user's persisted data. It is opt-in in scoop
        // and dotpkg never opts in: the uninstall+install window is supposed
        // to risk binaries and shims, not somebody's config.
        assert_eq!(uninstall_argv(&Name::new("FZF")), vec!["uninstall", "fzf"]);
        let argv = uninstall_argv(&Name::new("fzf"));
        assert!(!argv.iter().any(|a| a == "-p" || a == "--purge"), "{argv:?}");
        assert!(!argv.iter().any(|a| a == "-g" || a == "--global"), "{argv:?}");
    }

    #[test]
    fn the_install_argv_names_the_staged_path_and_always_passes_no_update_scoop() {
        let m = Path::new("/stage/fzf/0.74.1/fzf.json");
        assert_eq!(
            install_argv(m, Some("arm64")),
            vec!["install", "-u", "-a", "arm64", "/stage/fzf/0.74.1/fzf.json"]
        );
        assert_eq!(
            install_argv(m, None),
            vec!["install", "-u", "/stage/fzf/0.74.1/fzf.json"]
        );
    }

    #[test]
    fn no_argv_this_crate_builds_ever_skips_hash_checking() {
        let m = Path::new("/stage/fzf/0.74.1/fzf.json");
        for argv in [
            install_argv(m, Some("arm64")),
            install_argv(m, None),
            download_argv(m),
            uninstall_argv(&Name::new("fzf")),
        ] {
            assert!(
                !argv.iter().any(|a| a == "-s" || a == "--skip-hash-check"),
                "{argv:?}"
            );
        }
    }

    #[test]
    fn every_scoop_argv_is_built_by_a_named_function() {
        // The argv tests above are only honest if there is exactly one
        // construction site per command. An inline `.args([...])` would slip
        // past all of them.
        //
        // `git` argv are exempt: they are built inline on purpose in
        // `git_show` and `resolve_spelling`, and neither is a scoop
        // invocation. Verified at plan time: exactly two such sites exist
        // (`git_ok` takes a slice variable, so it does not match).
        let src = include_str!("scoop.rs");
        let inline = src.matches(".args([").count();
        assert_eq!(
            inline, 2,
            "the two inline .args([..]) belong to git (git_show, resolve_spelling); \
             build every SCOOP argv in a *_argv function so the tests above cover it"
        );
    }
```

The `2` was checked against the tree, not assumed:
`grep -c '\.args(\[' src/backend/scoop.rs` → 2, at lines 426 and 469. If your
count differs, the file changed — say so in the task report rather than
editing the number to match.

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --lib backend::scoop::tests::the_uninstall_argv`
Expected: FAIL to compile — `cannot find function `uninstall_argv``.

- [ ] **Step 3: Implement the argv builders**

In `src/backend/scoop.rs`, beside `download_argv`:

```rust
/// The exact argv for removing an installed app.
///
/// `app.key()`, not the display form: scoop resolves names case-insensitively
/// and the folded key is the one thing that cannot depend on how the user
/// spelled it in `pkg.toml`.
///
/// **Never `-p`/`--purge`.** Measured: without it, `scoop uninstall` keeps
/// everything under `persist`, so the window this opens risks binaries and
/// shims and not the user's data. Adding it would silently change that.
pub fn uninstall_argv(app: &Name) -> Vec<String> {
    vec!["uninstall".to_string(), app.key().to_string()]
}

/// The exact argv for installing a staged manifest.
///
/// `-u`/`--no-update-scoop` keeps a scoop self-update and a bucket `git pull`
/// out of the window between an uninstall and its install. Measured: it is
/// accepted alongside a manifest path.
///
/// `-a` is passed whenever an architecture is known, because `scoop download`
/// without it fetches the *default* architecture's artifact — measured, two
/// different files for one version — and an install that then wants the other
/// one reaches the network from inside the window.
pub fn install_argv(manifest: &Path, arch: Option<&str>) -> Vec<String> {
    let mut argv = vec!["install".to_string(), "-u".to_string()];
    if let Some(a) = arch {
        argv.push("-a".to_string());
        argv.push(a.to_string());
    }
    argv.push(manifest.to_string_lossy().into_owned());
    argv
}
```

Change `download_argv` to take an architecture, same reasoning:

```rust
pub fn download_argv(manifest: &Path, arch: Option<&str>) -> Vec<String> {
    let mut argv = vec!["download".to_string()];
    if let Some(a) = arch {
        argv.push("-a".to_string());
        argv.push(a.to_string());
    }
    argv.push(manifest.to_string_lossy().into_owned());
    argv
}
```

Update `Scoop::download` to `pub fn download(&self, manifest: &Path, arch: Option<&str>)`
and pass `arch` through. Update `stage_and_fetch` in `src/apply.rs` to call
`scoop.download(&manifest, None)` for now; Task 8 threads the real value.
Update the existing `download_argv` test in `tests/prepare.rs` accordingly.

- [ ] **Step 4: Create the trait and the real impl**

Create `src/execute.rs`:

```rust
//! The executor: the only part of dotpkg that changes installed software.
//!
//! One seam is faked in tests — `Mutator`, the scoop subprocess. Everything
//! else, including every observation of the result, runs against a real
//! directory tree, because a fake that both performs and reports the mutation
//! proves only that it is self-consistent.

use crate::model::Name;
use anyhow::Result;
use std::path::Path;

/// What one scoop invocation said.
///
/// `code` is recorded and never believed. Measured on a14: scoop exits 0 for a
/// hash mismatch, a dead URL, an install over a nonexistent manifest path, and
/// an uninstall of an app that is not installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandReport {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Every scoop invocation that changes installed software.
///
/// `Err` means the process could not be run at all. It does **not** mean the
/// operation failed — that is `verify::verdict`'s answer, and only its answer.
pub trait Mutator {
    fn uninstall(&self, app: &Name) -> Result<CommandReport>;
    fn install(&self, manifest: &Path, arch: Option<&str>) -> Result<CommandReport>;
}
```

In `src/backend/scoop.rs`:

```rust
impl crate::execute::Mutator for Scoop {
    fn uninstall(&self, app: &Name) -> Result<crate::execute::CommandReport> {
        self.run(&uninstall_argv(app))
    }
    fn install(
        &self,
        manifest: &Path,
        arch: Option<&str>,
    ) -> Result<crate::execute::CommandReport> {
        self.run(&install_argv(manifest, arch))
    }
}

impl Scoop {
    /// Run scoop and capture everything it said. The exit code is recorded,
    /// not judged.
    fn run(&self, argv: &[String]) -> Result<crate::execute::CommandReport> {
        let out = Command::new(self.scoop_exe())
            .args(argv)
            .output()
            .with_context(|| format!("cannot run {}", self.scoop_exe().display()))?;
        Ok(crate::execute::CommandReport {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}
```

Add `pub mod execute;` to `src/lib.rs`.

- [ ] **Step 5: Run**

Run: `cargo test --all` — expected: 174 passed.
Run: `cargo clippy --all-targets -- -D warnings` — expected: clean.

- [ ] **Step 6: Negative control**

Add `argv.push("--purge".to_string());` to `uninstall_argv`. Run
`cargo test --lib backend::scoop`; record that
`the_uninstall_argv_is_exactly_this_and_never_purges` fails on the
whole-vector equality **and** on the `-p`/`--purge` assertion. Restore.

Then add an inline `Command::new(self.scoop_exe()).args(["list"])` anywhere in
`scoop.rs`; record that `every_scoop_argv_is_built_by_a_named_function` fails.
Restore.

- [ ] **Step 7: Commit**

```bash
git add src/backend/scoop.rs src/execute.rs src/lib.rs src/apply.rs tests/prepare.rs
git commit -m "Add the Mutator seam and the uninstall/install argv

-u keeps a scoop self-update and a bucket git pull out of the window
between an uninstall and its install; -a is threaded through download as
well as install, because download without it fetches the default
architecture's artifact and the install would then reach the network from
inside the window. Never -p, so persisted data survives; never
--skip-hash-check, enforced across every argv this crate builds."
```

---

## Task 8: Architecture is resolved at plan time and appears in the plan

**Files:**
- Modify: `src/plan.rs` (`Action::Install`, `Upgrade`, `Downgrade` gain `arch`)
- Modify: `src/render.rs`, `src/apply.rs`
- Test: `tests/planner.rs`, `src/render.rs`'s `mod tests`

**Interfaces:**
- Produces: `Action::Install { backend, name, version, arch: Option<String> }`
  and the same `arch: Option<String>` field on `Upgrade` and `Downgrade`.
- Consumes: `Config::scoop.opts`, `Installed.arch`, `Arch::as_scoop`.

- [ ] **Step 1: Write the failing test**

Add to `tests/planner.rs`:

```rust
#[test]
fn the_architecture_an_install_will_use_is_decided_in_the_plan_not_in_the_executor() {
    // Three cases in one: declared wins, otherwise the installed value is
    // preserved, and `keep` means "pass no -a at all".
    let declared = config::parse(
        "[scoop]\npackages = [\"python\", \"stylua\", \"kanata\"]\n\
         [scoop.opts]\npython = { arch = \"arm64\" }\nkanata = { arch = \"keep\" }\n",
    )
    .unwrap();
    let lock = lock::parse(&[
        pin("python", "3.14.6"),
        pin("stylua", "2.5.3"),
        pin("kanata", "1.13.0"),
    ].concat()).unwrap();
    let installed = vec![
        installed_arch("python", "3.14.5", Some("64bit")),
        installed_arch("stylua", "2.5.2", Some("64bit")),
        installed_arch("kanata", "1.12.0", Some("arm64")),
    ];

    let p = plan(&declared, &lock, &installed, &State::default(), &Running::default());

    let arch_of = |n: &str| -> Option<String> {
        p.actions.iter().find_map(|a| match a {
            Action::Upgrade { name, arch, .. } if *name == Name::new(n) => Some(arch.clone()),
            _ => None,
        })?
    };
    assert_eq!(arch_of("python").as_deref(), Some("arm64"), "declared wins");
    assert_eq!(
        arch_of("stylua").as_deref(),
        Some("64bit"),
        "an undeclared package keeps the architecture it already has -- reinstalling \
         it as arm64 would be an unasked-for change"
    );
    assert_eq!(arch_of("kanata"), None, "`keep` means pass no -a at all");
}
```

`tests/planner.rs` **already has** `installed_arch(name, version, arch)` at
line 570 — do not add a second one. It does not have a lock-text helper; add
this one:

```rust
/// One `[scoop.<name>]` block with a syntactically valid 40-hex commit.
/// The planner never looks at the commit, but `lock::parse` and (from Task 4)
/// `lock_coherence_guard` both do.
fn pin(name: &str, version: &str) -> String {
    format!(
        "[scoop.{name}]\nbucket = \"main\"\ncommit = \"{}\"\nversion = \"{version}\"\n\n",
        "a".repeat(40)
    )
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo test --test planner the_architecture_an_install_will_use -- --exact`
Expected: FAIL to compile — `struct variant `Action::Upgrade` has no field
named `arch``.

- [ ] **Step 3: Implement**

In `src/plan.rs`, add `arch: Option<String>` to the three change variants, and
resolve it once per declared package, above the `match current`:

```rust
        // Resolved here, not in the executor, so the architecture an install
        // will actually use is visible in the plan the user confirms.
        //
        // Declared wins; otherwise keep what is installed, because
        // reinstalling an undeclared package under a different architecture is
        // a change nobody asked for. `Arch::Keep` yields None, which means
        // "pass no -a".
        let arch: Option<String> = match declared.scoop.opts.get(name).and_then(|o| o.arch) {
            Some(a) => a.as_scoop().map(str::to_string),
            None => current.and_then(|c| c.arch.clone()),
        };
```

Thread `arch: arch.clone()` into the three pushes. Update `src/render.rs`'s
destructuring (the `..` patterns already absorb it; `ready_rest` and
`action_backend_name` need no change) and `src/apply.rs`'s `stage_and_fetch` to
capture and forward it:

```rust
    let (Action::Install { backend, name, arch, .. }
    | Action::Upgrade { backend, name, arch, .. }
    | Action::Downgrade { backend, name, arch, .. }) = action
    else { /* unchanged */ };
    ...
    let staged = scoop.stage(staging_root, name, pin).and_then(|manifest| {
        scoop.download(&manifest, arch.as_deref())?;
        Ok(manifest)
    });
```

Update every existing `Action::{Install,Upgrade,Downgrade}` literal in
`src/apply.rs`, `src/plan.rs`, `src/render.rs`, `tests/planner.rs`,
`tests/prepare.rs` to add `arch: None`.

- [ ] **Step 4: Run**

Run: `cargo test --all` — expected: 175 passed.

- [ ] **Step 5: Negative control**

Change the resolution to `let arch: Option<String> = None;`. Run
`cargo test --test planner the_architecture_an_install_will_use -- --exact`;
record that it fails on `declared wins`. Then change it to always use the
declared value with no fallback; record that it fails on the `stylua`
assertion. Both must be shown — the fallback is the half a single control
would miss. Restore.

- [ ] **Step 6: Commit**

```bash
git add src/plan.rs src/render.rs src/apply.rs tests/planner.rs tests/prepare.rs
git commit -m "Resolve the install architecture in the plan, not the executor

scoop download without -a fetches the default architecture's artifact --
measured, two different files for one version -- so an install that wants
the other one reaches the network from inside the mutation window.
Deciding it at plan time also puts it in front of the user before they
say yes. An undeclared package keeps the architecture it already has."
```

---

## Task 9: `Step`, `order`, and `run_step`

**Files:**
- Modify: `src/execute.rs`
- Create: `tests/execute.rs`

**Interfaces:**
- Produces:
  - `pub enum Step { Install { app, staged, arch }, Replace { app, staged, arch }, Remove { app } }`
  - `pub fn order(steps: Vec<Step>) -> Vec<Step>`
  - `pub const DEFER_LAST: &[&str]`
  - `pub enum StepOutcome { Done, Failed(String) }`
  - `pub fn run_step(root: &Path, m: &dyn Mutator, state: &mut State, step: &Step) -> StepOutcome`
- Consumes: `verify::{verdict, Expected, Disagreement}`, `State`.

- [ ] **Step 1: Write the failing tests**

Create `tests/execute.rs`. The fake is the centre of it:

```rust
//! The executor against real directory trees and a fake that lies exactly the
//! way scoop was measured to lie.

use dotpkg::execute::*;
use dotpkg::model::{Name, SCOOP};
use dotpkg::state::{Ownership, State};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

const BODY_A: &str = r#"{"version":"1.0.0","url":"https://good/v1.zip"}"#;

struct Tree(tempfile::TempDir);
impl Tree {
    fn new() -> Tree {
        Tree(tempfile::tempdir().unwrap())
    }
    fn root(&self) -> &Path {
        self.0.path()
    }
    fn stage(&self, app: &str, version: &str, body: &str) -> PathBuf {
        let d = self.root().join("stage").join(app).join(version);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("{app}.json"));
        std::fs::write(&p, body).unwrap();
        p
    }
    fn install(&self, app: &str, body: &str) {
        let cur = self.root().join("apps").join(app).join("current");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::write(cur.join("manifest.json"), body).unwrap();
    }
    fn half_install(&self, app: &str, version: &str) {
        let d = self.root().join("apps").join(app).join(version);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("thing.zip"), b"PK\x03\x04").unwrap();
    }
    fn empty_apps(&self) {
        std::fs::create_dir_all(self.root().join("apps")).unwrap();
    }
}

/// A fake scoop.
///
/// The two booleans exist because the executor's whole job is to disbelieve a
/// tool that reports success, and a fake that always tells the truth cannot
/// test that. Both default to scoop's measured worst behaviour: exit 0, having
/// done nothing. See `docs/specs/2026-08-08-phase2b2-executor-design.md`.
///
/// It deliberately does NOT serve its own idea of state back: observation goes
/// through the real filesystem, so a test cannot pass merely because the fake
/// is self-consistent.
struct Fake<'a> {
    tree: &'a Tree,
    uninstall_really_removes: bool,
    install_really_installs: bool,
    calls: RefCell<Vec<String>>,
}

impl<'a> Fake<'a> {
    fn honest(tree: &'a Tree) -> Fake<'a> {
        Fake { tree, uninstall_really_removes: true, install_really_installs: true, calls: RefCell::new(Vec::new()) }
    }
    fn silent_install(tree: &'a Tree) -> Fake<'a> {
        Fake { tree, uninstall_really_removes: true, install_really_installs: false, calls: RefCell::new(Vec::new()) }
    }
    fn silent_uninstall(tree: &'a Tree) -> Fake<'a> {
        Fake { tree, uninstall_really_removes: false, install_really_installs: true, calls: RefCell::new(Vec::new()) }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Mutator for Fake<'_> {
    fn uninstall(&self, app: &Name) -> anyhow::Result<CommandReport> {
        self.calls.borrow_mut().push(format!("uninstall {}", app.key()));
        if self.uninstall_really_removes {
            let _ = std::fs::remove_dir_all(self.tree.root().join("apps").join(app.key()));
        }
        Ok(CommandReport {
            code: Some(0),
            stdout: "ERROR 'x' isn't installed.\n".into(),
            stderr: String::new(),
        })
    }
    fn install(&self, manifest: &Path, _arch: Option<&str>) -> anyhow::Result<CommandReport> {
        self.calls.borrow_mut().push(format!("install {}", manifest.display()));
        if self.install_really_installs {
            let app = manifest.file_stem().unwrap().to_string_lossy().into_owned();
            let body = std::fs::read(manifest).unwrap();
            let cur = self.tree.root().join("apps").join(app).join("current");
            std::fs::create_dir_all(&cur).unwrap();
            std::fs::write(cur.join("manifest.json"), body).unwrap();
        }
        Ok(CommandReport {
            code: Some(0),
            stdout: "WARN  'fzf' (0.74.1) is already installed.\n".into(),
            stderr: String::new(),
        })
    }
}

#[test]
fn an_install_scoop_silently_did_not_perform_is_reported_and_not_claimed() {
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    let fake = Fake::silent_install(&t);
    let mut state = State::default();

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Install { app: Name::new("fzf"), staged, arch: Some("arm64".into()) },
    );

    let StepOutcome::Failed(why) = out else {
        panic!("a silent no-op must be a failure, got {out:?}")
    };
    assert!(why.contains("install did not happen"), "{why}");
    assert!(
        !state.owns(SCOOP, &Name::new("fzf")),
        "a failed install must not be recorded as owned"
    );
    // Not decoration: under a `verdict`-always-fails mutation the substring
    // assertion above stays green and only this one fires.
    assert_eq!(fake.calls().len(), 2, "an absent tree earns exactly one retry: {:?}", fake.calls());
}

#[test]
fn a_successful_install_is_claimed_exactly_once() {
    // The positive control for the test above. Without it, a run_step that
    // always fails passes that test.
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    let fake = Fake::honest(&t);
    let mut state = State::default();

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Install { app: Name::new("fzf"), staged, arch: None },
    );

    assert_eq!(out, StepOutcome::Done);
    assert_eq!(state.ownership(SCOOP, &Name::new("fzf")), Some(Ownership::Installed));
    assert_eq!(fake.calls().len(), 1, "a successful install is not retried");
}

#[test]
fn a_half_install_earns_no_retry() {
    // Retrying over a half-install gets `WARN ... is already installed`, exit
    // 0, and no change -- manufacturing the silent success verification exists
    // to catch. The retry gate is therefore on "nothing there at all", not on
    // "something went wrong".
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.half_install("fzf", "1.0.0");
    let fake = Fake::silent_install(&t);
    let mut state = State::default();

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Install { app: Name::new("fzf"), staged, arch: None },
    );

    assert!(matches!(out, StepOutcome::Failed(_)), "{out:?}");
    assert_eq!(fake.calls().len(), 1, "{:?}", fake.calls());
}

#[test]
fn a_replace_whose_uninstall_did_nothing_never_reaches_the_install() {
    let t = Tree::new();
    let staged = t.stage("fzf", "0.74.2", r#"{"version":"0.74.2"}"#);
    t.install("fzf", r#"{"version":"0.74.1"}"#);
    let fake = Fake::silent_uninstall(&t);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("fzf"), Ownership::Adopted);

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Replace { app: Name::new("fzf"), staged, arch: None },
    );

    let StepOutcome::Failed(why) = out else { panic!("got {out:?}") };
    assert!(why.contains("uninstall did not happen"), "{why}");
    assert_eq!(
        fake.calls(),
        vec!["uninstall fzf".to_string()],
        "install must not run after an unverified uninstall"
    );
    assert_eq!(
        state.ownership(SCOOP, &Name::new("fzf")),
        Some(Ownership::Adopted),
        "a failed replace changes no ownership, and must not downgrade adopt to installed"
    );
}

#[test]
fn a_successful_replace_of_an_adopted_package_keeps_it_adopted() {
    let t = Tree::new();
    let staged = t.stage("fzf", "0.74.2", r#"{"version":"0.74.2"}"#);
    t.install("fzf", r#"{"version":"0.74.1"}"#);
    let fake = Fake::honest(&t);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("fzf"), Ownership::Adopted);

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Replace { app: Name::new("fzf"), staged, arch: None },
    );

    assert_eq!(out, StepOutcome::Done);
    assert_eq!(
        state.ownership(SCOOP, &Name::new("fzf")),
        Some(Ownership::Adopted),
        "upgrading an adopted package must not silently reclassify it as installed"
    );
}

#[test]
fn a_prune_releases_ownership_only_after_the_disk_agrees() {
    let t = Tree::new();
    t.install("aichat", BODY_A);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);

    let liar = Fake::silent_uninstall(&t);
    let out = run_step(t.root(), &liar, &mut state, &Step::Remove { app: Name::new("aichat") });
    assert!(matches!(out, StepOutcome::Failed(_)), "{out:?}");
    assert!(
        state.owns(SCOOP, &Name::new("aichat")),
        "a package still on disk must still be owned -- releasing here leaves it \
         installed and unmanageable, and `dotpkg adopt` does not exist"
    );

    let honest = Fake::honest(&t);
    let out = run_step(t.root(), &honest, &mut state, &Step::Remove { app: Name::new("aichat") });
    assert_eq!(out, StepOutcome::Done);
    assert_eq!(state.owned_count(SCOOP), 0);
}

#[test]
fn installs_precede_replacements_precede_removals_and_git_goes_last() {
    let s = |n: &str| PathBuf::from(format!("/stage/{n}.json"));
    let steps = vec![
        Step::Replace { app: Name::new("git"), staged: s("git"), arch: None },
        Step::Remove { app: Name::new("aichat") },
        Step::Replace { app: Name::new("bat"), staged: s("bat"), arch: None },
        Step::Install { app: Name::new("ripgrep"), staged: s("ripgrep"), arch: None },
        Step::Replace { app: Name::new("7zip"), staged: s("7zip"), arch: None },
    ];
    let got: Vec<String> = order(steps).iter().map(|s| s.app().key().to_string()).collect();
    assert_eq!(
        got,
        vec!["ripgrep", "bat", "7zip", "git", "aichat"],
        "a pure install opens no window at all and goes first; git is the binary \
         staging needs and is itself scoop-managed, so it goes last in its group"
    );
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo test --test execute`
Expected: FAIL to compile — `cannot find type `Step` in module `dotpkg::execute``.

- [ ] **Step 3: Implement**

Add to `src/execute.rs`:

```rust
use crate::model::SCOOP;
use crate::state::{Ownership, State};
use crate::verify::{verdict, Disagreement, Expected};
use std::path::PathBuf;

/// One mutation, already resolved against the plan and the preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Nothing is installed: no window opens at all.
    Install { app: Name, staged: PathBuf, arch: Option<String> },
    /// A version change, which scoop can only do as uninstall + install.
    Replace { app: Name, staged: PathBuf, arch: Option<String> },
    Remove { app: Name },
}

impl Step {
    pub fn app(&self) -> &Name {
        match self {
            Step::Install { app, .. } | Step::Replace { app, .. } | Step::Remove { app } => app,
        }
    }
}

/// Packages held back to the end of their group.
///
/// `git` is the binary `Scoop::stage` shells out to, and on the dogfood machine
/// it is itself scoop-managed (`where.exe git` resolves into
/// `scoop\apps\git\current`). The extraction helpers are what scoop uses to
/// unpack everything else.
pub const DEFER_LAST: &[&str] = &["git", "7zip", "dark", "innounp", "lessmsi"];

/// Installs, then replacements, then removals; `DEFER_LAST` at the end of each
/// group; alphabetical within that, so a run is reproducible.
pub fn order(mut steps: Vec<Step>) -> Vec<Step> {
    steps.sort_by_key(|s| {
        let group = match s {
            Step::Install { .. } => 0u8,
            Step::Replace { .. } => 1,
            Step::Remove { .. } => 2,
        };
        let deferred = u8::from(DEFER_LAST.contains(&s.app().key()));
        (group, deferred, s.app().key().to_string())
    });
    steps
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Done,
    Failed(String),
}

/// Perform one step and prove on disk that it happened.
///
/// State is written only after the disk agrees, and only when the answer
/// changes: an upgrade of a package dotpkg already owns writes nothing,
/// because ownership is intent and the uninstall half is an implementation
/// detail. A crash mid-window leaves the package absent and still declared,
/// and the next run's plan re-emits an `Install`.
pub fn run_step(root: &Path, m: &dyn Mutator, state: &mut State, step: &Step) -> StepOutcome {
    match step {
        Step::Install { app, staged, arch } | Step::Replace { app, staged, arch } => {
            if matches!(step, Step::Replace { .. }) {
                if let Err(e) = m.uninstall(app) {
                    return StepOutcome::Failed(format!("{app}: could not run uninstall: {e:#}"));
                }
                if let Err(d) = verdict(root, app, &Expected::Absent) {
                    return StepOutcome::Failed(format!("{app}: uninstall did not happen -- {d}"));
                }
            }
            if let Err(e) = m.install(staged, arch.as_deref()) {
                return StepOutcome::Failed(format!("{app}: could not run install: {e:#}"));
            }
            let want = Expected::Present { staged: staged.clone() };
            if let Err(d) = verdict(root, app, &want) {
                // Retry exactly once, and only when there is nothing there at
                // all. A retry over a half-install gets `WARN ... is already
                // installed`, exit 0, and no change -- which would then pass
                // no check dotpkg has.
                if d != Disagreement::NotInstalled {
                    return StepOutcome::Failed(format!("{app}: install did not happen -- {d}"));
                }
                if let Err(e) = m.install(staged, arch.as_deref()) {
                    return StepOutcome::Failed(format!("{app}: could not run retry: {e:#}"));
                }
                if let Err(d2) = verdict(root, app, &want) {
                    return StepOutcome::Failed(format!(
                        "{app}: install did not happen, even on retry -- {d2}"
                    ));
                }
            }
            // Claim only now, and preserve an existing `adopt`.
            if state.ownership(SCOOP, app).is_none() {
                state.set(SCOOP, app, Ownership::Installed);
            }
            StepOutcome::Done
        }
        Step::Remove { app } => {
            if let Err(e) = m.uninstall(app) {
                return StepOutcome::Failed(format!("{app}: could not run uninstall: {e:#}"));
            }
            if let Err(d) = verdict(root, app, &Expected::Absent) {
                return StepOutcome::Failed(format!("{app}: uninstall did not happen -- {d}"));
            }
            state.remove(SCOOP, app);
            StepOutcome::Done
        }
    }
}
```

- [ ] **Step 4: Run**

Run: `cargo test --test execute` — expected: 7 passed.
Run: `cargo test --all` — expected: 182 passed.

- [ ] **Step 5: Negative controls — four, each run separately**

Record the exact failing assertion for each.

1. `verdict` always `Ok(())` — expect
   `an_install_scoop_silently_did_not_perform_is_reported_and_not_claimed` and
   `a_prune_releases_ownership_only_after_the_disk_agrees` red.
2. `verdict` always `Err(Disagreement::ContentDiffers)` — **the trap**. Note in
   the record that `why.contains("install did not happen")` stays **green**
   under this mutation, and that what fires is the call-count assertion plus
   `a_successful_install_is_claimed_exactly_once`. If those two are ever
   deleted, this control stops catching anything.
3. Delete only the post-install `verdict` call — expect the two install tests
   red and the two uninstall tests green.
4. Delete only the two post-uninstall `verdict` calls — expect the two
   uninstall tests red and the install tests green.
5. Change the retry gate from `d != Disagreement::NotInstalled` to `false` —
   expect `a_half_install_earns_no_retry` red on the call count.
6. Change `if state.ownership(..).is_none()` to an unconditional
   `state.set(.., Ownership::Installed)` — expect
   `a_successful_replace_of_an_adopted_package_keeps_it_adopted` red.

- [ ] **Step 6: Commit**

```bash
git add src/execute.rs tests/execute.rs
git commit -m "Add the executor's step loop, ordered and verified

Every mutation is followed by a disk check, because scoop's exit code
carries no information. Retry is gated on 'nothing there at all': a retry
over a half-install gets WARN/exit 0 and would manufacture the exact
silent success the verification exists to catch. Ownership is claimed
after the disk agrees and released after it agrees, and an upgrade of an
adopted package stays adopted."
```

---

## Task 10: `execute` — the whole run, the recovery file, the held prunes

**Files:**
- Modify: `src/execute.rs`, `src/apply.rs`
- Test: `tests/execute.rs`

**Interfaces:**
- Produces:
  - `pub struct ExecOptions { pub recovery_path: Option<PathBuf> }` — note
    there is no `keep_going` here; see Step 3 for why.
  - `pub enum ItemResult { Done, Failed(String), Held(String) }`
  - `pub struct Execution { pub results: Vec<(Name, ItemResult)>, pub dropped_ghosts: Vec<Name> }`
  - `impl Execution { pub fn changed(&self) -> usize; pub fn failed(&self) -> usize; pub fn held(&self) -> usize; pub fn exit_code(&self, refused: bool) -> i32 }`
  - `pub fn execute(root, plan_steps, m, state, running, opts) -> Execution`
  - `pub fn plan_to_steps(prep: &Preparation) -> (Vec<Step>, Vec<(Name, String)>)` in `src/apply.rs`
  - `pub fn write_recovery(path: &Path, steps: &[Step]) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing tests**

Add to `tests/execute.rs`:

```rust
#[test]
fn one_packages_failure_does_not_stop_its_neighbours() {
    let t = Tree::new();
    let good = t.stage("bat", "1.0.0", BODY_A);
    let bad = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    // A fake that installs everything except fzf.
    struct Picky<'a> { tree: &'a Tree, calls: RefCell<Vec<String>> }
    impl Mutator for Picky<'_> {
        fn uninstall(&self, app: &Name) -> anyhow::Result<CommandReport> {
            self.calls.borrow_mut().push(format!("uninstall {}", app.key()));
            let _ = std::fs::remove_dir_all(self.tree.root().join("apps").join(app.key()));
            Ok(CommandReport { code: Some(0), stdout: String::new(), stderr: String::new() })
        }
        fn install(&self, manifest: &Path, _a: Option<&str>) -> anyhow::Result<CommandReport> {
            let app = manifest.file_stem().unwrap().to_string_lossy().into_owned();
            self.calls.borrow_mut().push(format!("install {app}"));
            if app != "fzf" {
                let cur = self.tree.root().join("apps").join(&app).join("current");
                std::fs::create_dir_all(&cur).unwrap();
                std::fs::write(cur.join("manifest.json"), std::fs::read(manifest).unwrap()).unwrap();
            }
            Ok(CommandReport { code: Some(0), stdout: String::new(), stderr: String::new() })
        }
    }
    let fake = Picky { tree: &t, calls: RefCell::new(Vec::new()) };
    let mut state = State::default();

    let ex = execute(
        t.root(),
        vec![
            Step::Install { app: Name::new("fzf"), staged: bad, arch: None },
            Step::Install { app: Name::new("bat"), staged: good, arch: None },
        ],
        &fake,
        &mut state,
        &Running::default(),
        &ExecOptions::default(),
    );

    assert_eq!(ex.failed(), 1);
    assert_eq!(ex.changed(), 1, "bat must still be installed: {:?}", ex.results);
    assert!(state.owns(SCOOP, &Name::new("bat")));
    assert!(!state.owns(SCOOP, &Name::new("fzf")));
}

#[test]
fn a_package_that_started_running_between_the_plan_and_the_mutation_is_skipped() {
    // `running` is sampled once, before roughly two dozen downloads. A user
    // who opens their editor during the prefetch must not have it uninstalled.
    let t = Tree::new();
    t.install("nvim-ish", BODY_A);
    let fake = Fake::honest(&t);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("nvim-ish"), Ownership::Installed);
    let running = Running::new(
        std::collections::BTreeSet::from(["nvim-ish".to_string()]),
        Default::default(),
    );

    let ex = execute(
        t.root(),
        vec![Step::Remove { app: Name::new("nvim-ish") }],
        &fake,
        &mut state,
        &running,
        &ExecOptions::default(),
    );

    assert_eq!(ex.held(), 1, "{:?}", ex.results);
    assert_eq!(fake.calls(), Vec::<String>::new(), "nothing may be run for it");
    assert!(state.owns(SCOOP, &Name::new("nvim-ish")), "and it stays owned");
}

#[test]
fn the_recovery_file_is_written_before_anything_is_mutated_and_names_every_artifact() {
    let t = Tree::new();
    let a = t.stage("bat", "1.0.0", BODY_A);
    let b = t.stage("fzf", "0.74.1", BODY_A);
    let out = t.root().join("recover.cmd");
    write_recovery(
        &out,
        &[
            Step::Replace { app: Name::new("bat"), staged: a.clone(), arch: Some("arm64".into()) },
            Step::Install { app: Name::new("fzf"), staged: b.clone(), arch: None },
            Step::Remove { app: Name::new("aichat") },
        ],
    )
    .unwrap();

    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains(&a.display().to_string()), "{text}");
    assert!(text.contains(&b.display().to_string()), "{text}");
    assert!(text.contains("-a arm64"), "the architecture must be in the recovery line: {text}");
    assert!(
        !text.contains("aichat"),
        "a removal has nothing to restore and must not appear: {text}"
    );
    assert!(
        !text.contains("uninstall"),
        "the recovery file only ever puts software back: {text}"
    );
}

#[test]
fn a_run_that_changed_nothing_and_a_run_that_changed_something_exit_differently() {
    let clean = Execution::default();
    assert_eq!(clean.exit_code(false), 0);
    assert_eq!(clean.exit_code(true), 2, "refused before starting");

    let mixed = Execution {
        results: vec![
            (Name::new("a"), ItemResult::Done),
            (Name::new("b"), ItemResult::Failed("no".into())),
        ],
        dropped_ghosts: Vec::new(),
    };
    assert_eq!(mixed.exit_code(false), 1, "something changed and something failed");
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo test --test execute`
Expected: FAIL to compile — `cannot find function `execute``.

- [ ] **Step 3: Implement**

Add to `src/execute.rs`:

```rust
use crate::model::Running;

#[derive(Debug, Default, Clone)]
pub struct ExecOptions {
    /// Where to write the recovery script before the first mutation.
    pub recovery_path: Option<PathBuf>,
}
```

**`keep_going` is deliberately NOT a field here.** It decides which steps get
built, and `main.rs` has already applied it by the time `execute` is called —
carrying it in would be a flag the function receives and never reads, which is
both dead and misleading about where the decision lives. If a later change
needs `execute` itself to branch on it, add it then, with the branch.

```rust

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemResult {
    Done,
    Failed(String),
    /// Not attempted, and the run is not at fault: the package started
    /// running, or removals are gated off.
    Held(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Execution {
    pub results: Vec<(Name, ItemResult)>,
    pub dropped_ghosts: Vec<Name>,
}

impl Execution {
    pub fn changed(&self) -> usize {
        self.results.iter().filter(|(_, r)| *r == ItemResult::Done).count()
    }
    pub fn failed(&self) -> usize {
        self.results.iter().filter(|(_, r)| matches!(r, ItemResult::Failed(_))).count()
    }
    pub fn held(&self) -> usize {
        self.results.iter().filter(|(_, r)| matches!(r, ItemResult::Held(_))).count()
    }
    /// 0 everything verified · 1 something changed and something failed ·
    /// 2 refused, nothing changed.
    ///
    /// The distinction 2 buys is the one a caller most needs: "go look at the
    /// machine" versus "nothing to look at".
    pub fn exit_code(&self, refused: bool) -> i32 {
        if refused {
            return 2;
        }
        if self.failed() > 0 {
            if self.changed() == 0 {
                return 2;
            }
            return 1;
        }
        0
    }
}

/// One `scoop install` line per artifact in the run, written **before** the
/// first mutation.
///
/// A run that dies leaves a file that puts the machine back. A run that only
/// prints advice leaves nothing once the terminal is gone — and the terminal
/// is exactly what a broken `git` or a broken shell takes with it.
///
/// Removals never appear: this file only ever puts software back.
pub fn write_recovery(path: &Path, steps: &[Step]) -> Result<()> {
    use std::fmt::Write as _;
    let mut text = String::from(
        "@echo off\r\nREM Written by dotpkg before it changed anything.\r\n\
         REM Each line reinstalls one package from the manifest dotpkg staged\r\n\
         REM and hash-verified. Safe to run more than once.\r\n",
    );
    for s in steps {
        let (staged, arch) = match s {
            Step::Install { staged, arch, .. } | Step::Replace { staged, arch, .. } => (staged, arch),
            Step::Remove { .. } => continue,
        };
        let a = match arch {
            Some(a) => format!("-a {a} "),
            None => String::new(),
        };
        let _ = writeln!(text, "scoop install -u {a}\"{}\"\r", staged.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
}

/// Run every step, in order, verifying each. One package's failure never
/// stops another's.
pub fn execute(
    root: &Path,
    steps: Vec<Step>,
    m: &dyn Mutator,
    state: &mut State,
    running: &Running,
    opts: &ExecOptions,
) -> Execution {
    let steps = order(steps);
    let mut ex = Execution::default();

    if let Some(p) = &opts.recovery_path {
        if let Err(e) = write_recovery(p, &steps) {
            eprintln!("warning: could not write the recovery script: {e:#}");
        }
    }

    for step in &steps {
        let app = step.app().clone();
        // Re-checked here, not only at plan time: `running` was sampled before
        // the prefetch, which takes minutes.
        if running.covers_name(&app) {
            ex.results.push((
                app,
                ItemResult::Held("started running since the plan was made -- stop it and run again".into()),
            ));
            continue;
        }
        let r = match run_step(root, m, state, step) {
            StepOutcome::Done => ItemResult::Done,
            StepOutcome::Failed(why) => ItemResult::Failed(why),
        };
        ex.results.push((app, r));
    }
    ex
}
```

Add to `src/model.rs`'s `impl Running`, since the executor has a `Name` and not
an `Installed`:

```rust
    /// The name-and-directory halves of `covers`, for a caller that has only a
    /// package name. The `bins` half cannot be consulted here, so this is
    /// strictly weaker: use `covers` wherever an `Installed` is available.
    pub fn covers_name(&self, name: &Name) -> bool {
        self.dirs.contains(name) || self.names.contains(name.key())
    }
```

Add to `src/apply.rs`:

```rust
/// Turn a finished `Preparation` into the steps the executor will run, plus
/// the packages that could not become steps and why.
pub fn plan_to_steps(prep: &Preparation) -> (Vec<Step>, Vec<(Name, String)>) {
    let mut steps = Vec::new();
    let mut unusable = Vec::new();
    for p in &prep.prepared {
        // Branch on the ACTION, never on the outcome: `Outcome::ReadyToRemove`
        // is still attachable to an `Install`, and nothing in the type system
        // binds the two.
        match (&p.action, &p.outcome) {
            (Action::Install { name, arch, .. }, Outcome::ReadyToFetch { manifest }) => {
                steps.push(Step::Install {
                    app: name.clone(),
                    staged: manifest.clone(),
                    arch: arch.clone(),
                })
            }
            (
                Action::Upgrade { name, arch, .. } | Action::Downgrade { name, arch, .. },
                Outcome::ReadyToFetch { manifest },
            ) => steps.push(Step::Replace {
                app: name.clone(),
                staged: manifest.clone(),
                arch: arch.clone(),
            }),
            (Action::Prune { name, .. }, Outcome::ReadyToRemove) => {
                steps.push(Step::Remove { app: name.clone() })
            }
            (a, Outcome::Failed { why }) => unusable.push((action_name(a), why.clone())),
            (a, Outcome::NotLocked) => unusable.push((
                action_name(a),
                "no lock entry -- run `dotpkg update`".to_string(),
            )),
            _ => {}
        }
    }
    (steps, unusable)
}
```

with a small `action_name(&Action) -> Name` helper mirroring
`render::action_backend_name`.

- [ ] **Step 4: Run**

Run: `cargo test --all` — expected: 186 passed.

- [ ] **Step 5: Negative controls**

1. Delete the `running.covers_name` check in `execute`. Record that
   `a_package_that_started_running_between_the_plan_and_the_mutation_is_skipped`
   fails on `ex.held()`. Restore.
2. Make `execute` return after the first `Failed`. Record that
   `one_packages_failure_does_not_stop_its_neighbours` fails on
   `bat must still be installed`. Restore.
3. Include `Step::Remove` in `write_recovery`. Record that
   `the_recovery_file_is_written_before_anything_is_mutated_and_names_every_artifact`
   fails on the `aichat` assertion. Restore.

- [ ] **Step 6: Commit**

```bash
git add src/execute.rs src/apply.rs src/model.rs tests/execute.rs
git commit -m "Add the whole-run executor, the recovery file, and the running re-check

The recovery script is written before the first mutation rather than
printed after a failure: a run that dies leaves a file that puts the
machine back, where advice on a lost terminal leaves nothing. `running`
is re-checked immediately before each mutation because it was sampled
once, before a prefetch that takes minutes."
```

---

## Task 11: `pkg.toml` buckets become data, and missing ones can be cloned

**Files:**
- Modify: `src/config.rs`, `src/backend/scoop.rs`, `src/apply.rs`
- Test: `src/config.rs`'s `mod tests`, `tests/prepare.rs`

**Interfaces:**
- Produces:
  - `pub struct BucketDecl { pub name: Name, pub url: Option<String> }`
  - `ScoopSection.buckets: Vec<BucketDecl>` (was `Vec<String>`)
  - `pub fn bucket_add_argv(b: &BucketDecl) -> Vec<String>` in `src/backend/scoop.rs`
  - `pub fn clone_missing_buckets(scoop: &Scoop, declared: &Config) -> Vec<(Name, String)>`
- Consumes: `ensure_plain_component`.

- [ ] **Step 1: Write the failing tests**

Add to `src/config.rs`'s `mod tests`:

```rust
    #[test]
    fn a_bucket_declaration_splits_into_a_name_and_an_optional_url() {
        let cfg = parse(
            "[scoop]\nbuckets = [\"main\", \"extras\", \
             \"xom11=https://github.com/xom11/scoop-bucket\"]\n",
        )
        .unwrap();
        let b = &cfg.scoop.buckets;
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].name, Name::new("main"));
        assert_eq!(b[0].url, None);
        assert_eq!(b[2].name, Name::new("xom11"));
        assert_eq!(
            b[2].url.as_deref(),
            Some("https://github.com/xom11/scoop-bucket")
        );
    }

    #[test]
    fn a_bucket_name_that_could_leave_its_directory_is_refused_at_parse_time() {
        for bad in ["../evil", "a/b", "-oops", "", "c:\\x"] {
            let text = format!("[scoop]\nbuckets = [\"{bad}=https://example.invalid/x\"]\n");
            assert!(parse(&text).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_bucket_url_must_look_like_a_url() {
        assert!(parse("[scoop]\nbuckets = [\"x=not a url\"]\n").is_err());
        assert!(parse("[scoop]\nbuckets = [\"x=https://example.invalid/b\"]\n").is_ok());
        assert!(parse("[scoop]\nbuckets = [\"x=git@example.invalid:b.git\"]\n").is_ok());
    }

    #[test]
    fn two_bucket_declarations_naming_the_same_bucket_are_refused() {
        let err = parse("[scoop]\nbuckets = [\"main\", \"MAIN=https://x.invalid/y\"]\n")
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("main") && msg.contains("MAIN"), "{msg}");
    }
```

Add to `tests/prepare.rs`:

```rust
#[test]
fn cloning_is_only_offered_for_a_bucket_pkg_toml_declares_with_a_url() {
    // Never a guessed URL: a lock naming an undeclared bucket is a failure
    // that says so.
    let cfg = dotpkg::config::parse(
        "[scoop]\nbuckets = [\"main\", \"xom11=https://example.invalid/b\"]\n",
    )
    .unwrap();
    let argvs: Vec<Vec<String>> = cfg
        .scoop
        .buckets
        .iter()
        .map(dotpkg::backend::scoop::bucket_add_argv)
        .collect();
    assert_eq!(argvs[0], vec!["bucket", "add", "main"]);
    assert_eq!(
        argvs[1],
        vec!["bucket", "add", "xom11", "https://example.invalid/b"]
    );
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --lib config`
Expected: FAIL to compile — `no field `name` on type `&String``.

- [ ] **Step 3: Implement**

In `src/config.rs`:

```rust
/// One entry of `[scoop] buckets`.
///
/// `"main"` names a bucket scoop already knows; `"xom11=https://…"` names one
/// it does not and says where to get it. Until Phase 2b-2 this list was parsed
/// into `Vec<String>` and read by nothing, while the approved design described
/// cloning from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketDecl {
    pub name: Name,
    pub url: Option<String>,
}

fn parse_buckets(raw: Vec<String>) -> Result<Vec<BucketDecl>> {
    let mut seen: BTreeMap<Name, String> = BTreeMap::new();
    let mut out = Vec::with_capacity(raw.len());
    for entry in raw {
        let (name_str, url) = match entry.split_once('=') {
            Some((n, u)) => (n.to_string(), Some(u.to_string())),
            None => (entry.clone(), None),
        };
        let name = Name::new(name_str.clone());
        // The bucket name becomes `$SCOOP/buckets/<name>` and a git argument.
        crate::backend::scoop::ensure_plain_component(&name, "bucket name", name.key())?;
        if let Some(u) = &url {
            anyhow::ensure!(
                u.starts_with("https://") || u.starts_with("http://") || u.contains('@'),
                "[scoop] buckets: {u:?} does not look like a git remote"
            );
        }
        if let Some(first) = seen.get(&name) {
            anyhow::bail!(
                "[scoop] buckets names the same bucket twice: {first:?} and {name_str:?} \
                 (bucket names are compared without regard to case)"
            );
        }
        seen.insert(name.clone(), name_str);
        out.push(BucketDecl { name, url });
    }
    Ok(out)
}
```

Change `ScoopSection.buckets` to `Vec<BucketDecl>` and call `parse_buckets` in
`parse`. Update `lock_coherence_guard`'s bucket set to
`declared.scoop.buckets.iter().map(|b| &b.name)` and compare with
`Name::new(bucket)`.

In `src/backend/scoop.rs`:

```rust
/// The argv for adding a bucket. A declaration with no URL names a bucket
/// scoop already knows by name (`main`, `extras`).
pub fn bucket_add_argv(b: &crate::config::BucketDecl) -> Vec<String> {
    let mut argv = vec!["bucket".to_string(), "add".to_string(), b.name.key().to_string()];
    if let Some(u) = &b.url {
        argv.push(u.clone());
    }
    argv
}

impl Scoop {
    /// Clone every declared bucket that is not on disk. Returns one entry per
    /// bucket that is still missing afterwards.
    ///
    /// Verified by looking for `.git` again, not by the exit code: measured,
    /// `scoop bucket add` exits 0 on a duplicate and on a failure alike.
    pub fn clone_missing_buckets(&self, declared: &crate::config::Config) -> Vec<(Name, String)> {
        let mut failed = Vec::new();
        for b in &declared.scoop.buckets {
            let dir = self.root.join("buckets").join(b.name.key());
            if dir.join(".git").exists() {
                continue;
            }
            let argv = bucket_add_argv(b);
            match self.run(&argv) {
                Ok(_) if dir.join(".git").exists() => {}
                Ok(r) => failed.push((b.name.clone(), tail(&r.stdout))),
                Err(e) => failed.push((b.name.clone(), format!("{e:#}"))),
            }
        }
        failed
    }
}
```

- [ ] **Step 4: Run**

Run: `cargo test --all` — expected: 191 passed.

- [ ] **Step 5: Negative control**

Remove the `ensure_plain_component` call from `parse_buckets`. Record that
`a_bucket_name_that_could_leave_its_directory_is_refused_at_parse_time` fails
for `"../evil"`. Restore.

- [ ] **Step 6: Commit**

```bash
git add src/config.rs src/backend/scoop.rs src/apply.rs tests/prepare.rs
git commit -m "Parse pkg.toml's buckets into data and let apply clone a missing one

design.md said 'offer to clone (URL is in pkg.toml)'; the list was parsed
into Vec<String> and read by nothing outside its own test. A clone is
verified by looking for .git again, because scoop bucket add exits 0 on a
duplicate."
```

---

## Task 12: The driver, the prompt, the flags, the exit codes

**Files:**
- Modify: `src/main.rs`, `src/render.rs`, `src/apply.rs`
- Test: `src/render.rs`'s `mod tests`, `tests/cli.rs`

**Interfaces:**
- Produces:
  - `pub struct Driver { pub declared: Config, pub locked: Lock, pub state: State, pub scoop: Scoop, pub scan: Scan, pub running: Running }`
  - `pub fn load_everything(config: &Path, lock: &Path, state: &Path) -> anyhow::Result<Driver>`
  - `pub fn confirm(question: &str, input: &mut dyn std::io::BufRead, err: &mut dyn std::io::Write) -> anyhow::Result<bool>`
  - `pub fn render_execution(ex: &Execution) -> String` in `src/render.rs`
- New CLI flags: `--yes`, `--allow-prune`, `--keep-going`, `--state <path>`.

- [ ] **Step 1: Write the failing tests**

Add to `src/apply.rs`'s `mod tests`:

```rust
    #[test]
    fn no_answer_at_all_means_no_and_says_which_flag_would_have_helped() {
        // A scheduled task with no console gives a child process an immediately
        // closed stdin: read_line returns Ok(0). That is the exact shape the
        // a14 dogfood runs under, and it must never read as consent.
        let mut empty: &[u8] = b"";
        let mut err = Vec::new();
        let answered = confirm("Continue? [y/N] ", &mut empty, &mut err).unwrap();
        assert!(!answered, "an empty stdin must not be a yes");
        let text = String::from_utf8(err).unwrap();
        assert!(text.contains("--yes"), "say what to pass instead: {text}");
    }

    #[test]
    fn only_an_explicit_yes_is_a_yes() {
        for (input, expected) in [
            ("y\n", true),
            ("Y\n", true),
            ("yes\n", true),
            ("\n", false),
            ("n\n", false),
            ("no\n", false),
            ("Yes please\n", false),
            ("  y  \n", true),
        ] {
            let mut r = input.as_bytes();
            let mut err = Vec::new();
            assert_eq!(
                confirm("q", &mut r, &mut err).unwrap(),
                expected,
                "input {input:?}"
            );
        }
    }

    #[test]
    fn the_question_goes_to_stderr_so_a_piped_run_still_shows_it() {
        let mut r: &[u8] = b"y\n";
        let mut err = Vec::new();
        confirm("Continue? [y/N] ", &mut r, &mut err).unwrap();
        assert!(String::from_utf8(err).unwrap().contains("Continue?"));
    }
```

Add to `src/render.rs`'s `mod tests`:

```rust
    #[test]
    fn the_summary_never_claims_more_than_the_run_verified() {
        let ex = Execution {
            results: vec![
                (Name::new("bat"), ItemResult::Done),
                (Name::new("fzf"), ItemResult::Failed("install did not happen".into())),
                (Name::new("kanata"), ItemResult::Held("started running".into())),
            ],
            dropped_ghosts: vec![Name::new("stale")],
        };
        let out = render_execution(&ex);
        assert!(out.contains("1 verified on disk"), "{out}");
        assert!(out.contains("1 failed"), "{out}");
        assert!(out.contains("1 held"), "{out}");
        assert!(out.contains("fzf"), "name what failed: {out}");
        assert!(out.contains("stale"), "say which ownership records were dropped: {out}");
        assert!(
            !out.contains("Nothing has been changed."),
            "that promise belongs to --prepare and must not appear after a mutation: {out}"
        );
    }
```

Add to `tests/cli.rs`. **Read this constraint first, because it decides every
fixture below:** on macOS there is no scoop binary, so any action needing an
artifact fails at `download` and makes `preparation.is_ok()` false. A run that
reaches the prompt at all must therefore have a plan containing **only prunes
and reports**. Any fixture with an `Install` in it exits at the
"could not be prepared" branch instead — which is a different code path, and a
test that does not know which one it exercised proves nothing.

First, two fixture helpers:

```rust
impl Fixture {
    /// A real one-commit git bucket plus a `pkg.lock` naming its real SHA, so
    /// a run can get past `lock_coherence_guard` and reach staging.
    fn write_lock_and_bucket_for(&self, app: &str, version: &str) {
        let dir = self.scoop.path().join("buckets").join("main");
        fs::create_dir_all(dir.join("bucket")).unwrap();
        let git = |args: &[&str]| -> String {
            let out = Command::new("git")
                .current_dir(&dir)
                .args(args)
                .output()
                .expect("git must be on PATH for this test");
            assert!(out.status.success(), "git {args:?}: {}", text(&out.stderr));
            text(&out.stdout)
        };
        git(&["init", "-q", "-b", "main"]);
        fs::write(
            dir.join("bucket").join(format!("{app}.json")),
            format!(r#"{{"version":"{version}","bin":"tool.exe"}}"#),
        )
        .unwrap();
        git(&["add", "-A"]);
        git(&[
            "-c", "user.email=t@example.invalid", "-c", "user.name=t",
            "commit", "-q", "-m", "x",
        ]);
        let sha = git(&["rev-parse", "HEAD"]).trim().to_string();
        fs::write(
            self.work.path().join("pkg.lock"),
            format!("[scoop.{app}]\nbucket = \"main\"\ncommit = \"{sha}\"\nversion = \"{version}\"\n"),
        )
        .unwrap();
    }

    /// An installed app in the shape `Scoop::scan` reads.
    fn install_app(&self, app: &str, version: &str) {
        let cur = self.scoop.path().join("apps").join(app).join("current");
        fs::create_dir_all(&cur).unwrap();
        fs::write(
            cur.join("manifest.json"),
            format!(r#"{{"version":"{version}"}}"#),
        )
        .unwrap();
        fs::write(
            cur.join("install.json"),
            r#"{"bucket":"main","architecture":"arm64"}"#,
        )
        .unwrap();
    }
}
```

Then the tests:

```rust
#[test]
fn apply_with_no_answer_available_refuses_and_changes_nothing() {
    // The fixture is built so the plan is exactly one PRUNE and nothing else:
    // fzf is declared, locked, and already at the locked version, so it
    // produces no action; aichat is owned, installed and undeclared. That is
    // the only shape whose preparation is `ok` on a machine with no scoop, and
    // therefore the only shape that reaches the prompt.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"fzf":"installed","aichat":"adopted"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");
    f.install_app("aichat", "0.30.0");
    let before = f.snapshot();

    // `Command::output` gives the child an immediately closed stdin, which is
    // exactly what the medium-integrity scheduled task on a14 produces.
    let out = f.run(&["apply"]);
    let stderr = text(&out.stderr);
    let stdout = text(&out.stdout);

    assert_eq!(
        out.status.code(),
        Some(2),
        "a refused run exits 2; stdout: {stdout} stderr: {stderr}"
    );
    assert!(stderr.contains("--yes"), "say what to pass instead: {stderr}");
    assert!(
        !stderr.contains("scoop.cmd") && !stdout.contains("scoop.cmd"),
        "it must refuse before running scoop at all: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn the_scoop_cmd_sentinel_is_reachable_or_the_assertion_above_proves_nothing() {
    // Production prints `cannot run <root>/shims/scoop.cmd` only once it has
    // actually tried to run scoop. Without this sibling, the negative
    // assertion above stays green even if the whole executor is deleted.
    //
    // Here fzf is declared and locked at 1.0.0 but NOT installed, so the plan
    // is an Install, prepare stages it for real against real git, and the
    // download is what fails.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        "{}",
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");

    let out = f.run(&["apply", "--prepare"]);
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(all.contains("scoop.cmd"), "the sentinel must be reachable: {all}");
}

#[test]
fn yes_alone_does_not_authorise_a_prune() {
    // Same one-prune fixture as the first test, so this exercises the
    // --allow-prune gate and not the not-ok-preparation gate.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"fzf":"installed","aichat":"adopted"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");
    f.install_app("aichat", "0.30.0");
    let before = f.snapshot();

    let out = f.run(&["apply", "--yes"]);
    let stderr = text(&out.stderr);

    assert_ne!(out.status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("--allow-prune"), "{stderr}");
    assert!(
        !stderr.contains("could not be prepared"),
        "this must fail on the prune gate, not on an unprepared package: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn a_prune_authorised_by_both_flags_runs_and_records_the_release() {
    // The positive control for the two tests above: without it, a main.rs that
    // refuses every run passes both of them. This is also the only test in the
    // suite that drives a real mutation end to end, and it can only be a prune
    // -- a prune is the one step that needs no scoop binary to be *planned*,
    // though it still needs one to be performed, so the run fails at the
    // uninstall and the assertion is that it failed HONESTLY: exit 1, aichat
    // still on disk, still owned.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"fzf":"installed","aichat":"adopted"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");
    f.install_app("aichat", "0.30.0");

    let out = f.run(&["apply", "--yes", "--allow-prune"]);
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));

    assert!(all.contains("scoop.cmd"), "it must have tried: {all}");
    assert_eq!(out.status.code(), Some(2), "nothing changed, so 2: {all}");
    assert!(
        f.scoop.path().join("apps").join("aichat").exists(),
        "a failed uninstall must leave the app alone"
    );
    let state: String =
        fs::read_to_string(f.local.path().join("dotpkg").join("state.json")).unwrap();
    assert!(
        state.contains("aichat"),
        "and must not release an app that is still installed: {state}"
    );
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --lib apply::tests::only_an_explicit_yes`
Expected: FAIL to compile — `cannot find function `confirm``.

- [ ] **Step 3: Implement `confirm` and the driver**

In `src/apply.rs`:

```rust
/// Ask, and treat anything that is not an explicit yes as a no.
///
/// The question goes to `err` so that `apply | tee` still shows it while the
/// plan and preparation tables stay on stdout.
///
/// `Ok(0)` from `read_line` is what a child process with no console gets — the
/// medium-integrity scheduled task the dogfood runs under — and it means
/// **no**, loudly, naming `--yes`. `is_terminal()` is deliberately not
/// consulted: whether a terminal is attached is not the same question as
/// whether the user agreed.
pub fn confirm(
    question: &str,
    input: &mut dyn std::io::BufRead,
    err: &mut dyn std::io::Write,
) -> Result<bool> {
    write!(err, "{question}")?;
    err.flush()?;
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) => {
            writeln!(
                err,
                "\napply needs an answer and stdin is not readable. Pass --yes if you \
                 have read the plan above."
            )?;
            Ok(false)
        }
        Ok(_) => {
            let a = line.trim().to_ascii_lowercase();
            Ok(a == "y" || a == "yes")
        }
        Err(e) => {
            writeln!(err, "\ncannot read an answer ({e}); treating that as no. Pass --yes.")?;
            Ok(false)
        }
    }
}

/// Everything a command needs, loaded once.
///
/// `main.rs` carried two inline copies of this and `tests/` reached neither.
pub struct Driver {
    pub declared: Config,
    pub locked: Lock,
    pub state: State,
    pub scoop: Scoop,
    pub scan: crate::backend::Scan,
    pub running: crate::model::Running,
}

pub fn load_everything(config: &Path, lock: &Path, state_path: &Path) -> Result<Driver> {
    let declared = crate::config::load(config)?;
    let locked = crate::lock::load_or_empty(lock)?;
    let state = State::load_or_empty(state_path)?;
    let scoop = Scoop::discover();
    let scan = <Scoop as crate::backend::Backend>::scan(&scoop)?;
    let procs = crate::sys::running_processes();
    let running = scoop.running_set(&procs);
    Ok(Driver { declared, locked, state, scoop, scan, running })
}
```

- [ ] **Step 4: Rewrite the Apply arm**

In `src/main.rs`, add `use dotpkg::execute::Step;` and add five flags to the
`Apply` variant — `yes`, `allow_prune`, `keep_going`, `clone_missing_buckets`,
and `state: Option<PathBuf>` — then replace the arm's body with the sequence
the spec fixes:

```rust
            let state_path = state.unwrap_or_else(State::default_path);
            anyhow::ensure!(
                state_path.is_absolute(),
                "the state file resolves to {}, which is relative to the current \
                 directory. Pass --state with an absolute path.",
                state_path.display()
            );

            let declared_only = dotpkg::config::load(&config)?;
            let state_only = State::load_or_empty(&state_path)?;
            if !allow_empty_config {
                dotpkg::apply::mass_prune_guard(&declared_only, &state_only)?;
            }
            let locked_only = dotpkg::lock::load_or_empty(&lock)?;
            dotpkg::apply::lock_coherence_guard(&declared_only, &locked_only)?;

            let mut d = dotpkg::apply::load_everything(&config, &lock, &state_path)?;
            for w in &d.scan.warnings {
                eprintln!("warning: scoop: {w}");
            }

            let plan =
                dotpkg::plan::plan(&d.declared, &d.locked, &d.scan.installed, &d.state, &d.running);
            print!("{}", dotpkg::render::render(&plan));

            if clone_missing_buckets {
                for (name, why) in d.scoop.clone_missing_buckets(&d.declared) {
                    eprintln!("warning: could not add bucket {name}: {why}");
                }
            }

            let staging_root = dotpkg::apply::default_staging_root();
            let preparation =
                dotpkg::apply::prepare(&plan, &d.locked, &d.scoop, &staging_root);
            print!("{}", dotpkg::render::render_preparation(&preparation));
            std::io::stdout().flush().ok();

            if prepare {
                if !preparation.is_ok() {
                    std::process::exit(1);
                }
                return Ok(());
            }

            let (mut steps, unusable) = dotpkg::apply::plan_to_steps(&preparation);
            let removals = steps.iter().filter(|s| matches!(s, Step::Remove { .. })).count();

            if !preparation.is_ok() && !keep_going {
                eprintln!(
                    "\n{} package(s) could not be prepared, so nothing has been changed. \
                     Fix them, or pass --keep-going to install the {} that are ready \
                     (removals stay held either way).",
                    unusable.len(),
                    steps.len() - removals
                );
                std::process::exit(2);
            }
            if !preparation.is_ok() {
                // Removals are gated on the whole preparation being ok, and no
                // flag opens that gate: a swap that installs nothing and
                // deletes something is the one shape reachable today, because
                // every newly typed package name is NotLocked until `update`
                // exists.
                steps.retain(|s| !matches!(s, Step::Remove { .. }));
            }
            if removals > 0 && yes && !allow_prune {
                anyhow::bail!(
                    "this run would remove {removals} package(s) and --yes was passed. \
                     Removals need --allow-prune as well."
                );
            }

            let question = format!(
                "\n{} package(s) will be uninstalled and reinstalled, {} installed, \
                 {} removed. Every version change is an uninstall followed by an \
                 install, in both directions. Continue? [y/N] ",
                steps.iter().filter(|s| matches!(s, Step::Replace { .. })).count(),
                steps.iter().filter(|s| matches!(s, Step::Install { .. })).count(),
                steps.iter().filter(|s| matches!(s, Step::Remove { .. })).count(),
            );
            if !yes {
                let stdin = std::io::stdin();
                let mut lock_in = stdin.lock();
                let mut errout = std::io::stderr();
                if !dotpkg::apply::confirm(&question, &mut lock_in, &mut errout)? {
                    eprintln!("Nothing has been changed.");
                    std::process::exit(2);
                }
            }

            let opts = dotpkg::execute::ExecOptions {
                recovery_path: staging_root.parent().map(|p| p.join("recover.cmd")),
            };
            let mut ex = dotpkg::execute::execute(
                d.scoop.root(),
                steps,
                &d.scoop,
                &mut d.state,
                &d.running,
                &opts,
            );

            // Report only what a fresh scan confirms.
            let after = <Scoop as dotpkg::backend::Backend>::scan(&d.scoop)?;
            let present: Vec<_> = after.installed.iter().map(|i| i.name.clone()).collect();
            ex.dropped_ghosts = d.state.reconcile(dotpkg::model::SCOOP, &present);
            d.state.save(&state_path)?;

            print!("{}", dotpkg::render::render_execution(&ex));
            std::io::stdout().flush().ok();
            let code = ex.exit_code(false);
            if code != 0 {
                std::process::exit(code);
            }
```

`Scoop::root()` becomes a public accessor.

- [ ] **Step 5: Implement `render_execution`**

In `src/render.rs`:

```rust
/// What the run actually did, according to the disk.
///
/// It never says "N upgraded". Every number here comes from a `verdict`
/// against the filesystem, and the wording says so, because the tool this
/// orchestrates reports success unconditionally.
pub fn render_execution(ex: &Execution) -> String {
    let mut out = String::new();
    for (name, r) in &ex.results {
        let line = match r {
            ItemResult::Done => format!("  done    scoop  {name:<13}verified on disk"),
            ItemResult::Failed(why) => format!("  FAILED  scoop  {name:<13}{why}"),
            ItemResult::Held(why) => format!("  held    scoop  {name:<13}{why}"),
        };
        out.push_str(&line);
        out.push('\n');
    }
    for name in &ex.dropped_ghosts {
        out.push_str(&format!(
            "  note    scoop  {name:<13}ownership record dropped: nothing by that name is installed\n"
        ));
    }
    out.push_str(&format!(
        "\n  {} verified on disk, {} failed, {} held.\n",
        ex.changed(),
        ex.failed(),
        ex.held()
    ));
    if ex.failed() > 0 {
        out.push_str(
            "  Some packages were changed and some were not. Look at the machine.\n",
        );
    }
    out
}
```

- [ ] **Step 6: Run**

Run: `cargo test --all` — expected: 199 passed.
Run: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` — clean.

- [ ] **Step 7: Negative controls**

1. Make `confirm` return `Ok(true)` for `Ok(0)`. Record that
   `no_answer_at_all_means_no_and_says_which_flag_would_have_helped` and
   `apply_with_no_answer_available_refuses_and_changes_nothing` both fail.
   Restore.
2. Delete the `steps.retain(|s| !matches!(s, Step::Remove { .. }))` line.
   Record which test fails; if none does, **add one** — a prepared plan with
   one `Failed` install and one ready prune, run with `--keep-going`, asserting
   the prune was held. This is the gate the whole halt-versus-proceed decision
   rests on and it must not be uncovered.
3. Delete the `--allow-prune` check. Record that
   `yes_alone_does_not_authorise_a_prune` fails. Restore.

- [ ] **Step 8: Commit**

```bash
git add src/main.rs src/apply.rs src/render.rs tests/cli.rs
git commit -m "Wire up the executor: one driver, one question, three exit codes

The question goes to stderr so a piped run still shows it, and an
unreadable stdin -- what a console-less scheduled task gives a child --
means no, not yes. --yes answers that question and nothing else; a run
containing removals needs --allow-prune as well, which is the cheapest
answer to one surviving declared package disarming the mass-prune guard.

Removals execute only when the whole preparation is ok, and --keep-going
does not open that gate."
```

---

## Task 13: Documentation and the carried-notes update

**Files:**
- Modify: `README.md`, `docs/phase2b-notes.md`
- Create: `docs/measurements-2026-08-08-scoop-exit-codes.md`

- [ ] **Step 1: Record the measurements as their own document**

Move the measurement tables out of the spec's prose into
`docs/measurements-2026-08-08-scoop-exit-codes.md`, with the exact commands,
the raw stdout excerpts, the machine state before and after (31 apps, 75 cache
entries, kanata PID 7868, explorer PID 9620), and the two contaminated results
that yield nothing (shim creation in a root with no `apps/scoop`). The spec
links to it.

- [ ] **Step 2: Close the carried notes**

In `docs/phase2b-notes.md`'s "Carried into Phase 2b-2", mark each item closed
with the commit that closed it, and correct the two claims this phase
disproved: `main.rs` had **two** inline copies, not three; and `scoop install`
over an installed app **does** print a `WARN` line on stdout, naming the
version already installed rather than the one requested.

- [ ] **Step 3: Update the README**

Document `--yes`, `--allow-prune`, `--keep-going`, `--clone-missing-buckets`,
`--state`, and the three exit codes.

- [ ] **Step 4: Run the whole suite one more time**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all`

- [ ] **Step 5: Commit**

```bash
git add README.md docs/
git commit -m "Record the scoop exit-code measurements and close the 2b-2 notes"
```

---

## Task 14: Dogfood on a14

Not a code task. Follows the spec's "Dogfood" section, and produces
`docs/dogfood-phase2b2-<date>.md`.

**Preconditions, all verifiable before anything is changed:**

- `ssh a14` answers. If it does not, **report BLOCKED and write nothing.**
- Baseline captured: app count, per-app version via the `.Target` junction-safe
  method, cache count, `kanata` PID, `explorer` PID.
- kanata is never started or stopped.

- [ ] **Stage 1 — a throwaway `$env:SCOOP` root.** Build the binary, point it
  at a probe root with two real staged manifests and a deliberately divergent
  lock. Confirm: an induced silent no-op is caught (make the manifest a
  same-version content swap); `recover.cmd` exists before the first mutation
  and restores a package after a killed install; a failed prefetch refuses the
  run and changes nothing.

- [ ] **Stage 2 — the real `~/scoop`, two gated acts.** `state.json` on a14
  owns nothing, so prune cannot be exercised until dotpkg's own install path
  puts an entry there. Act one: downgrade one leaf package and confirm exactly
  one new `state.json` key. Act two: undeclare that package and prune it,
  confirming the key disappears and the app directory is gone. Run at medium
  integrity via the scheduled-task XML clone.

- [ ] **Write the record**, answering the spec's five framed questions, and
  including anything that came back **different** from what this plan
  predicted. The prediction on record is that verification fires at least once
  for a reason nobody anticipated. If it never fires, say so and treat it as a
  reason for suspicion.

- [ ] **Clean up and verify:** probe root, staging, scheduled task, every
  scratch file, each re-checked with `Test-Path`. Confirm the app count,
  version table, kanata PID and explorer PID against the baseline.

---

## Task 15: Whole-branch review before merge

Separate from the per-task reviews, and the thing that found the most in both
previous phases — because it **runs** mutations rather than reading them.

- [ ] Re-run every negative control in this plan from a clean checkout, in one
  pass, and record which assertion fired for each. A control that cannot be
  reproduced is a control that was never run.
- [ ] Run the mutation that motivated Task 1 — inject a `state.json` write into
  a path that should not write — and confirm the suite now goes red.
- [ ] Confirm no test creates a file at `Scoop::scoop_exe()`'s path.
- [ ] Confirm `cargo test --all` is green with `--test-threads=1` as well, since
  several new tests write to real temp trees.
- [ ] Read the diff for any place a scoop exit code is consulted. There must be
  none outside `CommandReport`, where it is recorded and not judged.
