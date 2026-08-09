# Phase 3 — `update` and `adopt` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `dotpkg update` re-resolves `pkg.toml` against fetched buckets and rewrites `pkg.lock`; `dotpkg adopt` brings an already-installed package under management by writing all three files.

**Architecture:** All git moves into one new module, `src/bucket.rs`, which both commands and the existing `Scoop::stage` call. `update` resolves *latest* (plain `git log -1`, checked against the fetched tip). `adopt` resolves *what is installed* (`--full-history` plus one `git cat-file --batch`, matching installed manifest bytes first and version second). Both write through pure functions that are tested with no git at all; the git layer is tested against real repositories built in a `tempfile::tempdir`.

**Tech Stack:** Rust 2021, rust-version 1.85. `anyhow`, `clap` 4 (derive), `serde`, `serde_json`, `sysinfo`, `toml` 0.8, and **`toml_edit` 0.22 promoted from transitive to direct** (already in `Cargo.lock` at 0.22.27 — adds no crate to the tree). Dev: `tempfile`.

## Global Constraints

Copied verbatim from `docs/specs/2026-08-09-phase3-update-adopt-design.md` and the standing rules the previous three phases established. **Every task's requirements implicitly include this section.**

- **`commit` is a commit at which `bucket/<app>.json` has the pinned content.** It is *not* a claim about which commit authored that version. Nothing may assert the stronger claim.
- **`update` uses plain `git log -1`; `adopt` uses `--full-history`.** Measured: `--full-history` makes `update` return a merge commit; its absence makes `adopt` miss a version that is a genuine ancestor of HEAD.
- **Content matching must go through `verify::normalise`.** scoop rewrites line endings; a raw byte comparison against a bucket blob matches nothing.
- **`update` fetches; it never pulls and never checks out.** Resolution reads a remote-tracking ref. A failed or absent fetch warns, falls back to the local ref, and *says so in the output*.
- **`adopt` writes `pkg.lock` → `pkg.toml` → `state.json`, in that order.** Every prefix of that order is inert. `state.json` first is the prune-candidate shape and is forbidden.
- **`adopt` refuses and writes nothing at all** when no commit carries the installed version.
- **`pkg.toml` is the user's file.** Edited with `toml_edit`, displaced copy kept as `pkg.toml.bak`, and the result re-parsed with `config::parse` and compared before it replaces the original.
- **A failed re-resolve keeps the previous lock entry.** Only removal from `pkg.toml` drops an entry.
- **Bucket ambiguity refuses**, naming every bucket, and points at `[scoop.opts] <pkg> = { bucket = "..." }`.
- **Run `cargo test --no-fail-fast`.** cargo stops at the first failing target and hid two real Windows defects for several rounds in Phase 2b-2.
- **No test may create a file at `Scoop::scoop_exe()`'s path.** Enforced by the existing source scan over `tests/`.
- **Commit message style is this repository's**, not conventional commits: a sentence-style subject in the imperative, e.g. `Refuse a pkg.lock commit that is not a hash`. **No `Co-Authored-By` trailer.**
- Every negative control must be **run** and the assertion that fired **recorded** in the commit message. A control that cannot go red is a plan failure, not a passing test.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/bucket.rs` **(new)** | Every git invocation in the crate. Fetch, tip resolution, shallow detection, filename spelling, `git show`, per-file history, batched blob reads. |
| `src/update.rs` **(new)** | `update`'s pure decision layer (`resolve_into_lock`) and its driver. |
| `src/adopt.rs` **(new)** | `adopt`'s per-package resolution and its three-file write. |
| `src/config_edit.rs` **(new)** | Adding a package to a hand-written `pkg.toml` without destroying it. |
| `tests/common/mod.rs` **(new)** | `BucketFixture`: builds real git repositories in the shapes the measurements found. |
| `src/lock.rs` | +`render`, +`save`. Existing parse untouched; one test's fixture corrected. |
| `src/config.rs` | +`PkgOpts.bucket`. |
| `src/backend/scoop.rs` | git helpers move out to `bucket.rs`; staging path gains the commit. |
| `src/execute.rs` | `Mutator` gains `download`. |
| `src/apply.rs` | `prepare`/`stage_and_fetch` take the mutator. |
| `src/render.rs` | +`render_update`, +`render_adopt`. |
| `src/main.rs` | `Update` and `Adopt` subcommands; `status` warns on an incoherent lock. |
| `src/lib.rs` | Module declarations. |

---

## Task 1: Put `download` behind `Mutator`

Closes the one item `docs/phase2b-notes.md` still lists as open: nothing produced by real code is ever asserted to be `Outcome::ReadyToFetch`, and the last line of `stage_and_fetch` — that `arch` reaches `scoop download`'s argv — has no test on any platform.

**Files:**
- Modify: `src/execute.rs` (the `Mutator` trait, ~line 30)
- Modify: `src/backend/scoop.rs` (`Scoop::download`, `impl Mutator for Scoop`)
- Modify: `src/apply.rs` (`prepare`, `stage_and_fetch`)
- Modify: `src/main.rs` (the one `prepare` call site)
- Test: `tests/prepare.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `Mutator::download(&self, manifest: &Path, arch: Option<&str>) -> Result<CommandReport>`; `dotpkg::backend::scoop::download_outcome(stdout: &str, manifest: &Path) -> Result<()>`; `apply::prepare(plan, lock, scoop: &Scoop, mutator: &dyn Mutator, staging_root, declared)`.

- [ ] **Step 1: Write the failing test**

Add to `tests/prepare.rs`:

```rust
use dotpkg::apply::{prepare, Outcome};
use dotpkg::execute::{CommandReport, Mutator};
use std::cell::RefCell;

/// A fake scoop that only ever downloads. It records the argv it was handed
/// and reports scoop's measured success shape.
///
/// It deliberately cannot uninstall or install: `prepare` must never reach
/// those, and a fake that silently permits them could not prove it.
struct Downloader {
    calls: RefCell<Vec<(std::path::PathBuf, Option<String>)>>,
    verified: bool,
}

impl Downloader {
    fn ok() -> Downloader {
        Downloader { calls: RefCell::new(Vec::new()), verified: true }
    }
    fn hash_failure() -> Downloader {
        Downloader { calls: RefCell::new(Vec::new()), verified: false }
    }
}

impl Mutator for Downloader {
    fn uninstall(&self, app: &dotpkg::model::Name) -> anyhow::Result<CommandReport> {
        panic!("prepare must never uninstall anything, but it asked for {app}");
    }
    fn install(&self, m: &Path, _a: Option<&str>) -> anyhow::Result<CommandReport> {
        panic!("prepare must never install anything, but it asked for {}", m.display());
    }
    fn download(&self, manifest: &Path, arch: Option<&str>) -> anyhow::Result<CommandReport> {
        self.calls
            .borrow_mut()
            .push((manifest.to_path_buf(), arch.map(str::to_string)));
        // Both branches exit 0. Measured on a14: scoop reports a hash failure
        // through stdout and nothing else.
        let stdout = if self.verified {
            "Checking hash of tool-1.0.0.zip ... ok.\n'tool' (1.0.0) was downloaded successfully!\n"
        } else {
            "Checking hash of tool-1.0.0.zip ... ERROR Hash check failed!\n\
             'tool' (1.0.0) was downloaded successfully!\n"
        };
        Ok(CommandReport { code: Some(0), stdout: stdout.into(), stderr: String::new() })
    }
}

fn one_install_plan(name: &str, version: &str, arch: Option<&str>) -> dotpkg::plan::Plan {
    dotpkg::plan::Plan {
        actions: vec![dotpkg::plan::Action::Install {
            backend: dotpkg::model::SCOOP.into(),
            name: Name::new(name),
            version: version.into(),
            arch: arch.map(str::to_string),
        }],
    }
}

#[test]
fn a_real_ready_to_fetch_is_produced_by_production_code_and_carries_the_architecture() {
    // Two things at once, both of which Phase 2b-2 left unproven on every
    // platform: that `Outcome::ReadyToFetch` is reachable from real code at
    // all (every value of it in the suite was hand-built), and that the
    // architecture the planner resolved actually reaches the download argv.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);
    let scoop = Scoop::new(root.path().to_path_buf());
    let declared = dotpkg::config::parse(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n",
    )
    .unwrap();
    let mut lock = dotpkg::lock::Lock::default();
    lock.scoop.insert(Name::new("tool"), pin("main", &shas[0], "1.0.0"));

    let fake = Downloader::ok();
    let prep = prepare(
        &one_install_plan("tool", "1.0.0", Some("arm64")),
        &lock,
        &scoop,
        &fake,
        stage_dir.path(),
        &declared,
    );

    let staged = match &prep.prepared[0].outcome {
        Outcome::ReadyToFetch { manifest } => manifest.clone(),
        other => panic!("expected ReadyToFetch from real code, got {other:?}"),
    };
    assert!(staged.exists(), "the manifest must really be on disk");

    let calls = fake.calls.borrow();
    assert_eq!(calls.len(), 1, "exactly one download: {calls:?}");
    assert_eq!(calls[0].0, staged, "download must be handed the staged path");
    assert_eq!(
        calls[0].1.as_deref(),
        Some("arm64"),
        "the architecture the plan resolved must reach the download argv"
    );
}

#[test]
fn a_hash_failure_that_exits_zero_is_still_a_failed_outcome() {
    // The positive control's sibling. Without it, a `download` that ignored
    // its stdout entirely would pass the test above.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);
    let scoop = Scoop::new(root.path().to_path_buf());
    let declared =
        dotpkg::config::parse("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n").unwrap();
    let mut lock = dotpkg::lock::Lock::default();
    lock.scoop.insert(Name::new("tool"), pin("main", &shas[0], "1.0.0"));

    let prep = prepare(
        &one_install_plan("tool", "1.0.0", None),
        &lock,
        &scoop,
        &Downloader::hash_failure(),
        stage_dir.path(),
        &declared,
    );

    match &prep.prepared[0].outcome {
        Outcome::Failed { why } => assert!(
            why.contains("hash"),
            "name the diagnosis, not just that it failed: {why}"
        ),
        other => panic!("a hash failure must not be ready: {other:?}"),
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --no-fail-fast --test prepare 2>&1 | tail -30`
Expected: FAIL to compile — `no method named 'download' found for trait Mutator`, and `prepare` takes 5 arguments, not 6.

- [ ] **Step 3: Add `download` to the trait**

In `src/execute.rs`, extend the trait. Keep the existing doc comment and add:

```rust
pub trait Mutator {
    fn uninstall(&self, app: &Name) -> Result<CommandReport>;
    fn install(&self, manifest: &Path, arch: Option<&str>) -> Result<CommandReport>;
    /// Fetch and hash-verify. Not a mutation of installed software, but it is
    /// the third scoop invocation and it belongs behind the same seam: until
    /// it was here, no test on any platform could produce an
    /// `Outcome::ReadyToFetch` from production code, or see the argv that
    /// carries the resolved architecture.
    fn download(&self, manifest: &Path, arch: Option<&str>) -> Result<CommandReport>;
}
```

- [ ] **Step 4: Split `Scoop::download` into a runner and a pure verdict**

In `src/backend/scoop.rs`, replace `pub fn download` in the `impl Scoop` block with a free function, and add the trait method. `download_verdict` and its tests stay exactly as they are.

```rust
/// Turn `scoop download`'s stdout into a result. Pure, so the whole of the
/// prefetch promise is testable without a subprocess.
///
/// The exit code is not a parameter, deliberately: measured on a14, `scoop
/// download` returns 0 for a hash mismatch and for a dead URL, so a signature
/// that accepted a code would invite someone to consult it.
pub fn download_outcome(stdout: &str, manifest: &Path) -> Result<()> {
    match download_verdict(stdout) {
        FetchVerdict::Verified => Ok(()),
        FetchVerdict::HashFailed => anyhow::bail!(
            "hash check failed for {}: {}", manifest.display(), tail(stdout)
        ),
        FetchVerdict::UrlDead => anyhow::bail!(
            "the manifest's url is gone for {}: {}", manifest.display(), tail(stdout)
        ),
        FetchVerdict::Unproven => anyhow::bail!(
            "scoop download did not report a verified hash for {} (it exits 0 either way, \
             so this is treated as a failure): {}",
            manifest.display(),
            tail(stdout)
        ),
    }
}
```

And in `impl crate::execute::Mutator for Scoop`:

```rust
    fn download(&self, manifest: &Path, arch: Option<&str>) -> Result<crate::execute::CommandReport> {
        self.run(&download_argv(manifest, arch))
    }
```

Delete the old `impl Scoop { pub fn download(...) }` block entirely.

- [ ] **Step 5: Thread the mutator through `prepare`**

In `src/apply.rs`, change both signatures and the one call:

```rust
pub fn prepare(
    plan: &Plan,
    lock: &Lock,
    scoop: &Scoop,
    mutator: &dyn crate::execute::Mutator,
    staging_root: &Path,
    declared: &Config,
) -> Preparation {
```

Pass `mutator` into `stage_and_fetch`, and replace the block that carried the "NOT COVERED BY ANY TEST ON THIS PLATFORM" comment with:

```rust
    let staged = scoop.stage(staging_root, name, pin).and_then(|manifest| {
        // Behind `Mutator` since Phase 3, which is what finally lets a test on
        // any OS see both that this produces a real `Outcome::ReadyToFetch`
        // and that `arch` -- not `None` -- reaches the argv.
        let report = mutator.download(&manifest, arch.as_deref())?;
        crate::backend::scoop::download_outcome(&report.stdout, &manifest)?;
        Ok(manifest)
    });
```

In `src/main.rs`, the call becomes:

```rust
            let preparation = dotpkg::apply::prepare(
                &plan, &d.locked, &d.scoop, &d.scoop, &staging_root, &d.declared,
            );
```

- [ ] **Step 6: Run the whole suite**

Run: `cargo test --no-fail-fast 2>&1 | grep -E "^test result:"`
Expected: every target `ok`, total 260 (258 + the 2 new).

- [ ] **Step 7: Run the negative control and record what fired**

Make `download_outcome` return `Ok(())` unconditionally, run `cargo test --no-fail-fast --test prepare`, and confirm `a_hash_failure_that_exits_zero_is_still_a_failed_outcome` goes red while `a_real_ready_to_fetch_is_produced_by_production_code_and_carries_the_architecture` stays green. Then restore. Record the assertion text in the commit message.

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -F - <<'EOF'
Put download behind Mutator, and produce a real ReadyToFetch at last

The last item docs/phase2b-notes.md listed as open. Every Outcome::ReadyToFetch
in the suite was hand-built, because stage_and_fetch called Scoop::download
directly and no test may put a file at Scoop::scoop_exe()'s path. The comment
at that call site said so itself and named a later task; this is that task.

Two things become provable on macOS as a result: that production code reaches
ReadyToFetch at all, and that the architecture the planner resolved actually
arrives in the download argv rather than None.

Negative control: download_outcome returning Ok(()) unconditionally leaves
a_hash_failure_that_exits_zero_is_still_a_failed_outcome red on "a hash failure
must not be ready: ReadyToFetch { .. }" with the positive sibling green.
EOF
```

---

## Task 2: `status` warns on a lock `apply` would refuse

**Files:**
- Modify: `src/main.rs` (the `Status` arm)
- Modify: `src/lock.rs` (one test fixture)
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: `apply::lock_coherence_guard` (shipped).
- Produces: no new API.

- [ ] **Step 1: Write the failing test**

Add to `tests/cli.rs`, following that file's existing `Fixture` idiom:

```rust
#[test]
fn status_says_so_when_the_lock_is_one_apply_would_refuse() {
    // The worst pairing available before this: status prints an actionable
    // plan, apply exits 2 on the same two files. `status` still prints the
    // plan -- it is read-only and its whole product is telling the truth --
    // but it no longer does so in silence.
    let f = Fixture::new();
    f.write("pkg.toml", "[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n");
    f.write(
        "pkg.lock",
        "[scoop.tool]\nbucket = \"main\"\ncommit = \"main\"\nversion = \"1.0.0\"\n",
    );

    let out = f.run(&["status"]);
    assert_eq!(out.code, Some(0), "status stays read-only and never refuses");
    assert!(
        out.stderr.contains("not a commit hash"),
        "name the diagnosis: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("dotpkg update"),
        "name the command that fixes it -- it exists now: {}",
        out.stderr
    );
    // The plan is still printed. Without this, making status bail would pass
    // the assertions above while removing the thing status is for.
    assert!(
        out.stdout.contains("tool"),
        "the plan must still be printed: {}",
        out.stdout
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --no-fail-fast --test cli status_says_so 2>&1 | tail -20`
Expected: FAIL — `name the diagnosis:` with an empty stderr.

- [ ] **Step 3: Implement**

In `src/main.rs`'s `Status` arm, after `let locked = ...` and before the plan:

```rust
            // A warning, not a refusal. `apply` exits 2 on this lock, and
            // until now `status` printed an actionable plan from it in
            // silence. Refusing here would withhold exactly the information
            // the user needs to fix it, so the plan is still printed --
            // `status` is read-only and its whole product is the truth about
            // this machine.
            if let Err(e) = dotpkg::apply::lock_coherence_guard(&locked) {
                eprintln!("warning: {e:#}");
                eprintln!("warning: `dotpkg apply` will refuse this lock. The plan below is what it describes, not what apply would do.");
            }
```

- [ ] **Step 4: Correct the lock test fixture that enshrines a rejected shape**

In `src/lock.rs`, `parses_both_backends_into_distinct_pin_shapes` uses `commit = "a28d0c5648f1"` — twelve hex characters, which `lock_coherence_guard` and `Scoop::stage` both refuse. Replace it with a real 40-hex value and add a test that states the split deliberately:

```rust
        let lock = parse(
            r#"
[scoop.fzf]
bucket  = "main"
commit  = "a28d0c5648f1e9d3b7c2a41f6e8b9d0c5a7f3e12"
version = "0.74.1"

[winget."Git.Git"]
version = "2.55.0"
pin     = "version-only"
"#,
        )
        .unwrap();
```

Update the matching `assert_eq!` to the same 40-character string, then add:

```rust
    #[test]
    fn parse_accepts_a_commit_the_guards_reject_and_that_split_is_deliberate() {
        // There is no hex check here, on purpose. A lock too broken to run
        // must still be READABLE, or `status` could not explain it and
        // `update` could not tell the user which entries it is replacing.
        // The refusal lives in `apply::lock_coherence_guard` and in
        // `Scoop::stage`, both of which run before anything is staged.
        let lock = parse(
            "[scoop.fzf]\nbucket = \"main\"\ncommit = \"main\"\nversion = \"0.74.1\"\n",
        )
        .expect("parse must not be the layer that refuses this");
        assert_eq!(lock.scoop.len(), 1);

        let err = crate::apply::lock_coherence_guard(&lock).unwrap_err();
        assert!(format!("{err:#}").contains("hex"), "got {err:#}");
    }
```

- [ ] **Step 5: Run the suite**

Run: `cargo test --no-fail-fast 2>&1 | grep -E "^test result:"`
Expected: all `ok`.

- [ ] **Step 6: Run the negative control**

Delete the two `eprintln!` lines added in Step 3. Confirm `status_says_so_when_the_lock_is_one_apply_would_refuse` goes red on `name the diagnosis:` and that no other test changes. Restore. Then delete only the *second* `eprintln!` and confirm the test still goes red on the `dotpkg update` assertion — proving both lines are load-bearing rather than one covering the other.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -F - <<'EOF'
Warn from status about a lock apply would refuse

status did not run lock_coherence_guard, so it printed an actionable plan
built from a lock that apply exits 2 on -- the two commands disagreeing about
the same two files, with nothing said. It warns now and still prints the plan:
status is read-only and its product is the truth about the machine, so
refusing would withhold what the user needs to fix it.

The guard's message ends in "Run `dotpkg update`", which stops being a pointer
to a command that does not exist as of this phase.

src/lock.rs's parse test used a twelve-character commit -- input both shipped
guards reject -- which read like the documented shape. It now uses a real
40-hex value, and a new test states the split outright: parse stays permissive
so a broken lock is still readable, and the refusal lives in the guards.

Negative control: removing either eprintln! leaves the test red on its own
assertion, so neither line covers for the other.
EOF
```

---

## Task 3: `src/bucket.rs` — move every git invocation into one module

Pure refactor of shipped code plus three new read-only probes. The existing `tests/prepare.rs` suite is the safety net: `Scoop::stage`'s behaviour must not change.

**Files:**
- Create: `src/bucket.rs`
- Modify: `src/backend/scoop.rs` (delete `git_ok`, `git_show`, `resolve_spelling`; call `bucket::` instead)
- Modify: `src/lib.rs`
- Test: `tests/common/mod.rs` (new), `tests/bucket.rs` (new)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `bucket::git_ok(dir: &Path, args: &[&str]) -> bool`
  - `bucket::git_show(dir: &Path, rev: &str, path_in_repo: &str) -> Result<Option<String>>`
  - `bucket::resolve_spelling(dir: &Path, rev: &str, app_key: &str) -> Option<String>`
  - `bucket::is_shallow(dir: &Path) -> bool`
  - `bucket::tip(dir: &Path) -> Tip`, with `pub struct Tip { pub rev: String, pub stale: Option<String> }`
  - `bucket::fetch(dir: &Path) -> Result<()>`

- [ ] **Step 1: Write the fixture builder**

Create `tests/common/mod.rs`. This is the shape every later git test uses, and every shape in it is a scenario from `docs/measurements-2026-08-09-git-resolution.md`.

```rust
//! Real git repositories in the shapes the Phase 3 measurements found.
//!
//! git, unlike scoop, is on every machine this crate is developed on, so the
//! riskiest code in Phase 3 is tested against the real binary rather than
//! against a fake that can only be self-consistent.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} in {}: {e}", dir.display()));
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

pub struct Fixture {
    pub home: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Fixture {
        Fixture { home: tempfile::tempdir().unwrap() }
    }
    pub fn scoop_root(&self) -> PathBuf {
        self.home.path().join("scoop")
    }
    pub fn bucket_dir(&self, bucket: &str) -> PathBuf {
        self.scoop_root().join("buckets").join(bucket)
    }

    /// An empty bucket repository with an identity configured.
    pub fn bucket(&self, name: &str) -> PathBuf {
        let dir = self.bucket_dir(name);
        std::fs::create_dir_all(dir.join("bucket")).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "t@example.invalid"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        dir
    }

    /// Commit one manifest and return the sha.
    pub fn commit(&self, dir: &Path, file: &str, version: &str, url_tag: &str) -> String {
        std::fs::write(
            dir.join("bucket").join(file),
            format!("{{\n    \"version\": \"{version}\",\n    \"url\": \"https://example.invalid/{url_tag}.zip\"\n}}\n"),
        )
        .unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", &format!("{file} {version}")]);
        git(dir, &["rev-parse", "HEAD"]).trim().to_string()
    }

    /// The blob for `<rev>:bucket/<file>` exactly as git stores it.
    pub fn blob(&self, dir: &Path, rev: &str, file: &str) -> String {
        git(dir, &["show", &format!("{rev}:bucket/{file}")])
    }
}

/// Section B of the measurements: a version that reached the bucket only on a
/// side branch whose change was superseded at merge time. `git log -- <path>`
/// cannot see it; `--full-history` can.
///
/// Returns `(side_commit_for_1_0_1, main_commit_for_1_0_2)`.
pub fn merged_bucket(f: &Fixture, name: &str) -> (String, String) {
    let dir = f.bucket(name);
    f.commit(&dir, "tool.json", "1.0.0", "v100");
    git(&dir, &["checkout", "-q", "-b", "side"]);
    let side = f.commit(&dir, "tool.json", "1.0.1", "side101");
    git(&dir, &["checkout", "-q", "main"]);
    let main = f.commit(&dir, "tool.json", "1.0.2", "main102");
    git(&dir, &["merge", "-q", "--no-ff", "-X", "ours", "side", "-m", "merge side"]);
    (side, main)
}

/// Section E: the bucket spells the file with different case at an older
/// commit. Built with plumbing and never checked out -- `git mv` cannot make
/// this on macOS or Windows, whose filesystems are case-insensitive, and the
/// first probe run measured nothing because it tried.
pub fn case_renamed_bucket(f: &Fixture, name: &str) -> (String, String) {
    let dir = f.bucket(name);
    let old_body = "{\n    \"version\": \"1.0.0\"\n}\n";
    let new_body = "{\n    \"version\": \"1.0.1\"\n}\n";

    let write_tree = |path: &str, body: &str| -> String {
        let sha = {
            let mut c = Command::new("git");
            c.current_dir(&dir).args(["hash-object", "-w", "--stdin"]);
            c.stdin(std::process::Stdio::piped());
            c.stdout(std::process::Stdio::piped());
            let mut child = c.spawn().unwrap();
            use std::io::Write;
            child.stdin.as_mut().unwrap().write_all(body.as_bytes()).unwrap();
            let out = child.wait_with_output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&dir, &["read-tree", "--empty"]);
        git(&dir, &["update-index", "--add", "--cacheinfo", &format!("100644,{sha},{path}")]);
        git(&dir, &["write-tree"]).trim().to_string()
    };

    let t1 = write_tree("bucket/Tool.json", old_body);
    let c1 = git(&dir, &["commit-tree", &t1, "-m", "Tool 1.0.0"]).trim().to_string();
    let t2 = write_tree("bucket/tool.json", new_body);
    let c2 = git(&dir, &["commit-tree", &t2, "-p", &c1, "-m", "tool 1.0.1"]).trim().to_string();
    git(&dir, &["update-ref", "refs/heads/main", &c2]);
    (c1, c2)
}
```

- [ ] **Step 2: Write the failing tests for the three new probes**

Create `tests/bucket.rs`:

```rust
mod common;

use common::*;
use dotpkg::bucket;

#[test]
fn the_tip_is_the_upstream_ref_when_there_is_one() {
    // `update` resolves against a remote-tracking ref so that a fetch is
    // visible without moving the branch scoop owns.
    let f = Fixture::new();
    let upstream = f.bucket("upstream");
    f.commit(&upstream, "tool.json", "1.0.0", "v100");

    let clone_dir = f.scoop_root().join("buckets").join("main");
    git(
        f.home.path(),
        &["clone", "-q", &format!("file://{}", upstream.display()), &clone_dir.to_string_lossy()],
    );

    let tip = bucket::tip(&clone_dir);
    assert!(
        tip.rev.starts_with("origin/"),
        "a cloned bucket has an upstream and must resolve against it: {tip:?}"
    );
    assert_eq!(tip.stale, None, "an upstream ref is not stale");
}

#[test]
fn a_bucket_with_no_upstream_falls_back_to_head_and_says_why() {
    // A bucket created locally, or one whose remote was removed. Resolving is
    // still possible; claiming it is "latest" is not.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    let tip = bucket::tip(&dir);
    assert_eq!(tip.rev, "HEAD");
    let why = tip.stale.expect("falling back must be explained, not silent");
    assert!(
        why.contains("upstream"),
        "name what is missing: {why}"
    );
}

#[test]
fn a_shallow_clone_is_detected() {
    // Measured: adopt's walk on a shallow clone finds nothing and git says
    // nothing about why, which is indistinguishable from "this version was
    // never in this bucket".
    let f = Fixture::new();
    let upstream = f.bucket("upstream");
    f.commit(&upstream, "tool.json", "1.0.0", "v100");
    f.commit(&upstream, "tool.json", "1.0.1", "v101");

    let full = f.home.path().join("full");
    let shallow = f.home.path().join("shallow");
    let url = format!("file://{}", upstream.display());
    git(f.home.path(), &["clone", "-q", &url, &full.to_string_lossy()]);
    git(f.home.path(), &["clone", "-q", "--depth", "1", &url, &shallow.to_string_lossy()]);

    assert!(bucket::is_shallow(&shallow), "a --depth 1 clone is shallow");
    assert!(
        !bucket::is_shallow(&full),
        "a full clone must not be reported as shallow -- otherwise every adopt \
         failure blames shallowness"
    );
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test --no-fail-fast --test bucket 2>&1 | tail -20`
Expected: FAIL to compile — `unresolved import 'dotpkg::bucket'`.

- [ ] **Step 4: Create `src/bucket.rs`**

```rust
//! Every git invocation in the crate.
//!
//! Until Phase 3 these lived inline in `backend::scoop`, which was fine while
//! `stage` was the only caller. `update` and `adopt` are two more, and a
//! third copy of "run git, decide what its silence meant" is how the three
//! drift apart.
//!
//! Nothing here writes to a bucket's working tree or moves a branch. A bucket
//! is scoop's directory; dotpkg reads it and fetches into it, and that is all.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// True when git exited 0. Used where the question is yes/no and the output
/// does not matter.
pub fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `Ok(None)` when the path is absent from that revision; `Err` only when git
/// itself could not be run.
pub fn git_show(dir: &Path, rev: &str, path_in_repo: &str) -> Result<Option<String>> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["show", &format!("{rev}:{path_in_repo}")])
        .output()
        .with_context(|| format!("cannot run git in {}", dir.display()))?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
}

/// The bucket's own filename for this app, found case-insensitively, **at the
/// given revision**.
///
/// Measured: `git ls-tree` at an old commit returns that commit's spelling
/// (`bucket/Tool.json`) while HEAD has another (`bucket/tool.json`). Listing
/// HEAD instead would miss a historical name -- which is what the Phase 2b-1
/// rehearsal script did.
pub fn resolve_spelling(dir: &Path, rev: &str, app_key: &str) -> Option<String> {
    let listing = Command::new("git")
        .current_dir(dir)
        .args(["ls-tree", "--name-only", rev, "bucket/"])
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

/// Measured: `adopt`'s walk over a shallow clone finds nothing, and git prints
/// nothing to distinguish that from "this version was never here". `scoop
/// bucket add` clones in full, but a bucket the user cloned by hand is not
/// covered by that measurement.
pub fn is_shallow(dir: &Path) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--is-shallow-repository"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// The revision resolution reads from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tip {
    /// `origin/main`, or `HEAD` when there is no upstream.
    pub rev: String,
    /// Why this is not a remote-tracking ref, when it is not. `None` means the
    /// answer is as current as the last fetch made it.
    pub stale: Option<String>,
}

/// Where to resolve "latest" from, without moving anything.
///
/// The upstream of the bucket's current branch, so a `fetch` is visible
/// without a `pull`: the fetched objects are reachable from `refs/remotes/`,
/// which is all `git show` needs at `apply` time.
pub fn tip(dir: &Path) -> Tip {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--abbrev-ref", "@{u}"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let rev = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if rev.is_empty() {
                Tip { rev: "HEAD".into(), stale: Some("the bucket's branch names no upstream".into()) }
            } else {
                Tip { rev, stale: None }
            }
        }
        _ => Tip {
            rev: "HEAD".into(),
            stale: Some("the bucket's branch has no upstream to fetch from".into()),
        },
    }
}

/// `git fetch`. Never `pull`, never a checkout: the branch and working tree
/// belong to scoop.
pub fn fetch(dir: &Path) -> Result<()> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["fetch", "--quiet"])
        .output()
        .with_context(|| format!("cannot run git fetch in {}", dir.display()))?;
    anyhow::ensure!(
        out.status.success(),
        "git fetch failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}
```

- [ ] **Step 5: Delete the three helpers from `backend/scoop.rs` and call `bucket::` instead**

Remove `fn git_ok`, `fn git_show`, `fn resolve_spelling` from `src/backend/scoop.rs`. In `Scoop::stage`, replace the call sites:

- `git_ok(&bucket_dir, &["cat-file", ...])` → `crate::bucket::git_ok(...)`
- `git_show(&bucket_dir, commit, &in_repo)?` → `crate::bucket::git_show(&bucket_dir, commit, &in_repo)?`
- `resolve_spelling(&bucket_dir, commit, app.key())` → `crate::bucket::resolve_spelling(...)`

The `every_scoop_argv_is_built_by_a_named_function` test counts `.args([` occurrences in `scoop.rs` and expects **2** (the two git sites). Both move out, so that expectation becomes **0**. Update the constant and its comment:

```rust
        assert_eq!(
            inline, 0,
            "every git argv moved to src/bucket.rs in Phase 3, so scoop.rs now \
             has none; build every SCOOP argv in a *_argv function so the argv \
             tests cover it"
        );
```

- [ ] **Step 6: Declare the module**

In `src/lib.rs`, add `pub mod bucket;` in alphabetical position (after `pub mod backend;`).

- [ ] **Step 7: Run everything**

Run: `cargo test --no-fail-fast 2>&1 | grep -E "^test result:"`
Expected: all `ok`. `tests/prepare.rs`'s 22 tests are the proof the move changed no behaviour.

- [ ] **Step 8: Run the negative control**

Make `is_shallow` return `true` unconditionally and confirm `a_shallow_clone_is_detected` goes red on the *full clone* assertion — the half that a "detect shallowness" implementation is most likely to get wrong in the direction that blames the wrong thing. Restore. Then make `tip` always return `Tip { rev: "HEAD".into(), stale: None }` and confirm both tip tests go red.

- [ ] **Step 9: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Move every git invocation into src/bucket.rs, and add the three probes Phase 3 needs

git_ok, git_show and resolve_spelling were inline in backend/scoop.rs, which
was fine while stage() was the only caller. update and adopt are two more.
Behaviour is unchanged and tests/prepare.rs's 22 tests are the proof.

Three new read-only probes, each answering a question the measurements raised:

- tip() resolves against the upstream of the bucket's branch, so a fetch is
  visible without a pull. The fetched objects are reachable from refs/remotes/,
  which is all git show needs at apply time. No upstream falls back to HEAD and
  carries the reason, because resolving is still possible but calling the
  result "latest" is not.
- fetch() fetches and never pulls or checks out. A bucket is scoop's directory.
- is_shallow() exists because a shallow clone makes adopt's walk find nothing
  while git says nothing about why -- indistinguishable from "this version was
  never in this bucket".

tests/common/mod.rs builds real repositories in the measured shapes, including
the case-different filename, which is built with plumbing and never checked
out: git mv cannot make it on macOS or Windows, and the first probe run
measured nothing because it tried.

Negative controls: is_shallow returning true unconditionally leaves
a_shallow_clone_is_detected red on the full-clone half; tip() hardcoded to HEAD
leaves both tip tests red.
EOF
```

---

## Task 4: `bucket::resolve_latest` — what `update` asks

**Files:**
- Modify: `src/bucket.rs`
- Test: `tests/bucket.rs`

**Interfaces:**
- Consumes: `bucket::{tip, git_show, resolve_spelling}` (Task 3).
- Produces:

```rust
pub struct Latest {
    pub commit: String,
    pub version: String,
    pub path_in_repo: String,
    /// True when `git log -1` disagreed with the tip's blob and the tip was
    /// recorded instead.
    pub fell_back_to_tip: bool,
}
pub fn manifest_path(dir: &Path, app: &Name, rev: &str) -> Option<String>;
pub fn resolve_latest(dir: &Path, app: &Name, rev: &str) -> Result<Option<Latest>>;
```

- [ ] **Step 1: Write the failing tests**

Append to `tests/bucket.rs`:

```rust
use dotpkg::model::Name;

#[test]
fn latest_is_the_per_file_commit_not_the_bucket_tip() {
    // Measured section A. The whole reason pkg.lock records a commit per
    // package rather than one commit per bucket.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "fzf.json", "1.0.0", "v100");
    let want = f.commit(&dir, "fzf.json", "1.0.2", "v102");
    f.commit(&dir, "bat.json", "9.9.9", "bat");
    let tip_sha = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();

    let got = bucket::resolve_latest(&dir, &Name::new("fzf"), "HEAD")
        .unwrap()
        .expect("fzf is in this bucket");

    assert_eq!(got.commit, want, "the commit that last touched fzf.json");
    assert_ne!(got.commit, tip_sha, "not the bucket tip -- bat moved it on");
    assert_eq!(got.version, "1.0.2");
    assert_eq!(got.path_in_repo, "bucket/fzf.json");
    assert!(!got.fell_back_to_tip);
}

#[test]
fn latest_does_not_name_a_merge_commit() {
    // Measured section B'. Under --full-history this returns the MERGE, whose
    // blob is identical but which is not the commit that produced the version.
    // update must not have that flag; adopt must.
    let f = Fixture::new();
    let (_side, main_102) = merged_bucket(&f, "main");
    let dir = f.bucket_dir("main");

    let got = bucket::resolve_latest(&dir, &Name::new("tool"), "HEAD")
        .unwrap()
        .unwrap();

    assert_eq!(got.version, "1.0.2");
    assert_eq!(
        got.commit, main_102,
        "the commit that made 1.0.2, not the merge that carried it"
    );
}

#[test]
fn the_recorded_commit_always_carries_the_tips_content() {
    // The self-check that makes the property true by construction rather than
    // by trusting git log's history simplification. A shallow clone is the
    // shape that exercises it: there is exactly one commit, and it is the tip.
    let f = Fixture::new();
    let upstream = f.bucket("upstream");
    f.commit(&upstream, "tool.json", "1.0.0", "v100");
    f.commit(&upstream, "tool.json", "1.0.1", "v101");
    let shallow = f.home.path().join("shallow");
    git(
        f.home.path(),
        &["clone", "-q", "--depth", "1", &format!("file://{}", upstream.display()),
          &shallow.to_string_lossy()],
    );

    let got = bucket::resolve_latest(&shallow, &Name::new("tool"), "HEAD")
        .unwrap()
        .unwrap();
    assert_eq!(got.version, "1.0.1", "the tip's content, which is what latest means");
    assert_eq!(
        bucket::git_show(&shallow, &got.commit, &got.path_in_repo).unwrap(),
        bucket::git_show(&shallow, "HEAD", &got.path_in_repo).unwrap(),
        "the recorded commit's blob must equal the tip's"
    );
}

#[test]
fn an_app_the_bucket_does_not_have_resolves_to_none_rather_than_erroring() {
    // "not in this bucket" is an ordinary answer during a bucket search, not a
    // failure: update tries every declared bucket before giving up.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    assert_eq!(
        bucket::resolve_latest(&dir, &Name::new("nosuch"), "HEAD").unwrap(),
        None
    );
}

#[test]
fn latest_finds_a_manifest_the_bucket_spells_with_different_case() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "MixedCase.json", "1.0.0", "v100");

    let got = bucket::resolve_latest(&dir, &Name::new("MIXEDCASE"), "HEAD")
        .unwrap()
        .unwrap();
    assert_eq!(got.path_in_repo, "bucket/MixedCase.json");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --no-fail-fast --test bucket 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function 'resolve_latest' in module 'bucket'`.

- [ ] **Step 3: Implement**

Append to `src/bucket.rs`:

```rust
use crate::model::Name;

/// The bucket's own path for this app at `rev`, trying the cheap guesses
/// before paying for a tree listing. Mirrors `Scoop::stage`'s chain exactly,
/// so `update` records a path `stage` will later find.
pub fn manifest_path(dir: &Path, app: &Name, rev: &str) -> Option<String> {
    let mut tried: Vec<String> = Vec::new();
    for spelling in [app.to_string(), app.key().to_string()] {
        if tried.contains(&spelling) {
            continue;
        }
        tried.push(spelling.clone());
        let candidate = format!("bucket/{spelling}.json");
        if matches!(git_show(dir, rev, &candidate), Ok(Some(_))) {
            return Some(candidate);
        }
    }
    resolve_spelling(dir, rev, app.key()).map(|real| format!("bucket/{real}"))
}

/// What `update` records for one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latest {
    pub commit: String,
    pub version: String,
    pub path_in_repo: String,
    /// `git log -1` named a commit whose blob is not the tip's, so the tip was
    /// recorded instead.
    pub fell_back_to_tip: bool,
}

/// Resolve "latest" for one app at `rev`. `Ok(None)` means this bucket does
/// not have the app, which is an ordinary answer during a bucket search.
///
/// **Deliberately without `--full-history`.** Measured: that flag makes this
/// return the merge commit that carried a version rather than the one that
/// produced it. `adopt` needs the flag and this does not.
///
/// The blob comparison against `rev` is what makes "the recorded commit
/// carries the tip's content for this file" true by construction rather than
/// by trusting git's history simplification. It costs one extra `git show`.
pub fn resolve_latest(dir: &Path, app: &Name, rev: &str) -> Result<Option<Latest>> {
    let Some(path_in_repo) = manifest_path(dir, app, rev) else {
        return Ok(None);
    };
    let Some(tip_text) = git_show(dir, rev, &path_in_repo)? else {
        return Ok(None);
    };

    let out = Command::new("git")
        .current_dir(dir)
        .args(["log", "-1", "--format=%H", rev, "--", &path_in_repo])
        .output()
        .with_context(|| format!("cannot run git log in {}", dir.display()))?;
    let per_file = String::from_utf8_lossy(&out.stdout).trim().to_string();

    let (commit, fell_back_to_tip) = match git_show(dir, &per_file, &path_in_repo) {
        Ok(Some(t)) if !per_file.is_empty() && t == tip_text => (per_file, false),
        _ => {
            let sha = Command::new("git")
                .current_dir(dir)
                .args(["rev-parse", rev])
                .output()
                .with_context(|| format!("cannot run git rev-parse in {}", dir.display()))?;
            (String::from_utf8_lossy(&sha.stdout).trim().to_string(), true)
        }
    };

    let parsed: serde_json::Value = serde_json::from_str(&tip_text)
        .with_context(|| format!("{app}: {path_in_repo} at {rev} is not valid JSON"))?;
    let version = parsed
        .get("version")
        .and_then(|v| v.as_str())
        .with_context(|| format!("{app}: {path_in_repo} at {rev} declares no version"))?
        .to_string();

    Ok(Some(Latest { commit, version, path_in_repo, fell_back_to_tip }))
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --no-fail-fast --test bucket 2>&1 | grep -E "^test result:"`
Expected: `ok`, 8 passed.

- [ ] **Step 5: Run the negative controls**

Three, each aimed at a mutation the tests must be able to catch:

1. Add `"--full-history"` to the `git log` argv. Confirm `latest_does_not_name_a_merge_commit` goes red naming the merge sha and that `latest_is_the_per_file_commit_not_the_bucket_tip` stays green — a single fixture cannot tell those two apart.
2. Delete the blob comparison, always taking `per_file`. Confirm `the_recorded_commit_always_carries_the_tips_content` goes red.
3. Make `manifest_path` skip the `resolve_spelling` fallback. Confirm `latest_finds_a_manifest_the_bucket_spells_with_different_case` goes red and the others stay green.

Record all three assertions in the commit message.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Resolve latest per file, and check the answer against the tip

update's half of the resolver. Plain `git log -1`, deliberately without
--full-history: measured, that flag returns the merge commit that carried a
version rather than the one that produced it, which is right for adopt and
wrong here.

The extra `git show` comparing the per-file commit's blob against the tip's is
what makes "the recorded commit carries the tip's content for this file" true
by construction instead of by trusting git's history simplification. It is also
what makes a shallow clone degrade correctly rather than silently: there is one
commit, it is the tip, and the recorded pin is still right.

An app a bucket does not have is Ok(None), not an error -- update tries every
declared bucket before giving up.

Negative controls, all three run:
- adding --full-history leaves latest_does_not_name_a_merge_commit red naming
  the merge sha, with the linear fixture still green
- deleting the blob comparison leaves
  the_recorded_commit_always_carries_the_tips_content red
- dropping the resolve_spelling fallback leaves the mixed-case test red alone
EOF
```

---

## Task 5: `bucket::history` and `bucket::blobs` — what `adopt` asks

**Files:**
- Modify: `src/bucket.rs`
- Test: `tests/bucket.rs`

**Interfaces:**
- Consumes: Task 3's module.
- Produces:
  - `bucket::history(dir: &Path, path_in_repo: &str, rev: &str) -> Result<Vec<String>>`
  - `bucket::blobs(dir: &Path, commits: &[String], path_in_repo: &str) -> Result<Vec<Option<Vec<u8>>>>`

- [ ] **Step 1: Write the failing tests**

Append to `tests/bucket.rs`:

```rust
#[test]
fn history_sees_a_version_that_only_a_merged_branch_ever_had() {
    // Measured section B, and the single reason adopt carries --full-history.
    // Without it a version a user genuinely has installed is unreachable, and
    // adopt would report "not in this bucket" about a commit that is a real
    // ancestor of HEAD.
    let f = Fixture::new();
    let (side_101, _main) = merged_bucket(&f, "main");
    let dir = f.bucket_dir("main");

    let commits = bucket::history(&dir, "bucket/tool.json", "HEAD").unwrap();
    assert!(
        commits.contains(&side_101),
        "the side branch's 1.0.1 commit must be reachable: {commits:?}"
    );

    // The control this test needs to mean anything: the DEFAULT walk cannot
    // see it, so a `history` that quietly dropped the flag would be caught.
    let plain = git(&dir, &["log", "--format=%H", "--", "bucket/tool.json"]);
    assert!(
        !plain.contains(&side_101),
        "if the plain walk also saw it, this fixture stopped reproducing the \
         shape it exists for"
    );
}

#[test]
fn blobs_reads_a_whole_history_in_one_process_and_keeps_the_order() {
    // Measured: 395 processes and 3.16s the naive way, 2 processes and 0.02s
    // this way, identical answer. The count is what transfers to Windows.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let c1 = f.commit(&dir, "tool.json", "1.0.0", "v100");
    let c2 = f.commit(&dir, "tool.json", "1.0.1", "v101");
    let c3 = f.commit(&dir, "tool.json", "1.0.2", "v102");

    let commits = vec![c3.clone(), c2.clone(), c1.clone()];
    let got = bucket::blobs(&dir, &commits, "bucket/tool.json").unwrap();

    assert_eq!(got.len(), 3, "one answer per commit, in order");
    for (i, want) in ["1.0.2", "1.0.1", "1.0.0"].iter().enumerate() {
        let body = got[i].as_ref().unwrap_or_else(|| panic!("blob {i} missing"));
        assert!(
            String::from_utf8_lossy(body).contains(want),
            "position {i} must belong to commit {}: got {}",
            commits[i],
            String::from_utf8_lossy(body)
        );
    }
}

#[test]
fn a_commit_where_the_path_is_absent_yields_none_and_does_not_shift_the_rest() {
    // `git cat-file --batch` answers a missing object with a one-line
    // "<spec> missing" and no body. A parser that assumed every request has a
    // body would consume the NEXT blob's bytes as this one's and mis-attribute
    // every commit after it -- silently, since the bytes still parse as JSON.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let before = f.commit(&dir, "other.json", "0.0.1", "other");
    let after = f.commit(&dir, "tool.json", "1.0.0", "v100");

    let got = bucket::blobs(
        &dir,
        &[before.clone(), after.clone()],
        "bucket/tool.json",
    )
    .unwrap();

    assert_eq!(got.len(), 2);
    assert!(got[0].is_none(), "tool.json did not exist at {before}");
    let body = got[1].as_ref().expect("tool.json exists at the later commit");
    assert!(
        String::from_utf8_lossy(body).contains("1.0.0"),
        "the second answer must not have been shifted: {}",
        String::from_utf8_lossy(body)
    );
}

#[test]
fn blobs_returns_bytes_not_a_string_because_line_endings_are_the_evidence() {
    // adopt compares these against an installed manifest under
    // verify::normalise. A String round trip through lossy UTF-8 would be
    // lossless for JSON but the signature would invite someone to trim.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let c = f.commit(&dir, "tool.json", "1.0.0", "v100");
    let got = bucket::blobs(&dir, std::slice::from_ref(&c), "bucket/tool.json").unwrap();
    let body = got[0].as_ref().unwrap();
    assert_eq!(
        body.as_slice(),
        f.blob(&dir, &c, "tool.json").as_bytes(),
        "the blob must come back byte for byte, trailing newline included"
    );
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --no-fail-fast --test bucket 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function 'history'`.

- [ ] **Step 3: Implement**

Append to `src/bucket.rs`:

```rust
use std::io::Write;

/// Every commit touching `path_in_repo`, newest first.
///
/// **`--full-history`, deliberately.** Measured: default history
/// simplification follows one TREESAME parent through a merge, so a version
/// that reached the bucket only on a branch whose change was superseded is
/// invisible -- and `adopt` would report "not in this bucket" about a commit
/// that is a genuine ancestor of HEAD.
///
/// This is the opposite choice from `resolve_latest`, and both are right: see
/// `docs/measurements-2026-08-09-git-resolution.md`, sections B and B'.
pub fn history(dir: &Path, path_in_repo: &str, rev: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["log", "--full-history", "--format=%H", rev, "--", path_in_repo])
        .output()
        .with_context(|| format!("cannot run git log in {}", dir.display()))?;
    anyhow::ensure!(
        out.status.success(),
        "git log failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// `<commit>:<path_in_repo>` for every commit, in **one** process.
///
/// Measured on a 400-commit history with the match near the bottom: 2
/// processes and 0.02 s against 395 processes and 3.16 s, identical answer.
/// The ratio is from a synthetic repository; the process count is what
/// transfers, and it transfers to Windows.
///
/// `git cat-file --batch` writes `<sha> <type> <size>\n<contents>\n` per
/// request, in order -- except for a request it cannot resolve, which gets a
/// single `<spec> missing\n` line and **no body**. Keying on the header shape
/// rather than assuming one body per request is what stops a missing path from
/// shifting every later answer onto the wrong commit.
pub fn blobs(dir: &Path, commits: &[String], path_in_repo: &str) -> Result<Vec<Option<Vec<u8>>>> {
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(["cat-file", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("cannot run git cat-file in {}", dir.display()))?;
    {
        let stdin = child.stdin.as_mut().expect("stdin was piped");
        for c in commits {
            writeln!(stdin, "{c}:{path_in_repo}")
                .with_context(|| format!("cannot feed git cat-file in {}", dir.display()))?;
        }
    }
    let out = child
        .wait_with_output()
        .with_context(|| format!("cannot read git cat-file in {}", dir.display()))?;

    let data = out.stdout;
    let mut answers = Vec::with_capacity(commits.len());
    let mut i = 0usize;
    for _ in commits {
        let Some(nl) = data[i..].iter().position(|b| *b == b'\n').map(|p| i + p) else {
            answers.push(None);
            continue;
        };
        let header = String::from_utf8_lossy(&data[i..nl]).into_owned();
        let fields: Vec<&str> = header.split_whitespace().collect();
        // "<spec> missing" -- two fields, no body. Anything that is not a
        // three-field "<sha> <type> <size>" header is treated the same way:
        // no body to consume, so the next answer starts on the next line.
        let size = match fields.as_slice() {
            [_, _, size] => size.parse::<usize>().ok(),
            _ => None,
        };
        match size {
            Some(n) if nl + 1 + n <= data.len() => {
                answers.push(Some(data[nl + 1..nl + 1 + n].to_vec()));
                // The body is followed by a bare newline git adds itself.
                i = nl + 1 + n + 1;
            }
            _ => {
                answers.push(None);
                i = nl + 1;
            }
        }
    }
    Ok(answers)
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --no-fail-fast --test bucket 2>&1 | grep -E "^test result:"`
Expected: `ok`, 12 passed.

- [ ] **Step 5: Run the negative controls**

1. Drop `"--full-history"` from `history`. Confirm `history_sees_a_version_that_only_a_merged_branch_ever_had` goes red on the `contains(&side_101)` assertion.
2. In `blobs`, make the missing-object branch consume a body anyway (`i = nl + 1 + 1`). Confirm `a_commit_where_the_path_is_absent_yields_none_and_does_not_shift_the_rest` goes red on the *second* assertion, not the first — the shift is the defect, and a test that only checked `got[0].is_none()` would miss it.
3. Make `blobs` return the answers reversed. Confirm `blobs_reads_a_whole_history_in_one_process_and_keeps_the_order` goes red naming the position.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Walk a manifest's whole history, and read it in one process

adopt's half of the resolver, and the opposite git flag from update's.

history() carries --full-history because measured, default simplification
follows one TREESAME parent through a merge: a version that reached the bucket
only on a branch whose change was superseded is invisible, and adopt would say
"not in this bucket" about a commit that is a genuine ancestor of HEAD. The
test asserts the plain walk CANNOT see it, so the fixture cannot quietly stop
reproducing the shape it exists for.

blobs() feeds the whole history to one `git cat-file --batch`. Measured on 400
commits with the match at position 394: 2 processes and 0.02s against 395 and
3.16s, identical answer. The ratio is synthetic; the process count is what
transfers to Windows.

The parser keys on the header shape rather than assuming one body per request.
A path absent from a commit gets "<spec> missing" and no body, and consuming a
body there would attribute every later blob to the wrong commit -- silently,
because the bytes still parse as JSON.

Negative controls, all three run:
- dropping --full-history leaves the merge test red on contains(side_101)
- consuming a body for a missing object leaves the absent-path test red on its
  SECOND assertion, which is the shift, not the None
- reversing the answers leaves the ordering test red naming the position
EOF
```

---

## Task 6: `[scoop.opts] bucket`, and choosing a bucket

**Files:**
- Modify: `src/config.rs` (`PkgOpts`)
- Create: the `choose_bucket` function in `src/bucket.rs`
- Test: `src/config.rs` unit tests, `tests/bucket.rs`

**Interfaces:**
- Consumes: `bucket::manifest_path` (Task 4), `config::{Config, PkgOpts}`.
- Produces:

```rust
pub enum BucketChoice {
    Chosen { name: Name, dir: PathBuf, tip: Tip },
    Ambiguous { candidates: Vec<Name> },
    NotFound { searched: Vec<Name> },
}
pub fn choose_bucket(
    scoop_root: &Path,
    declared: &Config,
    app: &Name,
    already_locked: Option<&str>,
) -> BucketChoice;
```

- [ ] **Step 1: Write the failing config test**

Add to `src/config.rs`'s test module:

```rust
    #[test]
    fn a_package_can_name_the_bucket_it_comes_from() {
        // The only place this information can live. Two declared buckets can
        // both carry an app, and neither pkg.lock (which does not exist yet
        // for a new package) nor the machine (install.json loses `bucket` for
        // anything dotpkg installed) can answer which one the user meant.
        let cfg = parse(
            "[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n\
             [scoop.opts]\ntool = { bucket = \"extras\" }\n",
        )
        .unwrap();
        assert_eq!(
            cfg.scoop.opts[&Name::new("tool")].bucket.as_deref(),
            Some("extras")
        );
        // arch and bucket are independent, and neither may require the other.
        let cfg = parse("[scoop.opts]\ntool = { arch = \"arm64\" }\n").unwrap();
        assert_eq!(cfg.scoop.opts[&Name::new("tool")].bucket, None);
    }

    #[test]
    fn a_bucket_opt_that_could_leave_its_directory_is_refused_at_parse_time() {
        // Same rule as `[scoop] buckets`: this string becomes
        // `$SCOOP/buckets/<it>` and a git argument.
        for bad in ["../evil", "a/b", "-oops", "", "c:\\x"] {
            let text = format!("[scoop.opts]\ntool = {{ bucket = \"{bad}\" }}\n");
            assert!(parse(&text).is_err(), "{bad:?} must be refused");
        }
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --no-fail-fast --lib config 2>&1 | tail -20`
Expected: FAIL — `unknown field 'bucket'` from `deny_unknown_fields`.

- [ ] **Step 3: Add the field and validate it**

In `src/config.rs`:

```rust
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PkgOpts {
    #[serde(default)]
    pub arch: Option<Arch>,
    /// Which declared bucket this package comes from.
    ///
    /// Needed only when two declared buckets both carry the app. Nothing else
    /// can answer it: a new package has no lock entry, and `install.json`
    /// records `bucket` only for packages dotpkg has never installed.
    #[serde(default)]
    pub bucket: Option<String>,
}
```

`parse` must validate it, since `fold_map` does not look inside values. After the `Config` is built in `parse`, before returning:

```rust
    let cfg = Config { /* as before */ };
    for (name, opts) in &cfg.scoop.opts {
        if let Some(b) = &opts.bucket {
            crate::backend::scoop::ensure_plain_component(
                name,
                "pkg.toml [scoop.opts]",
                "bucket name",
                b,
            )?;
        }
    }
    Ok(cfg)
```

- [ ] **Step 4: Write the failing `choose_bucket` tests**

Append to `tests/bucket.rs`:

```rust
use dotpkg::bucket::BucketChoice;

fn cfg(text: &str) -> dotpkg::config::Config {
    dotpkg::config::parse(text).unwrap()
}

#[test]
fn a_package_in_exactly_one_declared_bucket_is_chosen_without_asking() {
    let f = Fixture::new();
    let main = f.bucket("main");
    f.commit(&main, "tool.json", "1.0.0", "v100");
    let extras = f.bucket("extras");
    f.commit(&extras, "other.json", "1.0.0", "v100");

    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Name::new("tool"),
        None,
    );
    match choice {
        BucketChoice::Chosen { name, .. } => assert_eq!(name, Name::new("main")),
        other => panic!("expected a clean choice, got {other:?}"),
    }
}

#[test]
fn a_package_in_two_declared_buckets_refuses_and_names_both() {
    // Never a guess based on declaration order: reordering `buckets` would
    // silently move a pin.
    let f = Fixture::new();
    for b in ["main", "extras"] {
        let dir = f.bucket(b);
        f.commit(&dir, "tool.json", "1.0.0", "v100");
    }

    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Name::new("tool"),
        None,
    );
    match choice {
        BucketChoice::Ambiguous { candidates } => {
            assert!(candidates.contains(&Name::new("main")), "{candidates:?}");
            assert!(candidates.contains(&Name::new("extras")), "{candidates:?}");
        }
        other => panic!("two buckets have it; guessing is forbidden: {other:?}"),
    }
}

#[test]
fn scoop_opts_bucket_settles_an_ambiguity() {
    let f = Fixture::new();
    for b in ["main", "extras"] {
        let dir = f.bucket(b);
        f.commit(&dir, "tool.json", "1.0.0", "v100");
    }

    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n\
              [scoop.opts]\ntool = { bucket = \"extras\" }\n"),
        &Name::new("tool"),
        None,
    );
    match choice {
        BucketChoice::Chosen { name, .. } => assert_eq!(name, Name::new("extras")),
        other => panic!("the opt must settle it: {other:?}"),
    }
}

#[test]
fn an_existing_lock_entry_pins_the_bucket_so_update_never_moves_a_package() {
    // update re-resolves a version, not a provenance. A package silently
    // migrating from extras to main would change its url and hash with no line
    // in the diff saying so.
    let f = Fixture::new();
    for b in ["main", "extras"] {
        let dir = f.bucket(b);
        f.commit(&dir, "tool.json", "1.0.0", "v100");
    }

    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Name::new("tool"),
        Some("extras"),
    );
    match choice {
        BucketChoice::Chosen { name, .. } => assert_eq!(name, Name::new("extras")),
        other => panic!("the lock's bucket wins: {other:?}"),
    }
}

#[test]
fn a_package_no_declared_bucket_has_names_what_was_searched() {
    let f = Fixture::new();
    f.bucket("main");
    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &Name::new("tool"),
        None,
    );
    match choice {
        BucketChoice::NotFound { searched } => assert_eq!(searched, vec![Name::new("main")]),
        other => panic!("{other:?}"),
    }
}
```

- [ ] **Step 5: Implement `choose_bucket`**

Append to `src/bucket.rs`:

```rust
/// Which bucket a package comes from, or why that cannot be decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BucketChoice {
    Chosen { name: Name, dir: std::path::PathBuf, tip: Tip },
    /// More than one declared bucket carries it. Never resolved by declaration
    /// order: reordering `buckets` would silently move a pin.
    Ambiguous { candidates: Vec<Name> },
    NotFound { searched: Vec<Name> },
}

/// Decide which declared bucket a package comes from.
///
/// Precedence, strongest first: the existing lock entry (so `update`
/// re-resolves a version and never a provenance), then `[scoop.opts] <pkg>
/// = { bucket = "..." }`, then a search of every declared bucket.
pub fn choose_bucket(
    scoop_root: &Path,
    declared: &crate::config::Config,
    app: &Name,
    already_locked: Option<&str>,
) -> BucketChoice {
    let open = |name: &Name| -> BucketChoice {
        let dir = scoop_root.join("buckets").join(name.key());
        BucketChoice::Chosen { name: name.clone(), tip: tip(&dir), dir }
    };

    let declared_names: Vec<Name> = declared.scoop.buckets.iter().map(|b| b.name.clone()).collect();

    for stated in [
        already_locked.map(Name::new),
        declared.scoop.opts.get(app).and_then(|o| o.bucket.as_deref()).map(Name::new),
    ]
    .into_iter()
    .flatten()
    {
        if declared_names.contains(&stated) {
            return open(&stated);
        }
        return BucketChoice::NotFound { searched: vec![stated] };
    }

    let mut found = Vec::new();
    for name in &declared_names {
        let dir = scoop_root.join("buckets").join(name.key());
        if !dir.join(".git").exists() {
            continue;
        }
        let at = tip(&dir);
        if manifest_path(&dir, app, &at.rev).is_some() {
            found.push(name.clone());
        }
    }
    match found.len() {
        0 => BucketChoice::NotFound { searched: declared_names },
        1 => open(&found[0]),
        _ => BucketChoice::Ambiguous { candidates: found },
    }
}
```

- [ ] **Step 6: Run everything**

Run: `cargo test --no-fail-fast 2>&1 | grep -E "^test result:"`
Expected: all `ok`.

- [ ] **Step 7: Run the negative controls**

1. Make the ambiguous branch return `open(&found[0])` instead. Confirm `a_package_in_two_declared_buckets_refuses_and_names_both` goes red and `a_package_in_exactly_one_declared_bucket_is_chosen_without_asking` stays green.
2. Remove the `already_locked` arm. Confirm `an_existing_lock_entry_pins_the_bucket_so_update_never_moves_a_package` goes red — it must, because with `main` first in declaration order the fixture would otherwise choose `main` and look like a plain ambiguity refusal.
3. Delete the `ensure_plain_component` loop in `config::parse`. Confirm `a_bucket_opt_that_could_leave_its_directory_is_refused_at_parse_time` goes red.

- [ ] **Step 8: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Let a package name its bucket, and refuse to guess when two have it

Two declared buckets can both carry an app, and nothing else on the machine can
say which the user meant: a new package has no lock entry, and install.json
records `bucket` only for packages dotpkg has never installed -- measured, it
records `url` and `architecture` instead for everything dotpkg touches.

So `[scoop.opts] <pkg> = { bucket = "..." }`, validated with the same
ensure_plain_component as `[scoop] buckets` because it becomes
$SCOOP/buckets/<it> and a git argument.

choose_bucket's precedence is the lock, then that opt, then a search. The lock
first is what stops update re-resolving a provenance as well as a version: a
package migrating from extras to main would change its url and hash with no
line in the diff saying so.

Ambiguity refuses and names every candidate rather than taking the first
declared. Declaration order deciding a pin means reordering a list in pkg.toml
silently changes what gets installed.

Negative controls, all three run:
- returning found[0] on ambiguity leaves the two-bucket test red with the
  one-bucket test green
- removing the already_locked arm leaves the lock-pins-the-bucket test red,
  which it must: `main` is first in declaration order, so without it the
  fixture would look like an ordinary ambiguity
- deleting the parse-time validation leaves the path-component test red
EOF
```

---

## Task 7: Write `pkg.lock`

**Files:**
- Modify: `src/lock.rs`
- Test: `src/lock.rs` unit tests

**Interfaces:**
- Consumes: `lock::{Lock, Pin}`, `apply::lock_coherence_guard`.
- Produces: `lock::render(lock: &Lock) -> String`, `lock::save(lock: &Lock, path: &Path) -> Result<()>`.

- [ ] **Step 1: Write the failing tests**

Add to `src/lock.rs`'s test module:

```rust
    #[test]
    fn a_written_lock_reads_back_as_the_same_lock() {
        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("fzf"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "a".repeat(40),
                version: "0.74.2".into(),
            },
        );
        lock.winget.insert(
            Name::new("Git.Git"),
            Pin::WingetVersion { version: "2.55.0".into() },
        );

        let text = render(&lock);
        assert_eq!(parse(&text).unwrap(), lock, "round trip: {text}");
        // The documented shape, not merely something that round-trips.
        assert!(text.contains("[scoop.fzf]"), "{text}");
        assert!(text.contains("pin     = \"version-only\""), "{text}");
        // Display case is preserved: `Git.Git` is what a winget user types.
        assert!(text.contains("[winget.\"Git.Git\"]"), "{text}");
    }

    #[test]
    fn a_lock_the_guards_would_reject_is_never_written() {
        // The writer validates its own output with the reader's guard. A Pin
        // that makes `apply` refuse the whole run must not reach disk, because
        // `update` is the command a user runs to FIX that state.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.lock");
        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("fzf"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "main".into(),
                version: "0.74.2".into(),
            },
        );

        let err = save(&lock, &path).unwrap_err();
        assert!(format!("{err:#}").contains("hex"), "got {err:#}");
        assert!(!path.exists(), "nothing may be written for a lock that fails the guard");
    }

    #[test]
    fn a_save_that_replaces_an_existing_lock_keeps_the_previous_one() {
        // pkg.lock is committed. A torn write is a git conflict on top of a
        // broken tool, and the previous pins are the only way back.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.lock");
        let mut first = Lock::default();
        first.scoop.insert(
            Name::new("fzf"),
            Pin::ScoopCommit { bucket: "main".into(), commit: "a".repeat(40), version: "1".into() },
        );
        let mut second = Lock::default();
        second.scoop.insert(
            Name::new("bat"),
            Pin::ScoopCommit { bucket: "main".into(), commit: "b".repeat(40), version: "2".into() },
        );

        save(&first, &path).unwrap();
        save(&second, &path).unwrap();

        assert_eq!(load_or_empty(&path).unwrap(), second);
        let bak = path.with_extension("lock.bak");
        assert!(bak.exists(), "the displaced lock is kept at {bak:?}");
        assert_eq!(load_or_empty(&bak).unwrap(), first);
    }

    #[test]
    fn a_save_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.lock");
        save(&Lock::default(), &path).unwrap();
        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "pkg.lock")
            .collect();
        assert!(strays.is_empty(), "left behind: {strays:?}");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --no-fail-fast --lib lock 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function 'render'`.

- [ ] **Step 3: Implement**

Append to `src/lock.rs`:

```rust
/// Render a lock as the TOML `parse` reads back.
///
/// Hand-written rather than `toml::to_string`: the file is committed, so its
/// diff is read by people, and the aligned `bucket  =` / `commit  =` shape is
/// what `docs/specs/2026-08-08-design.md` documents. A serializer would also
/// have to be taught that `Pin` is two shapes on purpose.
///
/// Keys are quoted, always. `Git.Git` is a bare-key-illegal name in TOML and
/// an unquoted `[winget.Git.Git]` would parse as three nested tables.
pub fn render(lock: &Lock) -> String {
    fn key(n: &Name) -> String {
        format!("{:?}", n.to_string())
    }
    let mut out = String::new();
    for (name, pin) in &lock.scoop {
        let Pin::ScoopCommit { bucket, commit, version } = pin else {
            continue;
        };
        out.push_str(&format!(
            "[scoop.{}]\nbucket  = {:?}\ncommit  = {:?}\nversion = {:?}\n\n",
            key(name),
            bucket,
            commit,
            version
        ));
    }
    for (name, pin) in &lock.winget {
        let Pin::WingetVersion { version } = pin else {
            continue;
        };
        out.push_str(&format!(
            "[winget.{}]\nversion = {:?}\npin     = \"version-only\"\n\n",
            key(name),
            version
        ));
    }
    out
}

/// Write the lock so that an interrupted write cannot destroy the old one, and
/// so that a lock `apply` would refuse never reaches disk.
///
/// The guard runs here rather than only at the call site because `update` is
/// the command a user runs to *fix* an unusable lock: writing another one
/// would leave them with no way forward that does not involve editing TOML by
/// hand. Same temp-then-rename discipline as `State::save`, and for the same
/// reason -- except that `pkg.lock` is committed, so a torn write is also a
/// git conflict.
pub fn save(lock: &Lock, path: &Path) -> Result<()> {
    crate::apply::lock_coherence_guard(lock)
        .context("refusing to write a pkg.lock that `dotpkg apply` would reject")?;

    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
    }
    let text = render(lock);
    let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("pkg.lock");
    let tmp = path.with_file_name(format!("{stem}.tmp{}", std::process::id()));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("cannot create {}", tmp.display()))?;
        f.write_all(text.as_bytes())
            .with_context(|| format!("cannot write {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("cannot flush {}", tmp.display()))?;
    }
    if path.exists() {
        let _ = std::fs::copy(path, path.with_extension("lock.bak"));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::Error::new(e)
            .context(format!("cannot move {} into place at {}", tmp.display(), path.display()))
    })
}
```

`lock.rs` needs `use std::path::Path;` (already present) and `anyhow::Context` (already imported).

- [ ] **Step 4: Run the suite**

Run: `cargo test --no-fail-fast 2>&1 | grep -E "^test result:"`
Expected: all `ok`.

- [ ] **Step 5: Run the negative controls**

1. Delete the `lock_coherence_guard` call in `save`. Confirm `a_lock_the_guards_would_reject_is_never_written` goes red on **both** assertions — the error and the absent file.
2. Make `key()` return the bare `n.to_string()`. Confirm `a_written_lock_reads_back_as_the_same_lock` goes red: `[winget.Git.Git]` parses as nested tables and the round trip loses the entry.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Write pkg.lock, and refuse to write one apply would reject

The writer validates its own output with the reader's guard. update is the
command a user runs to fix an unusable lock, so writing another unusable one
would leave them with no way forward that does not involve hand-editing TOML.

Rendered by hand rather than through toml::to_string: the file is committed, so
its diff is read by people, and Pin is two shapes on purpose. Table keys are
always quoted -- an unquoted [winget.Git.Git] parses as three nested tables and
the entry silently vanishes on the way back in.

Temp-then-rename with the displaced file kept as pkg.lock.bak, the same
discipline as State::save, except that this file is committed so a torn write
is a git conflict on top of a broken tool.

Negative controls: dropping the guard call leaves
a_lock_the_guards_would_reject_is_never_written red on both the message and the
absent file; rendering bare keys leaves the round-trip test red on the winget
entry.
EOF
```

---

## Task 8: `update`'s pure decision layer

No git, no filesystem. This is where the version-versus-commit answer lives.

**Files:**
- Create: `src/update.rs`
- Modify: `src/lib.rs`
- Test: `src/update.rs` unit tests

**Interfaces:**
- Consumes: `lock::{Lock, Pin}`, `model::Name`.
- Produces:

```rust
pub enum Resolution {
    Resolved { bucket: String, commit: String, version: String },
    Failed { why: String },
}
pub enum Change {
    Added { name: Name, version: String },
    VersionChanged { name: Name, from: String, to: String },
    RepinnedSameVersion { name: Name, version: String },
    Unchanged { name: Name },
    Dropped { name: Name, version: String },
    Kept { name: Name, version: String, why: String },
}
pub enum Scope { WholeRun, Named(Vec<Name>) }
pub struct Update { pub lock: Lock, pub changes: Vec<Change> }
pub fn resolve_into_lock(
    old: &Lock,
    declared: &[Name],
    resolutions: &BTreeMap<Name, Resolution>,
    scope: &Scope,
) -> Update;
impl Update { pub fn failed_count(&self) -> usize; pub fn wrote_anything(&self) -> bool; }
```

- [ ] **Step 1: Write the failing tests**

Create `src/update.rs` with only the test module first, so the failure is about the missing functions rather than a missing file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::Pin;

    fn sha(c: char) -> String {
        std::iter::repeat(c).take(40).collect()
    }
    fn locked(bucket: &str, commit: char, version: &str) -> Pin {
        Pin::ScoopCommit { bucket: bucket.into(), commit: sha(commit), version: version.into() }
    }
    fn resolved(bucket: &str, commit: char, version: &str) -> Resolution {
        Resolution::Resolved { bucket: bucket.into(), commit: sha(commit), version: version.into() }
    }
    fn lock_of(entries: &[(&str, Pin)]) -> Lock {
        let mut l = Lock::default();
        for (n, p) in entries {
            l.scoop.insert(Name::new(*n), p.clone());
        }
        l
    }
    fn res(entries: &[(&str, Resolution)]) -> BTreeMap<Name, Resolution> {
        entries.iter().map(|(n, r)| (Name::new(*n), r.clone())).collect()
    }

    #[test]
    fn a_package_with_no_previous_entry_is_added() {
        let u = resolve_into_lock(
            &Lock::default(),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'a', "0.74.2"))]),
            &Scope::WholeRun,
        );
        assert_eq!(u.changes, vec![Change::Added { name: Name::new("fzf"), version: "0.74.2".into() }]);
        assert_eq!(u.lock.scoop.len(), 1);
    }

    #[test]
    fn a_new_version_is_reported_as_a_version_change() {
        let u = resolve_into_lock(
            &lock_of(&[("fzf", locked("main", 'a', "0.74.1"))]),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'b', "0.74.2"))]),
            &Scope::WholeRun,
        );
        assert_eq!(
            u.changes,
            vec![Change::VersionChanged {
                name: Name::new("fzf"),
                from: "0.74.1".into(),
                to: "0.74.2".into()
            }]
        );
    }

    #[test]
    fn the_same_version_at_a_new_commit_is_a_repin_and_says_so() {
        // The answer to "does update converge by version or by commit", in one
        // test. It converges by COMMIT when it writes -- the new commit really
        // is recorded -- and `apply` converges by VERSION when it acts, so
        // this line is the only place a user can see the gap.
        let u = resolve_into_lock(
            &lock_of(&[("fzf", locked("main", 'a', "0.74.1"))]),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'b', "0.74.1"))]),
            &Scope::WholeRun,
        );
        assert_eq!(
            u.changes,
            vec![Change::RepinnedSameVersion { name: Name::new("fzf"), version: "0.74.1".into() }]
        );
        // And the commit really moved. A "report it and keep the old pin"
        // implementation would pass the assertion above and silently make the
        // lock a lie.
        match &u.lock.scoop[&Name::new("fzf")] {
            Pin::ScoopCommit { commit, .. } => assert_eq!(*commit, sha('b')),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_identical_resolution_is_unchanged_and_not_a_repin() {
        let u = resolve_into_lock(
            &lock_of(&[("fzf", locked("main", 'a', "0.74.1"))]),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'a', "0.74.1"))]),
            &Scope::WholeRun,
        );
        assert_eq!(u.changes, vec![Change::Unchanged { name: Name::new("fzf") }]);
    }

    #[test]
    fn a_package_no_longer_declared_is_dropped_on_a_whole_run() {
        let u = resolve_into_lock(
            &lock_of(&[
                ("fzf", locked("main", 'a', "0.74.1")),
                ("aichat", locked("main", 'c', "0.30.0")),
            ]),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'a', "0.74.1"))]),
            &Scope::WholeRun,
        );
        assert!(u.changes.contains(&Change::Dropped {
            name: Name::new("aichat"),
            version: "0.30.0".into()
        }));
        assert!(!u.lock.scoop.contains_key(&Name::new("aichat")));
    }

    #[test]
    fn a_named_run_touches_only_what_it_was_asked_about_and_drops_nothing() {
        // `update fzf` must not rewrite bat's pin, and must not drop a stale
        // aichat entry the user did not mention.
        let old = lock_of(&[
            ("fzf", locked("main", 'a', "0.74.1")),
            ("bat", locked("main", 'c', "0.26.0")),
            ("aichat", locked("main", 'd', "0.30.0")),
        ]);
        let u = resolve_into_lock(
            &old,
            &[Name::new("fzf"), Name::new("bat")],
            &res(&[("fzf", resolved("main", 'b', "0.74.2"))]),
            &Scope::Named(vec![Name::new("fzf")]),
        );
        assert_eq!(u.lock.scoop[&Name::new("bat")], old.scoop[&Name::new("bat")]);
        assert!(
            u.lock.scoop.contains_key(&Name::new("aichat")),
            "a named run drops nothing"
        );
        assert_eq!(u.changes.len(), 1, "only fzf is reported: {:?}", u.changes);
    }

    #[test]
    fn a_failed_reresolve_keeps_the_previous_entry_rather_than_dropping_it() {
        // Dropping it would turn a package that works today into
        // Skip{NotLocked}, which makes the NEXT apply refuse the whole run.
        // The failure is per package; the pin that already worked survives.
        let old = lock_of(&[("zellij", locked("extras", 'a', "0.44.3"))]);
        let u = resolve_into_lock(
            &old,
            &[Name::new("zellij")],
            &res(&[("zellij", Resolution::Failed { why: "bucket \"extras\" has no zellij.json".into() })]),
            &Scope::WholeRun,
        );
        assert_eq!(
            u.lock.scoop[&Name::new("zellij")],
            old.scoop[&Name::new("zellij")],
            "the previous pin must survive a failed re-resolve"
        );
        assert_eq!(
            u.changes,
            vec![Change::Kept {
                name: Name::new("zellij"),
                version: "0.44.3".into(),
                why: "bucket \"extras\" has no zellij.json".into()
            }]
        );
        assert_eq!(u.failed_count(), 1);
    }

    #[test]
    fn a_failed_reresolve_for_a_package_that_had_no_entry_adds_nothing() {
        let u = resolve_into_lock(
            &Lock::default(),
            &[Name::new("new")],
            &res(&[("new", Resolution::Failed { why: "no declared bucket has it".into() })]),
            &Scope::WholeRun,
        );
        assert!(u.lock.scoop.is_empty(), "nothing to keep, so nothing is written");
        assert_eq!(u.failed_count(), 1);
        match &u.changes[0] {
            Change::Kept { why, .. } => assert!(why.contains("no declared bucket")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn winget_entries_survive_a_scoop_update_untouched() {
        // Phase 3 resolves scoop only. Dropping the winget map because this
        // command cannot resolve it would delete pins Phase 4 is going to need.
        let mut old = Lock::default();
        old.winget.insert(Name::new("Git.Git"), Pin::WingetVersion { version: "2.55.0".into() });
        let u = resolve_into_lock(&old, &[], &BTreeMap::new(), &Scope::WholeRun);
        assert_eq!(u.lock.winget, old.winget);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Add `pub mod update;` to `src/lib.rs`, then run:
Run: `cargo test --no-fail-fast --lib update 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find type 'Resolution' in this scope`.

- [ ] **Step 3: Implement**

Prepend to `src/update.rs`, above the test module:

```rust
//! `dotpkg update` — the only command that resolves "latest".
//!
//! This module is the decision, not the plumbing: no git, no filesystem, no
//! network. The driver hands it what the buckets said and it produces the new
//! lock plus the diff a user reads.

use crate::lock::{Lock, Pin};
use crate::model::Name;
use std::collections::BTreeMap;

/// What a bucket said about one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Resolved { bucket: String, commit: String, version: String },
    /// Per package, never fatal to the run.
    Failed { why: String },
}

/// One line of the diff `update` prints.
///
/// `RepinnedSameVersion` is the variant that exists because the answer to
/// "version or commit" is *both, in different places*: `update` records the
/// new commit, and `apply` -- whose decision is `cur.version == want` -- will
/// do nothing about it. This is the only place a user can see that gap, so it
/// is a named variant rather than folded into `Unchanged`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added { name: Name, version: String },
    VersionChanged { name: Name, from: String, to: String },
    RepinnedSameVersion { name: Name, version: String },
    Unchanged { name: Name },
    Dropped { name: Name, version: String },
    /// Re-resolution failed and the previous pin was kept. Dropping it would
    /// turn a working package into `Skip{NotLocked}`, which makes the next
    /// `apply` refuse the whole run.
    Kept { name: Name, version: String, why: String },
}

/// Whether this is `dotpkg update` or `dotpkg update <pkg>...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    WholeRun,
    Named(Vec<Name>),
}

impl Scope {
    fn covers(&self, name: &Name) -> bool {
        match self {
            Scope::WholeRun => true,
            Scope::Named(names) => names.contains(name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub lock: Lock,
    pub changes: Vec<Change>,
}

impl Update {
    pub fn failed_count(&self) -> usize {
        self.changes.iter().filter(|c| matches!(c, Change::Kept { .. })).count()
    }

    /// Whether the new lock differs from the old one at all. `main` uses this
    /// to avoid rewriting a file -- and displacing its `.bak` -- for nothing.
    pub fn wrote_anything(&self) -> bool {
        self.changes
            .iter()
            .any(|c| !matches!(c, Change::Unchanged { .. } | Change::Kept { .. }))
    }
}

/// Fold what the buckets said into a new lock, and say what changed.
///
/// Pure. Every git result arrives as a `Resolution`, which is what lets the
/// whole of `update`'s judgement be tested with no repository at all.
pub fn resolve_into_lock(
    old: &Lock,
    declared: &[Name],
    resolutions: &BTreeMap<Name, Resolution>,
    scope: &Scope,
) -> Update {
    // Phase 3 resolves scoop only. Carrying the winget map through untouched
    // is deliberate: dropping pins this command cannot resolve would delete
    // what Phase 4 needs.
    let mut lock = Lock { scoop: BTreeMap::new(), winget: old.winget.clone() };
    let mut changes = Vec::new();

    for name in declared {
        let previous = old.scoop.get(name);
        if !scope.covers(name) {
            if let Some(p) = previous {
                lock.scoop.insert(name.clone(), p.clone());
            }
            continue;
        }
        match resolutions.get(name) {
            Some(Resolution::Resolved { bucket, commit, version }) => {
                let fresh = Pin::ScoopCommit {
                    bucket: bucket.clone(),
                    commit: commit.clone(),
                    version: version.clone(),
                };
                changes.push(match previous {
                    None => Change::Added { name: name.clone(), version: version.clone() },
                    Some(p) if *p == fresh => Change::Unchanged { name: name.clone() },
                    Some(p) if p.version() != version => Change::VersionChanged {
                        name: name.clone(),
                        from: p.version().to_string(),
                        to: version.clone(),
                    },
                    Some(_) => Change::RepinnedSameVersion {
                        name: name.clone(),
                        version: version.clone(),
                    },
                });
                lock.scoop.insert(name.clone(), fresh);
            }
            Some(Resolution::Failed { why }) => {
                changes.push(Change::Kept {
                    name: name.clone(),
                    version: previous.map(|p| p.version().to_string()).unwrap_or_default(),
                    why: why.clone(),
                });
                if let Some(p) = previous {
                    lock.scoop.insert(name.clone(), p.clone());
                }
            }
            // Not resolved and not failed: the driver never asked about it,
            // which happens for a named run's untouched neighbours. Keep it.
            None => {
                if let Some(p) = previous {
                    lock.scoop.insert(name.clone(), p.clone());
                }
            }
        }
    }

    // Entries for packages pkg.toml no longer declares. Only a whole run drops
    // them: `update fzf` must not quietly delete a stale aichat pin the user
    // did not mention.
    for (name, pin) in &old.scoop {
        if declared.contains(name) {
            continue;
        }
        match scope {
            Scope::WholeRun => changes.push(Change::Dropped {
                name: name.clone(),
                version: pin.version().to_string(),
            }),
            Scope::Named(_) => {
                lock.scoop.insert(name.clone(), pin.clone());
            }
        }
    }

    Update { lock, changes }
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --no-fail-fast --lib update 2>&1 | grep -E "^test result:"`
Expected: `ok`.

- [ ] **Step 5: Run the negative controls**

1. Fold `RepinnedSameVersion` into `Unchanged`. Confirm `the_same_version_at_a_new_commit_is_a_repin_and_says_so` goes red on the *changes* assertion, and that `an_identical_resolution_is_unchanged_and_not_a_repin` stays green — the pair is what makes either meaningful.
2. Make the `Resolution::Failed` arm skip re-inserting `previous`. Confirm `a_failed_reresolve_keeps_the_previous_entry_rather_than_dropping_it` goes red on the pin comparison.
3. Make `Scope::covers` always return `true`. Confirm `a_named_run_touches_only_what_it_was_asked_about_and_drops_nothing` goes red.
4. Drop the `winget: old.winget.clone()` carry-through. Confirm `winget_entries_survive_a_scoop_update_untouched` goes red.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Decide what update writes, with no git in sight

The whole judgement of `update` as a pure function over what the buckets said,
so every branch is testable with no repository at all.

Change::RepinnedSameVersion is the answer to "does update converge by version
or by commit", made into a type. It converges by COMMIT when it writes -- the
new commit really is recorded, and the test asserts that, because a "report it
and keep the old pin" implementation would satisfy the reporting assertion
while making the lock a lie. apply converges by VERSION when it acts. This line
in the diff is the only place a user can see the gap, which is why it is a
named variant rather than folded into Unchanged.

A failed re-resolve keeps the previous entry. Dropping it would turn a package
that works today into Skip{NotLocked}, and that makes the NEXT apply refuse the
whole run -- a re-resolve failing for one package must not disarm the machine.

A named run touches only what it was asked about and drops nothing: `update
fzf` quietly deleting a stale aichat pin is a surprise nobody asked for.

The winget map is carried through untouched. Phase 3 resolves scoop only, and
dropping pins this command cannot resolve would delete what Phase 4 needs.

Negative controls, all four run: folding Repinned into Unchanged, skipping the
kept-entry reinsert, making Scope::covers always true, and dropping the winget
carry-through each leave exactly their own test red.
EOF
```

---

## Task 9: The `update` driver, its CLI, and its output

**Files:**
- Modify: `src/update.rs` (the driver)
- Modify: `src/render.rs`
- Modify: `src/main.rs`
- Test: `tests/update.rs` (new)

**Interfaces:**
- Consumes: Tasks 4, 6, 7, 8.
- Produces:
  - `update::run(scoop_root: &Path, declared: &Config, old: &Lock, scope: &Scope, offline: bool) -> (Update, Vec<String>)` — the `Vec<String>` is warnings.
  - `render::render_update(u: &Update) -> String`

- [ ] **Step 1: Write the failing integration test**

Create `tests/update.rs`:

```rust
mod common;

use common::*;
use dotpkg::lock::{Lock, Pin};
use dotpkg::model::Name;
use dotpkg::update::{self, Change, Scope};

fn cfg(text: &str) -> dotpkg::config::Config {
    dotpkg::config::parse(text).unwrap()
}

#[test]
fn update_resolves_a_declared_package_against_the_bucket_on_disk() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");
    let newest = f.commit(&dir, "tool.json", "2.0.0", "v200");

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );

    assert_eq!(
        u.changes,
        vec![Change::Added { name: Name::new("tool"), version: "2.0.0".into() }]
    );
    match &u.lock.scoop[&Name::new("tool")] {
        Pin::ScoopCommit { bucket, commit, version } => {
            assert_eq!(bucket, "main");
            assert_eq!(commit, &newest);
            assert_eq!(version, "2.0.0");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_lock_update_writes_is_one_apply_would_accept() {
    // The property that makes update a fix rather than another way to break
    // the machine: its own output goes through the reader's guard.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    let (u, _) = update::run(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    dotpkg::apply::lock_coherence_guard(&u.lock)
        .expect("update must never produce a lock apply refuses");

    let path = f.home.path().join("pkg.lock");
    dotpkg::lock::save(&u.lock, &path).unwrap();
    assert_eq!(dotpkg::lock::load_or_empty(&path).unwrap(), u.lock);
}

#[test]
fn an_ambiguous_bucket_keeps_the_old_pin_and_names_both_candidates() {
    let f = Fixture::new();
    for b in ["main", "extras"] {
        let dir = f.bucket(b);
        f.commit(&dir, "tool.json", "1.0.0", "v100");
    }
    let mut old = Lock::default();
    old.scoop.insert(
        Name::new("tool"),
        Pin::ScoopCommit { bucket: "main".into(), commit: "a".repeat(40), version: "0.9.0".into() },
    );

    // The lock names a bucket, so it decides -- that arm must be exercised
    // separately from the ambiguity. Here the lock is dropped to force it.
    let (u, _) = update::run(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    match &u.changes[0] {
        Change::Kept { why, .. } => {
            assert!(why.contains("main") && why.contains("extras"), "name both: {why}");
            assert!(why.contains("scoop.opts"), "say how to fix it: {why}");
        }
        other => panic!("ambiguity must not be guessed: {other:?}"),
    }
    assert!(u.lock.scoop.is_empty(), "nothing resolved, nothing written");
    let _ = old;
}

#[test]
fn a_bucket_with_no_upstream_warns_that_latest_is_only_as_current_as_the_clone() {
    // A locally-created bucket cannot be fetched. Resolving is still possible;
    // calling the answer "latest" without saying so is not.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    let (_u, warnings) = update::run(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        false,
    );
    assert!(
        warnings.iter().any(|w| w.contains("main") && w.contains("upstream")),
        "name the bucket and what is missing: {warnings:?}"
    );
}

#[test]
fn offline_skips_the_fetch_and_says_the_result_may_be_stale() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    let (_u, warnings) = update::run(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    assert!(
        warnings.iter().any(|w| w.contains("offline")),
        "an offline run must say so: {warnings:?}"
    );
}

#[test]
fn a_fetch_moves_the_pin_forward() {
    // The one property that most needs proving and is invisible when the
    // bucket is already current: `latest` means fetched, not cached.
    let f = Fixture::new();
    let upstream = f.bucket("upstream");
    f.commit(&upstream, "tool.json", "1.0.0", "v100");

    let clone_dir = f.scoop_root().join("buckets").join("main");
    git(
        f.home.path(),
        &["clone", "-q", &format!("file://{}", upstream.display()), &clone_dir.to_string_lossy()],
    );

    // The upstream moves after the clone. Without a fetch, update cannot see it.
    let moved = f.commit(&upstream, "tool.json", "2.0.0", "v200");

    let config = cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n");
    let (stale, _) = update::run(&f.scoop_root(), &config, &Lock::default(), &Scope::WholeRun, true);
    assert_eq!(
        stale.changes,
        vec![Change::Added { name: Name::new("tool"), version: "1.0.0".into() }],
        "offline must see only what the clone had"
    );

    let (fresh, _) = update::run(&f.scoop_root(), &config, &Lock::default(), &Scope::WholeRun, false);
    assert_eq!(
        fresh.changes,
        vec![Change::Added { name: Name::new("tool"), version: "2.0.0".into() }],
        "a fetch is what makes `latest` mean latest"
    );
    match &fresh.lock.scoop[&Name::new("tool")] {
        Pin::ScoopCommit { commit, .. } => assert_eq!(commit, &moved),
        other => panic!("{other:?}"),
    }

    // And the bucket's own branch did not move: it is scoop's, not dotpkg's.
    assert_eq!(
        git(&clone_dir, &["rev-parse", "HEAD"]).trim(),
        git(&clone_dir, &["rev-parse", "refs/heads/main"]).trim(),
    );
    assert_ne!(
        git(&clone_dir, &["rev-parse", "HEAD"]).trim(),
        moved,
        "update must fetch, never pull: the working branch stays where scoop put it"
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --no-fail-fast --test update 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function 'run' in module 'update'`.

- [ ] **Step 3: Implement the driver**

Append to `src/update.rs`, above the test module:

```rust
use crate::bucket::{self, BucketChoice};
use crate::config::Config;
use std::path::Path;

/// Resolve every declared scoop package against the buckets on disk.
///
/// Returns the decision plus the warnings that belong on stderr. Warnings are
/// returned rather than printed so that this whole function is testable.
///
/// `offline` skips the fetch. Everything else about the run is identical, and
/// the caller is told, because "latest" out of a bucket nobody fetched is
/// "latest as of whenever something else last pulled it".
pub fn run(
    scoop_root: &Path,
    declared: &Config,
    old: &Lock,
    scope: &Scope,
    offline: bool,
) -> (Update, Vec<String>) {
    let mut warnings = Vec::new();

    if offline {
        warnings.push(
            "offline: buckets were not fetched, so `latest` means whatever this \
             machine last pulled."
                .to_string(),
        );
    } else {
        for b in &declared.scoop.buckets {
            let dir = scoop_root.join("buckets").join(b.name.key());
            if !dir.join(".git").exists() {
                continue;
            }
            if bucket::tip(&dir).stale.is_some() {
                warnings.push(format!(
                    "bucket {}: no upstream to fetch from, so `latest` is only as \
                     current as this clone.",
                    b.name
                ));
                continue;
            }
            if let Err(e) = bucket::fetch(&dir) {
                warnings.push(format!(
                    "bucket {}: could not fetch ({e:#}); resolving against what is \
                     already on disk.",
                    b.name
                ));
            }
        }
    }

    let mut resolutions = BTreeMap::new();
    for name in &declared.scoop.packages {
        if !scope.covers(name) {
            continue;
        }
        let already = old.scoop.get(name).and_then(|p| match p {
            Pin::ScoopCommit { bucket, .. } => Some(bucket.as_str()),
            Pin::WingetVersion { .. } => None,
        });
        let resolution = match bucket::choose_bucket(scoop_root, declared, name, already) {
            BucketChoice::Ambiguous { candidates } => {
                let names: Vec<String> = candidates.iter().map(|c| c.to_string()).collect();
                Resolution::Failed {
                    why: format!(
                        "{} declared buckets carry it ({}). Say which with \
                         `[scoop.opts] {name} = {{ bucket = \"...\" }}`.",
                        candidates.len(),
                        names.join(", ")
                    ),
                }
            }
            BucketChoice::NotFound { searched } => {
                let names: Vec<String> = searched.iter().map(|s| s.to_string()).collect();
                Resolution::Failed {
                    why: format!("no declared bucket has it (searched: {})", names.join(", ")),
                }
            }
            BucketChoice::Chosen { name: bucket_name, dir, tip } => {
                match bucket::resolve_latest(&dir, name, &tip.rev) {
                    Ok(Some(latest)) => {
                        if latest.fell_back_to_tip {
                            warnings.push(format!(
                                "{name}: no single commit carries this manifest's current \
                                 content, so the bucket tip was pinned instead."
                            ));
                        }
                        Resolution::Resolved {
                            bucket: bucket_name.to_string(),
                            commit: latest.commit,
                            version: latest.version,
                        }
                    }
                    Ok(None) => Resolution::Failed {
                        why: format!("bucket {bucket_name} has no manifest for it"),
                    },
                    Err(e) => Resolution::Failed { why: format!("{e:#}") },
                }
            }
        };
        resolutions.insert(name.clone(), resolution);
    }

    if !declared.winget.packages.is_empty() {
        warnings.push(format!(
            "{} winget package(s) were not resolved: the winget backend lands in \
             phase 4. Their existing pins are untouched.",
            declared.winget.packages.len()
        ));
    }

    (
        resolve_into_lock(old, &declared.scoop.packages, &resolutions, scope),
        warnings,
    )
}
```

- [ ] **Step 4: Render the diff**

Append to `src/render.rs`:

```rust
use crate::update::{Change, Update};

/// The diff between the old lock and the new one — the only place both exist
/// at once, and therefore the only place a user can be told that a
/// same-version re-pin will produce no action at all.
pub fn render_update(u: &Update) -> String {
    let mut out = String::new();
    for c in &u.changes {
        let line = match c {
            Change::Added { name, version } => {
                format!("  + scoop  {name:<14} {version:<26} (new pin)")
            }
            Change::VersionChanged { name, from, to } => format!(
                "  ^ scoop  {name:<14} {:<26} (version changed)",
                format!("{from} -> {to}")
            ),
            Change::RepinnedSameVersion { name, version } => format!(
                "  = scoop  {name:<14} {:<26} (apply will not act on this)",
                format!("{version}, commit re-pinned")
            ),
            Change::Dropped { name, version } => {
                format!("  - scoop  {name:<14} {version:<26} (dropped, no longer declared)")
            }
            Change::Kept { name, why, .. } => {
                format!("  ! scoop  {name:<14} kept the previous pin: {why}")
            }
            // An unchanged package is the ordinary case and would drown the
            // lines that matter. Counted in the summary instead.
            Change::Unchanged { .. } => continue,
        };
        out.push_str(&line);
        out.push('\n');
    }

    let unchanged = u
        .changes
        .iter()
        .filter(|c| matches!(c, Change::Unchanged { .. }))
        .count();
    out.push_str(&format!(
        "\n  {} changed, {} unchanged, {} could not be resolved.\n",
        u.changes.len() - unchanged - u.failed_count(),
        unchanged,
        u.failed_count(),
    ));
    if !u.wrote_anything() {
        out.push_str("  pkg.lock is already current -- not rewritten.\n");
    }
    out
}
```

- [ ] **Step 5: Wire the subcommand**

In `src/main.rs`, add to `enum Command`:

```rust
    /// Re-resolve pkg.toml against the buckets and rewrite pkg.lock. The only
    /// command that asks what is newest, and the only one that fetches.
    Update {
        #[arg(long, default_value = "pkg.toml")]
        config: PathBuf,
        #[arg(long, default_value = "pkg.lock")]
        lock: PathBuf,
        /// Do not fetch. `latest` then means whatever this machine last
        /// pulled, and the output says so.
        #[arg(long)]
        offline: bool,
        /// Resolve only these packages. Nothing else is rewritten and no
        /// entry is dropped.
        packages: Vec<String>,
    },
```

And the arm:

```rust
        Command::Update { config, lock, offline, packages } => {
            let declared = dotpkg::config::load(&config)?;
            let old = dotpkg::lock::load_or_empty(&lock)?;
            let scope = if packages.is_empty() {
                dotpkg::update::Scope::WholeRun
            } else {
                dotpkg::update::Scope::Named(
                    dotpkg::model::fold_names(packages, "the packages named on the command line")?,
                )
            };
            if let dotpkg::update::Scope::Named(names) = &scope {
                for n in names {
                    if !declared.scoop.packages.contains(n) {
                        refuse(anyhow::anyhow!(
                            "{n} is not declared in {}. `update` re-resolves what pkg.toml \
                             already asks for; add it there first.",
                            config.display()
                        ));
                    }
                }
            }

            let scoop = Scoop::discover();
            let (u, warnings) = dotpkg::update::run(scoop.root(), &declared, &old, &scope, offline);
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            print!("{}", dotpkg::render::render_update(&u));
            std::io::stdout().flush().ok();

            if u.wrote_anything() {
                dotpkg::lock::save(&u.lock, &lock)?;
            }
            if u.failed_count() > 0 {
                std::process::exit(1);
            }
        }
```

- [ ] **Step 6: Run everything**

Run: `cargo test --no-fail-fast 2>&1 | grep -E "^test result:"`
Expected: all `ok`.

- [ ] **Step 7: Run it against this repository's own fixtures by hand**

```bash
cargo run -- update --help
```
Expected: the flags above, `--offline` among them.

- [ ] **Step 8: Run the negative controls**

1. Make `run` skip the fetch entirely (delete the `else` branch's body). Confirm `a_fetch_moves_the_pin_forward` goes red on the "a fetch is what makes `latest` mean latest" assertion, while `offline_skips_the_fetch_and_says_the_result_may_be_stale` stays green.
2. Replace `bucket::fetch` with `git pull` semantics by adding `git(&dir, &["merge", "--ff-only", "@{u}"])` after the fetch. Confirm the final assertion in `a_fetch_moves_the_pin_forward` — that the branch did not move — goes red. Restore. This is the control that proves the "fetch, never pull" rule is tested rather than merely written down.
3. Make the ambiguity arm resolve to the first candidate. Confirm `an_ambiguous_bucket_keeps_the_old_pin_and_names_both_candidates` goes red.

- [ ] **Step 9: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Add dotpkg update: fetch, resolve, and say what apply will ignore

The command that makes the reproducibility claim true by running dotpkg rather
than by running a PowerShell script out of a dogfood appendix.

It fetches. "latest" out of a bucket nobody fetched is "latest as of whenever
something else last pulled it", and a lock built from that is stale while
claiming to be current -- the same class of error as a lock quietly falling
back to latest, in the other direction. --offline skips it and the output says
so rather than passing the result off as current.

It fetches and never pulls or checks out. A bucket is scoop's directory;
resolution reads the remote-tracking ref, and the fetched objects stay
reachable from refs/remotes/ so Scoop::stage can git show them at apply time
without any branch having moved. A test asserts the branch did NOT move, and
its negative control adds a `merge --ff-only` to prove that assertion can fire.

`=` lines say outright that a same-version re-pin produces no action. Ambiguity
keeps the old pin and names every candidate plus the [scoop.opts] key that
settles it. A named run refuses a package pkg.toml does not declare rather than
inventing one -- that is `add`, which is Phase 5.

Negative controls, all three run: skipping the fetch leaves the fetch test red
with the offline test green; adding a merge after the fetch leaves the
branch-did-not-move assertion red; resolving ambiguity to the first candidate
leaves the ambiguity test red.
EOF
```

---

## Task 10: Edit `pkg.toml` without destroying it

`pkg.toml` is the only file dotpkg writes that a human wrote by hand and committed with comments. It gets more protection than the two files dotpkg owns, not less.

**Files:**
- Create: `src/config_edit.rs`
- Modify: `Cargo.toml`, `src/lib.rs`
- Test: `src/config_edit.rs` unit tests

**Interfaces:**
- Consumes: `config::{parse, Config}`, `model::Name`.
- Produces:
  - `config_edit::add_scoop_package(text: &str, name: &Name) -> Result<String>`
  - `config_edit::save(path: &Path, text: &str) -> Result<()>`

- [ ] **Step 1: Promote `toml_edit` to a direct dependency**

In `Cargo.toml`, under `[dependencies]`:

```toml
toml_edit = "0.22"
```

It is already in `Cargo.lock` at 0.22.27 as a transitive dependency of `toml` 0.8, so this adds no crate to the tree. Verify:

```bash
cargo tree -i toml_edit 2>&1 | head -10
git diff --stat Cargo.lock
```
Expected: `Cargo.lock` gains only the `dotpkg` → `toml_edit` edge, no new `[[package]]` block. If a new package appears, stop and report it — the "no new crates" claim in the design is then false.

- [ ] **Step 2: Write the failing tests**

Create `src/config_edit.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const HAND_WRITTEN: &str = r#"# what this machine should have
[scoop]
buckets  = ["main", "extras"]
packages = [
  "fzf",     # fuzzy finder
  "bat",
]

[scoop.opts]
python = { arch = "64bit" }   # force an architecture
"#;

    #[test]
    fn a_package_is_added_and_every_comment_survives() {
        let out = add_scoop_package(HAND_WRITTEN, &Name::new("ripgrep")).unwrap();

        assert!(out.contains("# what this machine should have"), "{out}");
        assert!(out.contains("# fuzzy finder"), "{out}");
        assert!(out.contains("# force an architecture"), "{out}");

        let cfg = crate::config::parse(&out).unwrap();
        assert!(cfg.scoop.packages.contains(&Name::new("ripgrep")));
        assert!(cfg.scoop.packages.contains(&Name::new("fzf")));
        assert!(cfg.scoop.packages.contains(&Name::new("bat")));
        assert_eq!(cfg.scoop.buckets.len(), 2);
        assert_eq!(cfg.scoop.opts[&Name::new("python")].arch, Some(crate::config::Arch::X64));
    }

    #[test]
    fn adding_a_package_that_is_already_declared_is_refused_rather_than_duplicated() {
        // `packages = ["fzf", "fzf"]` is refused by config::parse, so a blind
        // append would produce a pkg.toml that no longer loads at all.
        let err = add_scoop_package(HAND_WRITTEN, &Name::new("FZF")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("fzf") || msg.contains("FZF"), "name it: {msg}");
        assert!(msg.contains("already"), "say why: {msg}");
    }

    #[test]
    fn a_file_with_no_scoop_section_grows_one() {
        let out = add_scoop_package("[winget]\npackages = [\"Git.Git\"]\n", &Name::new("fzf")).unwrap();
        let cfg = crate::config::parse(&out).unwrap();
        assert!(cfg.scoop.packages.contains(&Name::new("fzf")));
        assert!(cfg.winget.packages.contains(&Name::new("Git.Git")), "{out}");
    }

    #[test]
    fn an_edit_that_changes_anything_else_is_refused_rather_than_written() {
        // The guard, exercised through a document toml_edit and config::parse
        // disagree about. `parse` uses deny_unknown_fields, so a stray key
        // means the round trip cannot be checked -- and an unverifiable edit
        // to a hand-written committed file is refused, not guessed at.
        let err = add_scoop_package("[scoop]\npackagess = [\"fzf\"]\n", &Name::new("bat"))
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("packagess"),
            "the original file's own problem must be named: {err:#}"
        );
    }

    #[test]
    fn the_round_trip_guard_is_reached_and_compares_the_whole_config() {
        // A positive statement of the same guard: the result must parse to
        // exactly the original config plus one package, and nothing else.
        let out = add_scoop_package(HAND_WRITTEN, &Name::new("ripgrep")).unwrap();
        let before = crate::config::parse(HAND_WRITTEN).unwrap();
        let after = crate::config::parse(&out).unwrap();

        assert_eq!(after.scoop.buckets, before.scoop.buckets);
        assert_eq!(after.scoop.opts, before.scoop.opts);
        assert_eq!(after.winget.packages, before.winget.packages);
        assert_eq!(after.scoop.packages.len(), before.scoop.packages.len() + 1);
    }

    #[test]
    fn saving_keeps_the_displaced_file_alongside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.toml");
        std::fs::write(&path, HAND_WRITTEN).unwrap();

        let out = add_scoop_package(HAND_WRITTEN, &Name::new("ripgrep")).unwrap();
        save(&path, &out).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), out);
        let bak = path.with_extension("toml.bak");
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap(),
            HAND_WRITTEN,
            "the file the user wrote is kept at {bak:?}"
        );
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Add `pub mod config_edit;` to `src/lib.rs`, then:
Run: `cargo test --no-fail-fast --lib config_edit 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function 'add_scoop_package'`.

- [ ] **Step 4: Implement**

Prepend to `src/config_edit.rs`:

```rust
//! Adding a package to `pkg.toml`.
//!
//! `pkg.toml` is the only file dotpkg writes that a human wrote by hand and
//! committed with comments in it. `pkg.lock` and `state.json` are dotpkg's own
//! and can be rendered from scratch; this one cannot, so it is edited in place
//! with `toml_edit` and every edit is verified before it replaces the original.

use crate::model::Name;
use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Value};

/// Add `name` to `[scoop] packages`, preserving comments, ordering and
/// formatting.
///
/// Refuses rather than guesses in three cases: the file does not parse, the
/// package is already declared (`config::parse` rejects a duplicate, so a
/// blind append would leave a `pkg.toml` that no longer loads), or the result
/// does not re-parse to exactly the original config plus this one name.
///
/// That last check is the reason this function returns a `String` instead of
/// writing: the verification has to happen before anything reaches disk.
pub fn add_scoop_package(text: &str, name: &Name) -> Result<String> {
    let before = crate::config::parse(text)
        .context("refusing to edit a pkg.toml that does not parse")?;
    anyhow::ensure!(
        !before.scoop.packages.contains(name),
        "{name} is already declared in pkg.toml (package names are compared \
         without regard to case)"
    );

    let mut doc: DocumentMut = text
        .parse()
        .context("refusing to edit a pkg.toml that does not parse as TOML")?;

    let scoop = doc
        .entry("scoop")
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("pkg.toml's [scoop] is not a table")?;
    let packages = scoop
        .entry("packages")
        .or_insert_with(|| Item::Value(Value::Array(Array::new())))
        .as_array_mut()
        .context("pkg.toml's [scoop] packages is not an array")?;

    // Match the surrounding style: if the existing entries are on their own
    // lines, keep that; otherwise append inline.
    let multiline = packages.iter().count() > 0
        && packages
            .iter()
            .any(|_| packages.to_string().contains('\n'));
    packages.push(name.to_string());
    if multiline {
        if let Some(last) = packages.get_mut(packages.len() - 1) {
            last.decor_mut().set_prefix("\n  ");
        }
        packages.set_trailing_comma(true);
        packages.set_trailing("\n");
    }

    let out = doc.to_string();

    // The guard. An edit to a hand-written committed file that cannot be
    // verified is refused, not written.
    let after = crate::config::parse(&out)
        .context("the edit produced a pkg.toml that no longer parses; refusing to write it")?;
    anyhow::ensure!(
        after.scoop.buckets == before.scoop.buckets
            && after.scoop.opts == before.scoop.opts
            && after.winget.packages == before.winget.packages,
        "the edit changed something other than [scoop] packages; refusing to write it"
    );
    let mut want = before.scoop.packages.clone();
    want.push(name.clone());
    anyhow::ensure!(
        after.scoop.packages == want,
        "the edit did not add exactly {name} to [scoop] packages; refusing to write it"
    );

    Ok(out)
}

/// Replace `pkg.toml`, keeping the file the user wrote as `pkg.toml.bak`.
///
/// Temp-then-rename, the same discipline as `State::save` and `lock::save`.
pub fn save(path: &Path, text: &str) -> Result<()> {
    let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("pkg.toml");
    let tmp = path.with_file_name(format!("{stem}.tmp{}", std::process::id()));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("cannot create {}", tmp.display()))?;
        f.write_all(text.as_bytes())
            .with_context(|| format!("cannot write {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("cannot flush {}", tmp.display()))?;
    }
    if path.exists() {
        let _ = std::fs::copy(path, path.with_extension("toml.bak"));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::Error::new(e)
            .context(format!("cannot move {} into place at {}", tmp.display(), path.display()))
    })
}
```

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test --no-fail-fast --lib config_edit 2>&1 | grep -E "^test result:"`
Expected: `ok`. If the multiline formatting assertion fails, adjust the decor calls — the *behaviour* under test is that comments survive and the config round-trips, not the exact whitespace.

- [ ] **Step 6: Run the negative controls**

1. Delete the whole guard block (from `let after = ...` to the last `ensure!`) and return `Ok(out)` directly. Confirm `an_edit_that_changes_anything_else_is_refused_rather_than_written` goes red. This control needs the *file with a stray key*, which is why that fixture is `packagess` rather than a valid file — a valid file's round trip would succeed and prove nothing.
2. Delete only the `already declared` check. Confirm `adding_a_package_that_is_already_declared_is_refused_rather_than_duplicated` goes red — and note in the commit message that it goes red at the *round-trip guard*, not at the intended check, which is why both exist.
3. Replace `doc.to_string()` with `toml::to_string(&before)`-style re-serialisation. Confirm `a_package_is_added_and_every_comment_survives` goes red on the first comment assertion.

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Edit pkg.toml in place, and verify the edit before it lands

pkg.toml is the only file dotpkg writes that a human wrote by hand and
committed with comments. pkg.lock and state.json are dotpkg's own and can be
rendered from scratch; this one cannot. So it gets more protection than either,
not less: toml_edit in place, the displaced file kept as pkg.toml.bak, and the
result re-parsed with config::parse and compared field by field against the
original before it replaces anything.

Returning a String rather than writing is deliberate -- the verification has to
happen before anything reaches disk.

Adding a package that is already declared is refused rather than appended:
config::parse rejects ["fzf", "fzf"], so a blind append would leave a pkg.toml
that no longer loads at all.

toml_edit was already in Cargo.lock at 0.22.27 as a transitive dependency of
toml 0.8, so promoting it adds no crate to the tree. Verified with cargo tree
and by the absence of a new [[package]] block in the lock diff.

Negative controls: removing the round-trip guard leaves the stray-key test red;
removing the duplicate check leaves that test red at the round-trip guard
instead of at its own check, which is why both exist; re-serialising instead of
editing leaves the comment-preservation test red.
EOF
```

---

## Task 11: `adopt`'s resolution — content first, version second

**Files:**
- Create: `src/adopt.rs`
- Modify: `src/lib.rs`, `src/verify.rs` (make `normalise` reachable)
- Test: `tests/adopt.rs` (new)

**Interfaces:**
- Consumes: `bucket::{history, blobs, manifest_path, is_shallow, choose_bucket}`, `verify::normalise`.
- Produces:

```rust
pub enum Matched { Content, Version }
pub struct Found { pub commit: String, pub version: String, pub matched: Matched }
pub fn resolve_installed(
    bucket_dir: &Path,
    app: &Name,
    installed_version: &str,
    installed_manifest: &[u8],
    rev: &str,
) -> Result<Option<Found>>;
```

- [ ] **Step 1: Make `normalise` reachable**

In `src/verify.rs`, change `fn normalise` to `pub(crate) fn normalise`, and extend its doc comment:

```rust
/// Collapse CRLF and drop trailing newlines.
///
/// Two files equal under this are the same JSON, so `verdict` treats them as
/// a match. The class is slightly wider than "line endings" -- `{"a":1}` vs.
/// `{"a":1}\n\n\n` lands here too -- and that is still safe: neither
/// transformation can change a url or a hash, which is what the byte
/// comparison exists to protect.
///
/// `adopt` is the second consumer as of Phase 3, and needs exactly this
/// function rather than one of its own: measured, comparing an installed
/// manifest against a bucket blob raw matches nothing at all, because scoop
/// rewrites line endings when it copies the file into `apps/<app>/current`.
pub(crate) fn normalise(b: &[u8]) -> Vec<u8> {
```

- [ ] **Step 2: Write the failing tests**

Create `tests/adopt.rs`:

```rust
mod common;

use common::*;
use dotpkg::adopt::{self, Matched};
use dotpkg::model::Name;

/// The installed manifest, as scoop leaves it: the bucket's bytes with CRLF.
fn as_scoop_installs_it(body: &str) -> Vec<u8> {
    body.replace('\n', "\r\n").into_bytes()
}

#[test]
fn the_installed_bytes_pick_the_right_commit_when_two_carry_one_version() {
    // Measured section C, and the reason adopt is strictly better than the
    // Phase 2b-1 rehearsal script it replaces. That script matched on version
    // and would pin this machine to the NEWER commit -- content it is not
    // running.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let older = f.commit(&dir, "tool.json", "2.0.0", "good");
    let newer = f.commit(&dir, "tool.json", "2.0.0", "amended");
    assert_ne!(older, newer);

    let installed = as_scoop_installs_it(&f.blob(&dir, &older, "tool.json"));
    let found = adopt::resolve_installed(&dir, &Name::new("tool"), "2.0.0", &installed, "HEAD")
        .unwrap()
        .expect("2.0.0 is in this history twice");

    assert_eq!(found.commit, older, "the commit whose content is actually installed");
    assert_eq!(found.matched, Matched::Content);
}

#[test]
fn a_manifest_scoop_rewrote_still_matches_because_normalise_is_used() {
    // The control for the test above: without normalise the comparison finds
    // nothing and the fallback silently picks the newer commit instead.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let older = f.commit(&dir, "tool.json", "2.0.0", "good");
    f.commit(&dir, "tool.json", "2.0.0", "amended");

    let raw = f.blob(&dir, &older, "tool.json");
    assert!(raw.contains('\n') && !raw.contains("\r\n"), "the blob is LF");
    let installed = as_scoop_installs_it(&raw);
    assert!(
        String::from_utf8_lossy(&installed).contains("\r\n"),
        "the fixture must actually differ from the blob"
    );

    let found = adopt::resolve_installed(&dir, &Name::new("tool"), "2.0.0", &installed, "HEAD")
        .unwrap()
        .unwrap();
    assert_eq!(found.matched, Matched::Content);
    assert_eq!(found.commit, older);
}

#[test]
fn a_manifest_that_matches_nothing_byte_for_byte_falls_back_to_the_version() {
    // A machine whose manifest was rewritten by something other than line
    // endings -- an older scoop, a hand edit. The version is a weaker answer
    // and it is recorded as such rather than presented as exact.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let c = f.commit(&dir, "tool.json", "3.1.0", "v310");

    let found = adopt::resolve_installed(
        &dir,
        &Name::new("tool"),
        "3.1.0",
        br#"{"version":"3.1.0","note":"rewritten by something else"}"#,
        "HEAD",
    )
    .unwrap()
    .unwrap();
    assert_eq!(found.commit, c);
    assert_eq!(found.matched, Matched::Version);
}

#[test]
fn adopt_finds_a_version_that_only_a_merged_branch_ever_had() {
    // Measured section B. Without --full-history this is unreachable and adopt
    // would refuse a package the user genuinely has installed.
    let f = Fixture::new();
    let (side_101, _main) = merged_bucket(&f, "main");
    let dir = f.bucket_dir("main");

    let found = adopt::resolve_installed(
        &dir,
        &Name::new("tool"),
        "1.0.1",
        br#"{"version":"1.0.1"}"#,
        "HEAD",
    )
    .unwrap()
    .expect("1.0.1 is an ancestor of HEAD even though the plain walk hides it");
    assert_eq!(found.commit, side_101);
}

#[test]
fn a_version_no_commit_carries_resolves_to_none() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    assert_eq!(
        adopt::resolve_installed(&dir, &Name::new("tool"), "9.9.9", b"{}", "HEAD").unwrap(),
        None
    );
}

#[test]
fn an_app_the_bucket_has_never_had_resolves_to_none() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    assert_eq!(
        adopt::resolve_installed(&dir, &Name::new("nosuch"), "1.0.0", b"{}", "HEAD").unwrap(),
        None
    );
}
```

- [ ] **Step 3: Run to verify they fail**

Add `pub mod adopt;` to `src/lib.rs`, then:
Run: `cargo test --no-fail-fast --test adopt 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function 'resolve_installed'`.

- [ ] **Step 4: Implement**

Create `src/adopt.rs`:

```rust
//! `dotpkg adopt` — bringing an already-installed package under management.
//!
//! Reaches no network and changes no installed software. Its whole job is to
//! find the commit whose manifest is the one this machine is actually running,
//! and then to write the three files that make the package managed rather than
//! merely known about.

use crate::bucket;
use crate::model::Name;
use anyhow::Result;
use std::path::Path;

/// Which rule found the commit. Reported, because the two are not equally
/// strong and a user is entitled to know which one answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matched {
    /// The installed manifest and the bucket blob are the same file. Exact,
    /// and the only rule that can tell two same-version commits apart.
    Content,
    /// Only the version agreed. Weaker: measured, when a bucket amends a
    /// manifest without bumping the version, this picks the newer of the two.
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub commit: String,
    pub version: String,
    pub matched: Matched,
}

/// Find the commit that carries what is installed.
///
/// `Ok(None)` means no commit in this bucket's history for this app carries
/// the installed version -- an ordinary answer while searching, and the caller
/// turns it into a refusal that writes nothing.
///
/// Content is tried across the whole history before version is tried at all,
/// rather than per commit: an exact match anywhere beats an approximate match
/// higher up. Measured, the difference is which of two same-version commits
/// gets pinned, and the version rule picks the wrong one.
pub fn resolve_installed(
    bucket_dir: &Path,
    app: &Name,
    installed_version: &str,
    installed_manifest: &[u8],
    rev: &str,
) -> Result<Option<Found>> {
    let Some(path_in_repo) = bucket::manifest_path(bucket_dir, app, rev) else {
        return Ok(None);
    };
    // --full-history: measured, the default walk hides a version that reached
    // the bucket only on a branch whose change was superseded at merge time.
    let commits = bucket::history(bucket_dir, &path_in_repo, rev)?;
    let blobs = bucket::blobs(bucket_dir, &commits, &path_in_repo)?;

    let want = crate::verify::normalise(installed_manifest);
    for (commit, blob) in commits.iter().zip(blobs.iter()) {
        let Some(body) = blob else { continue };
        if crate::verify::normalise(body) == want {
            return Ok(Some(Found {
                commit: commit.clone(),
                version: blob_version(body).unwrap_or_else(|| installed_version.to_string()),
                matched: Matched::Content,
            }));
        }
    }
    for (commit, blob) in commits.iter().zip(blobs.iter()) {
        let Some(body) = blob else { continue };
        if blob_version(body).as_deref() == Some(installed_version) {
            return Ok(Some(Found {
                commit: commit.clone(),
                version: installed_version.to_string(),
                matched: Matched::Version,
            }));
        }
    }
    Ok(None)
}

fn blob_version(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get("version")?.as_str().map(str::to_string)
}
```

`verify::normalise` is `pub(crate)`, so `tests/adopt.rs` reaches it only through `adopt`, which is correct — the tests exercise the rule, not the helper.

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test --no-fail-fast --test adopt 2>&1 | grep -E "^test result:"`
Expected: `ok`, 6 passed.

- [ ] **Step 6: Run the negative controls**

1. Delete the content loop entirely, leaving only the version loop. Confirm **both** `the_installed_bytes_pick_the_right_commit_when_two_carry_one_version` and `a_manifest_scoop_rewrote_still_matches_because_normalise_is_used` go red, and `a_manifest_that_matches_nothing_byte_for_byte_falls_back_to_the_version` stays green — the fallback must keep working.
2. Compare raw bytes instead of `normalise`d ones. Confirm `a_manifest_scoop_rewrote_still_matches_because_normalise_is_used` goes red *on the `matched` assertion* — it silently degrades to `Version` and picks the newer commit, which is exactly the failure this rule exists to prevent, and a test that only checked "something was found" would miss it.
3. Interleave the two loops (check content then version per commit). Confirm `the_installed_bytes_pick_the_right_commit_when_two_carry_one_version` goes red: the newer commit matches on version first and wins.
4. Drop `--full-history` in `bucket::history`. Confirm `adopt_finds_a_version_that_only_a_merged_branch_ever_had` goes red.

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Resolve what is installed by its bytes, not by its version number

adopt's resolver, and the place it becomes strictly better than the Phase 2b-1
rehearsal script it replaces. That script matched on version. Measured: given
two commits carrying one version -- a bucket amending a url or hash without
bumping -- version matching selects the NEWER, pinning a machine to content it
is not running.

Content is tried across the whole history before version is tried at all,
rather than per commit. An exact match anywhere beats an approximate match
higher up, and interleaving the two loops reintroduces the bug: the newer
commit matches on version first and wins. There is a negative control for
exactly that.

The comparison goes through verify::normalise, which now has a second consumer
and says so. Raw bytes match nothing: scoop rewrites line endings when it
copies a manifest into apps/<app>/current, the same fact that nearly made every
successful install in Phase 2b-2 report as a failure.

Matched::{Content,Version} is reported because the two rules are not equally
strong and a user is entitled to know which one answered.

Negative controls, all four run. The sharpest: comparing raw bytes leaves
a_manifest_scoop_rewrote_still_matches_because_normalise_is_used red on the
`matched` assertion rather than on "something was found" -- it degrades
silently to Version and picks the wrong commit, which a weaker test would miss.
EOF
```

---

## Task 12: `adopt` writes three files, in an order whose every prefix is inert

**Files:**
- Modify: `src/adopt.rs`, `src/render.rs`, `src/main.rs`
- Test: `tests/adopt.rs`

**Interfaces:**
- Consumes: Tasks 6, 7, 10, 11; `state::{State, Ownership}`; `backend::Backend` for the scan.
- Produces:

```rust
pub struct Plan { pub name: Name, pub bucket: Name, pub found: Found, pub installed_version: String }
pub enum Refusal { NotInstalled, AlreadyManaged, NoBucket(String), NotInHistory { searched: Name, shallow: bool } }
pub fn plan_one(...) -> std::result::Result<Plan, Refusal>;
pub fn commit_one(plan: &Plan, config_path: &Path, lock_path: &Path, state_path: &Path, lock: &mut Lock, state: &mut State) -> Result<()>;
```

- [ ] **Step 1: Write the failing test for the write order**

Append to `tests/adopt.rs`:

```rust
use dotpkg::state::{Ownership, State};

/// The three-file write, and the property that every prefix of it is inert.
#[test]
fn adopt_writes_the_lock_then_pkg_toml_then_state_and_each_prefix_is_safe() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    let c = f.commit(&dir, "aichat", "0.30.0", "v030");
    let _ = c;

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    std::fs::write(
        &config_path,
        "# hand written\n[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
    )
    .unwrap();

    // An installed, unowned aichat.
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(cur.join("manifest.json"), f.blob(&dir, "HEAD", "aichat")).unwrap();

    let out = dotpkg::adopt::run(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();
    assert_eq!(out.adopted.len(), 1, "{out:?}");

    // All three files, and only the intended change in each.
    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    assert!(lock.scoop.contains_key(&Name::new("aichat")));

    let cfg_text = std::fs::read_to_string(&config_path).unwrap();
    assert!(cfg_text.contains("# hand written"), "comments survive: {cfg_text}");
    let cfg = dotpkg::config::parse(&cfg_text).unwrap();
    assert!(cfg.scoop.packages.contains(&Name::new("aichat")));
    assert!(cfg.scoop.packages.contains(&Name::new("fzf")));

    let state = State::load_or_empty(&state_path).unwrap();
    assert_eq!(
        state.ownership(dotpkg::model::SCOOP, &Name::new("aichat")),
        Some(Ownership::Adopted),
        "adopt is the first writer of this variant"
    );
}

#[test]
fn an_adopted_package_is_not_a_prune_candidate_and_not_notlocked() {
    // The two failure modes the three-file rule exists to prevent, asserted
    // through the shipped planner rather than by reasoning about it.
    //
    // state.json alone => installed, owned, undeclared => Prune.
    // state.json + pkg.toml => declared, unlocked => Skip{NotLocked}, which
    // makes the next apply refuse the whole run at exit 2.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "aichat", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    std::fs::write(&config_path, "[scoop]\nbuckets = [\"main\"]\npackages = []\n").unwrap();
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(cur.join("manifest.json"), f.blob(&dir, "HEAD", "aichat")).unwrap();

    dotpkg::adopt::run(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    let declared = dotpkg::config::load(&config_path).unwrap();
    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    let state = State::load_or_empty(&state_path).unwrap();
    let scoop = dotpkg::backend::scoop::Scoop::new(f.scoop_root());
    let scan = dotpkg::backend::Backend::scan(&scoop).unwrap();
    let plan = dotpkg::plan::plan(
        &declared,
        &lock,
        &scan.installed,
        &state,
        &dotpkg::model::Running::default(),
    );

    for a in &plan.actions {
        match a {
            dotpkg::plan::Action::Prune { name, .. } => {
                panic!("an adopted package must never be a prune candidate: {name}")
            }
            dotpkg::plan::Action::Skip { name, reason, .. }
                if *reason == dotpkg::plan::SkipReason::NotLocked =>
            {
                panic!("an adopted package must not be NotLocked: {name}")
            }
            _ => {}
        }
    }
}

#[test]
fn a_package_whose_version_is_not_in_the_bucket_writes_nothing_at_all() {
    // All-or-nothing per package. A partial adopt is the shape the write order
    // is designed around, and the refusal path must not produce one.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "aichat", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    let original = "[scoop]\nbuckets = [\"main\"]\npackages = []\n";
    std::fs::write(&config_path, original).unwrap();

    // Installed at a version the bucket has never had.
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(cur.join("manifest.json"), r#"{"version":"9.9.9"}"#).unwrap();

    let out = dotpkg::adopt::run(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 0);
    assert_eq!(out.refused.len(), 1);
    let (name, why) = &out.refused[0];
    assert_eq!(name, &Name::new("aichat"));
    assert!(why.contains("9.9.9"), "name the version: {why}");
    assert!(why.contains("main"), "name the bucket searched: {why}");

    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), original, "pkg.toml untouched");
    assert!(!lock_path.exists(), "no lock written");
    assert!(!state_path.exists(), "no state written");
}

#[test]
fn a_refusal_names_shallowness_when_that_is_the_likely_cause() {
    // Measured: a shallow clone produces exactly the same "not found" with no
    // other signal, and the user has no way to tell the two apart.
    let f = Fixture::new();
    let upstream = f.bucket("upstream");
    f.commit(&upstream, "aichat", "0.29.0", "v029");
    f.commit(&upstream, "aichat", "0.30.0", "v030");
    let shallow = f.scoop_root().join("buckets").join("main");
    git(
        f.home.path(),
        &["clone", "-q", "--depth", "1", &format!("file://{}", upstream.display()),
          &shallow.to_string_lossy()],
    );

    let config_path = f.home.path().join("pkg.toml");
    std::fs::write(&config_path, "[scoop]\nbuckets = [\"main\"]\npackages = []\n").unwrap();
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(cur.join("manifest.json"), r#"{"version":"0.29.0"}"#).unwrap();

    let out = dotpkg::adopt::run(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &f.home.path().join("pkg.lock"),
        &f.home.path().join("state.json"),
    )
    .unwrap();

    let (_, why) = &out.refused[0];
    assert!(
        why.contains("shallow"),
        "a shallow bucket is the likely cause and must be named: {why}"
    );
}

#[test]
fn a_package_that_is_not_installed_is_refused_rather_than_invented() {
    let f = Fixture::new();
    f.bucket("main");
    let config_path = f.home.path().join("pkg.toml");
    std::fs::write(&config_path, "[scoop]\nbuckets = [\"main\"]\npackages = []\n").unwrap();

    let out = dotpkg::adopt::run(
        &f.scoop_root(),
        &[Name::new("nothere")],
        &config_path,
        &f.home.path().join("pkg.lock"),
        &f.home.path().join("state.json"),
    )
    .unwrap();
    let (_, why) = &out.refused[0];
    assert!(
        why.contains("not installed"),
        "adopt brings an EXISTING package under management: {why}"
    );
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --no-fail-fast --test adopt 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function 'run' in module 'adopt'`.

- [ ] **Step 3: Implement**

Append to `src/adopt.rs`:

```rust
use crate::backend::Backend;
use crate::config::Config;
use crate::lock::{Lock, Pin};
use crate::state::{Ownership, State};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub adopted: Vec<(Name, Matched)>,
    pub refused: Vec<(Name, String)>,
}

/// Adopt every named package. Per package it is all or nothing; across
/// packages a refusal is reported and the rest proceed, the same shape as
/// `prepare`.
///
/// **Write order: `pkg.lock`, then `pkg.toml`, then `state.json`.** Every
/// prefix of that order is inert:
///
/// - lock only: an entry for an undeclared package. `plan()` never reads it
///   and the next whole-run `update` drops it.
/// - lock + `pkg.toml`: declared, locked, and installed at the locked version,
///   so `plan()` emits nothing at all.
/// - all three: adopted.
///
/// The dangerous order is `state.json` first, which makes the package
/// `installed ∧ ¬declared ∧ owned` -- a **prune candidate** (`src/plan.rs`).
/// This mirrors the executor's own reasoning about claiming ownership late.
pub fn run(
    scoop_root: &Path,
    names: &[Name],
    config_path: &Path,
    lock_path: &Path,
    state_path: &Path,
) -> Result<Outcome> {
    let scoop = crate::backend::scoop::Scoop::new(scoop_root.to_path_buf());
    let scan = Backend::scan(&scoop)?;
    let mut out = Outcome::default();

    for name in names {
        // Re-read all three every iteration: each package's write must land
        // before the next one's guard reads it, or adopting two packages in
        // one command would lose the first.
        let declared = crate::config::load(config_path)?;
        let mut lock = crate::lock::load_or_empty(lock_path)?;
        let mut state = State::load_or_empty(state_path)?;

        match adopt_one(
            scoop_root, &scan, &declared, &lock, &state, name, config_path,
        ) {
            Err(why) => out.refused.push((name.clone(), why)),
            Ok((bucket_name, found, config_text)) => {
                lock.scoop.insert(
                    name.clone(),
                    Pin::ScoopCommit {
                        bucket: bucket_name.to_string(),
                        commit: found.commit.clone(),
                        version: found.version.clone(),
                    },
                );
                crate::lock::save(&lock, lock_path)?;
                crate::config_edit::save(config_path, &config_text)?;
                state.set(crate::model::SCOOP, name, Ownership::Adopted);
                state.save(state_path)?;
                out.adopted.push((name.clone(), found.matched));
            }
        }
    }
    Ok(out)
}

/// Everything that can refuse, before anything is written. Returns the pieces
/// the caller needs, so no partial state can exist between a check and a write.
#[allow(clippy::too_many_arguments)]
fn adopt_one(
    scoop_root: &Path,
    scan: &crate::backend::Scan,
    declared: &Config,
    lock: &Lock,
    state: &State,
    name: &Name,
    config_path: &Path,
) -> std::result::Result<(Name, Found, String), String> {
    let Some(inst) = scan
        .installed
        .iter()
        .find(|i| i.backend == crate::model::SCOOP && &i.name == name)
    else {
        return Err(format!(
            "{name} is not installed. `adopt` brings an existing package under \
             management; to install one, declare it and run `dotpkg update` then \
             `dotpkg apply`."
        ));
    };
    if state.owns(crate::model::SCOOP, name) {
        return Err(format!("{name} is already managed by dotpkg"));
    }

    let already = lock.scoop.get(name).and_then(|p| match p {
        Pin::ScoopCommit { bucket, .. } => Some(bucket.as_str()),
        Pin::WingetVersion { .. } => None,
    });
    // install.json's `bucket` is a legitimate hint here and nowhere else:
    // adopt targets packages dotpkg has never touched, and it is dotpkg's own
    // installs that lose the field.
    let hint = already.or(inst.bucket.as_deref());
    let (bucket_name, dir, rev) =
        match bucket::choose_bucket(scoop_root, declared, name, hint) {
            bucket::BucketChoice::Chosen { name: b, dir, tip } => (b, dir, tip.rev),
            bucket::BucketChoice::Ambiguous { candidates } => {
                let names: Vec<String> = candidates.iter().map(|c| c.to_string()).collect();
                return Err(format!(
                    "{} declared buckets carry {name} ({}). Say which with \
                     `[scoop.opts] {name} = {{ bucket = \"...\" }}`.",
                    candidates.len(),
                    names.join(", ")
                ));
            }
            bucket::BucketChoice::NotFound { searched } => {
                let names: Vec<String> = searched.iter().map(|s| s.to_string()).collect();
                return Err(format!(
                    "no declared bucket has {name} (searched: {})",
                    names.join(", ")
                ));
            }
        };

    let installed_manifest = std::fs::read(
        scoop_root
            .join("apps")
            .join(inst.name.to_string())
            .join("current")
            .join("manifest.json"),
    )
    .unwrap_or_default();

    let found = match resolve_installed(&dir, name, &inst.version, &installed_manifest, &rev) {
        Ok(Some(f)) => f,
        Ok(None) => {
            // Measured: a shallow clone gives exactly this answer with no
            // other signal, and the user cannot tell the two apart.
            let shallow = if bucket::is_shallow(&dir) {
                format!(
                    " -- and bucket {bucket_name} is a SHALLOW clone, so most of its \
                     history is not on this machine. `git -C {} fetch --unshallow` \
                     and try again.",
                    dir.display()
                )
            } else {
                String::new()
            };
            return Err(format!(
                "no commit in bucket {bucket_name} carries {name} {}{}",
                inst.version, shallow
            ));
        }
        Err(e) => return Err(format!("{e:#}")),
    };

    // Prepared, not written: the caller writes all three in order only once
    // every refusal above has been passed.
    let text = std::fs::read_to_string(config_path).map_err(|e| format!("{e}"))?;
    let config_text = if declared.scoop.packages.contains(name) {
        text
    } else {
        crate::config_edit::add_scoop_package(&text, name).map_err(|e| format!("{e:#}"))?
    };

    Ok((bucket_name, found, config_text))
}
```

- [ ] **Step 4: Render and wire the CLI**

Append to `src/render.rs`:

```rust
use crate::adopt::{Matched, Outcome as AdoptOutcome};

pub fn render_adopt(o: &AdoptOutcome) -> String {
    let mut out = String::new();
    for (name, matched) in &o.adopted {
        let how = match matched {
            Matched::Content => "the installed manifest matches the bucket exactly",
            Matched::Version => "matched by version only -- the installed manifest differs",
        };
        out.push_str(&format!("  + scoop  {name:<14} adopted ({how})\n"));
    }
    for (name, why) in &o.refused {
        out.push_str(&format!("  ! scoop  {name:<14} {why}\n"));
    }
    out.push_str(&format!(
        "\n  {} adopted, {} refused. Nothing installed and nothing removed.\n",
        o.adopted.len(),
        o.refused.len()
    ));
    out
}
```

In `src/main.rs`, add to `enum Command`:

```rust
    /// Bring already-installed packages under management. Writes pkg.lock,
    /// pkg.toml and state.json; installs and removes nothing.
    Adopt {
        #[arg(long, default_value = "pkg.toml")]
        config: PathBuf,
        #[arg(long, default_value = "pkg.lock")]
        lock: PathBuf,
        /// Where dotpkg records what it owns. Must be absolute if given.
        #[arg(long)]
        state: Option<PathBuf>,
        /// The packages to adopt. At least one -- there is deliberately no
        /// "adopt everything", which would be one keystroke from letting a
        /// later pkg.toml edit delete the whole machine.
        #[arg(required = true)]
        packages: Vec<String>,
    },
```

And the arm:

```rust
        Command::Adopt { config, lock, state, packages } => {
            let state_path = state.unwrap_or_else(State::default_path);
            if !state_path.is_absolute() {
                refuse(anyhow::anyhow!(
                    "the state file resolves to {}, which is relative to the current \
                     directory. Pass --state with an absolute path.",
                    state_path.display()
                ));
            }
            let names = dotpkg::model::fold_names(packages, "the packages named on the command line")?;
            let scoop = Scoop::discover();
            let out = dotpkg::adopt::run(scoop.root(), &names, &config, &lock, &state_path)?;
            print!("{}", dotpkg::render::render_adopt(&out));
            std::io::stdout().flush().ok();
            if !out.refused.is_empty() {
                std::process::exit(1);
            }
        }
```

- [ ] **Step 5: Run everything**

Run: `cargo test --no-fail-fast 2>&1 | grep -E "^test result:"`
Expected: all `ok`.

- [ ] **Step 6: Run the negative controls**

1. Reorder the writes so `state.save` runs first. Confirm nothing goes red — then **add the test that makes it go red**: assert in `a_package_whose_version_is_not_in_the_bucket_writes_nothing_at_all` that `state_path` does not exist. If it already does, note that the ordering itself is not directly observable from outside and record that honestly: the order's value is what a *crash* between writes leaves behind, which no test in this suite can induce. Say so in the commit message rather than claiming coverage.
2. Delete the `state.owns` early return. Confirm a new test — adopt the same package twice — reports the second as refused. Add it if it does not exist.
3. Delete the shallow branch. Confirm `a_refusal_names_shallowness_when_that_is_the_likely_cause` goes red.
4. Skip the `config_edit::add_scoop_package` call (write the original text back). Confirm `an_adopted_package_is_not_a_prune_candidate_and_not_notlocked` goes red on the **Prune** panic — which is the whole reason `adopt` writes `pkg.toml`.

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Add dotpkg adopt: three files, in an order whose every prefix is inert

Read off the shipped planner rather than reasoned about: state.json alone makes
the package installed-and-owned-and-undeclared, which src/plan.rs turns into a
Prune -- so `dotpkg adopt aichat; dotpkg apply` would REMOVE aichat. Adding
pkg.toml without the lock makes it Skip{NotLocked}, which makes every later
apply refuse the whole run at exit 2 and, under --keep-going, hold every prune
in the plan. All three, or the machine is left in a state dotpkg itself will
not act on. A test asserts both of those through plan() directly.

Write order is lock, pkg.toml, state.json. Lock only is an entry plan() never
reads. Lock plus pkg.toml is a declared, locked, correctly-installed package
that plan() says nothing about. state.json first is the prune shape and is
forbidden. Honest limit: no test here can induce a crash between two writes, so
the ORDER is argued rather than covered -- what is covered is that a refusal
writes none of the three.

adopt is the first writer of Ownership::Adopted, which has been readable since
2b-2 and never written. The 2b-2 test that an upgrade does not silently rewrite
it to Installed stops being vacuous.

install.json's `bucket` is used as a hint here and nowhere else, because adopt
targets packages dotpkg has never touched and it is dotpkg's own installs that
lose the field -- measured 2026-08-08.

A refusal names shallowness when the bucket is shallow: measured, that produces
exactly the same "not found" with no other signal.

There is deliberately no `adopt --all`. It is one keystroke from letting a
later pkg.toml edit delete every package on the machine.

Negative controls: dropping the shallow branch, the already-managed check, and
the pkg.toml write each leave their own test red -- the last on the Prune
panic, which is the whole reason adopt touches pkg.toml at all.
EOF
```

---

## Task 13: Content-address the staging path

The second carried debt. `stage_text` writes to `<staging_root>/<app>/<version>`, keyed on app and version only — so re-pinning the same version to a different commit overwrites the exact file an installed app's `install.json` points at. Phase 3 is the phase that makes re-pinning routine.

**Files:**
- Modify: `src/backend/scoop.rs` (`stage_text`, `Scoop::stage`)
- Test: `tests/prepare.rs`

**Interfaces:**
- Consumes: `bucket::*` (Task 3).
- Produces: staged path becomes `<staging_root>/<app.key()>/<version>/<commit>/<file>.json`.

- [ ] **Step 1: Write the failing test**

Add to `tests/prepare.rs`:

```rust
#[test]
fn two_commits_of_one_version_stage_to_different_paths() {
    // install.json records the staged path verbatim -- measured 2026-08-08,
    // `{"architecture":"arm64","url":"<the staging path>"}`. Keyed on app and
    // version alone, re-pinning the same version to a different commit
    // overwrites the file an installed app is still pointing at, and the app
    // silently starts describing a manifest it was not installed from.
    //
    // Phase 3 makes that re-pin routine: a bucket amending a url or hash
    // without bumping the version is exactly what `update`'s `=` line reports.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let dir = root.path().join("buckets").join("main");
    fs::create_dir_all(dir.join("bucket")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "t@example.invalid"]);
    git(&dir, &["config", "user.name", "t"]);

    let mut shas = Vec::new();
    for url in ["good", "amended"] {
        fs::write(
            dir.join("bucket").join("tool.json"),
            format!(r#"{{"version":"1.0.0","url":"https://example.invalid/{url}.zip"}}"#),
        )
        .unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", url]);
        shas.push(git(&dir, &["rev-parse", "HEAD"]).trim().to_string());
    }

    let scoop = Scoop::new(root.path().to_path_buf());
    let first = scoop
        .stage(stage_dir.path(), &Name::new("tool"), &pin("main", &shas[0], "1.0.0"))
        .unwrap();
    let second = scoop
        .stage(stage_dir.path(), &Name::new("tool"), &pin("main", &shas[1], "1.0.0"))
        .unwrap();

    assert_ne!(first, second, "one version at two commits must not share a path");
    assert!(first.exists(), "the first staged manifest must survive the second staging");
    assert!(
        fs::read_to_string(&first).unwrap().contains("good"),
        "the first path must still hold the FIRST commit's manifest"
    );
    assert!(fs::read_to_string(&second).unwrap().contains("amended"));

    // The filename is still what scoop takes the app name from.
    assert_eq!(first.file_name().unwrap(), "tool.json");
    assert_eq!(second.file_name().unwrap(), "tool.json");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --no-fail-fast --test prepare two_commits 2>&1 | tail -20`
Expected: FAIL on `one version at two commits must not share a path`.

- [ ] **Step 3: Implement**

In `src/backend/scoop.rs`, `stage_text` gains the commit in its path. It already takes `commit`, so only the `join` changes:

```rust
    // Keyed on the commit as well as the app and version. `install.json`
    // records this path verbatim, so two commits carrying one version sharing
    // a directory means the second staging silently rewrites the manifest the
    // first install still points at. `ensure_commit_hash` has already run, so
    // this component is 40 or 64 lowercase hex and needs no further check.
    let dir = staging_root
        .join(app.key())
        .join(version)
        .join(commit);
```

- [ ] **Step 4: Run everything**

Run: `cargo test --no-fail-fast 2>&1 | grep -E "^test result:"`
Expected: all `ok`. Several existing `tests/prepare.rs` assertions check `fs::read_dir(stage_dir).count() == 0` on refusal paths — those still hold, because nothing is created before `ensure_commit_hash` passes.

- [ ] **Step 5: Check the Windows path length**

`%LOCALAPPDATA%\dotpkg\manifests\<app>\<version>\<40 hex>\<app>.json` on a14 is roughly `C:\Users\kln\AppData\Local\dotpkg\manifests\tree-sitter\0.26.11\<40>\tree-sitter.json` — about 130 characters, comfortably inside the 260-character limit. Record the arithmetic in the commit message rather than assuming it; if a real package name pushes past 200, stop and use the first 12 characters of the commit instead, and say why.

- [ ] **Step 6: Run the negative control**

Revert the `join(commit)` and confirm `two_commits_of_one_version_stage_to_different_paths` goes red on the *content* assertion (`the first path must still hold the FIRST commit's manifest`), not only on the path inequality. The content assertion is the defect; the path inequality is only its symptom.

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && git add -A && git commit -F - <<'EOF'
Content-address the staging path so a re-pin cannot rewrite an install's manifest

The second debt docs/phase2b-notes.md carried into this phase. stage_text keyed
on app and version alone, and install.json records the staged path verbatim --
measured, {"architecture":"arm64","url":"<the staging path>"}. So re-pinning one
version to a different commit overwrote the exact file an installed app was
still pointing at, and the app silently began describing a manifest it was not
installed from.

Phase 3 is where that stops being theoretical: a bucket amending a url or hash
without bumping the version is precisely what update's `=` line reports, and
update makes producing one a single command.

The commit is safe as a path component without further checking:
ensure_commit_hash has already run, so it is 40 or 64 lowercase hex.

Path length on a14, worked out rather than assumed: the longest declared
package gives roughly 130 characters, well inside 260.

Negative control: reverting the join leaves the test red on the CONTENT
assertion, not merely on the path inequality -- the rewrite is the defect and
the shared path is only its symptom.
EOF
```

---

## Task 14: Whole-branch review, separate from the per-task reviews

This is the pass that found 8 surviving mutants in Phase 2b-2 by *running* mutation testing rather than by reading. It is a task, not a formality.

**Files:** none directly; produces `docs/phase3-notes.md`.

- [ ] **Step 1: Run the full suite on macOS with `--no-fail-fast`**

```bash
cargo test --no-fail-fast 2>&1 | grep -E "^test result:|FAILED|panicked"
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: all `ok`, no diff, no warnings.

- [ ] **Step 2: Run the suite on Windows, before the dogfood**

Copy `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` to `C:\Users\kln\dotpkg-build` on a14 (kept from earlier phases) and build natively. **`--no-fail-fast` is mandatory**: in Phase 2b-2 one failing target hid two real Windows defects for several rounds.

```powershell
cargo test --no-fail-fast 2>&1 | Select-String "^test result:|FAILED"
```

Three classes of failure to expect, all seen before: a rendered-path comparison (`/x/` vs `/x\`), a `#[should_panic]` on a `debug_assert!` compiled out under `--release`, and a comparison against `fs::canonicalize` where production strips `\\?\`. The new git tests add a fourth candidate: `file://` clone URLs built from a Windows path. If `tests/bucket.rs` or `tests/update.rs` fails on URL construction, fix the fixture, not the production code.

- [ ] **Step 3: Run mutation testing**

```bash
cargo mutants --no-shuffle -- --no-fail-fast 2>&1 | tail -40
```

Every surviving mutant is either a missing test or a deliberate decision recorded in `docs/phase3-notes.md` with its reason. The functions where a survivor matters most, in order:

1. `adopt::resolve_installed` — the two-loop ordering.
2. `update::resolve_into_lock` — the `RepinnedSameVersion` / `Unchanged` split and the `Kept` re-insert.
3. `bucket::blobs` — the missing-object branch.
4. `bucket::resolve_latest` — the tip self-check.
5. `config_edit::add_scoop_package` — the round-trip guard.
6. `lock::save` — the `lock_coherence_guard` call.

- [ ] **Step 4: Audit every negative control this plan claims**

For each task, confirm the control was actually run and its assertion recorded in the commit message. A control that "ran" and could not have gone red is the failure mode this project has hit three times. Specifically re-verify:

- Task 11's control 2 (raw bytes instead of `normalise`) goes red on the `matched` assertion, not merely on "something was found".
- Task 12's control 4 goes red on the **Prune** panic.
- Task 9's control 2 (`merge --ff-only` after fetch) goes red on the branch-did-not-move assertion.
- Task 13's control goes red on the content assertion.

- [ ] **Step 5: Check for `msg.contains` assertions with no counterweight**

```bash
grep -rn "contains(" tests/ src/ | grep -c "assert"
```
Every refusal assertion in this phase must be paired with either a count of files written (which must be zero) or a positive sibling that stays green. A mutation that always fails with the right words survives an unpaired `contains`.

- [ ] **Step 6: Write `docs/phase3-notes.md`**

Carrying forward, in the shape the previous three phases established: what was found by review, by mutation, or by running; which predictions were falsified; what is still open. It must state at minimum:

- Whether the write-order argument in `adopt` is covered or only argued.
- Which mutants survived and why each is acceptable.
- That `mass_prune_guard` still reads scoop only.
- Whatever the Windows run found.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -F - <<'EOF'
Review the whole branch by running it, not by reading it

Mutation testing, the full suite on Windows before the dogfood rather than
after, and an audit of every negative control this phase claims.

docs/phase3-notes.md records what survived and why, which predictions were
falsified, and what is carried forward.
EOF
```

---

## Task 15: Dogfood on a14

The first phase whose commands change no installed software. The risk is not that the machine breaks during the run — it is that the lock these commands produce is wrong and the *next* `apply` acts on it. So the product of this dogfood is a lock, and the test of the lock is `apply --prepare` against it.

- [ ] **Step 1: Reconnaissance, read-only**

Capture before anything runs, so nothing downstream compares against stale numbers: app count, cache count, every app's version, `kanata`'s process and PID, `explorer`'s PID, whether `%LOCALAPPDATA%\dotpkg` exists.

Method, unchanged from Phase 2b-2 and non-negotiable:
- `ssh a14`, user `kln`. **It is a laptop and it sleeps.** If ssh times out, ask for it to be woken. **Do not fabricate results.**
- Quoting through ssh breaks PowerShell: `-EncodedCommand` with UTF-16LE base64, output to a file in `$env:TEMP`, `scp` it back.
- Every file dotpkg parses is written with `[System.IO.File]::WriteAllText` and `UTF8Encoding($false)`. PowerShell 5.1's `Set-Content -Encoding UTF8` writes a BOM and `serde_json` rejects it with `expected value at line 1 column 1`.
- **Never start or stop kanata.**

- [ ] **Step 2: Build natively on a14**

Reuse `C:\Users\kln\dotpkg-build`. Confirm `dotpkg update --help` and `dotpkg adopt --help` show the flags this phase added — the binary under test must be the one just built.

- [ ] **Step 3: Question 1 — does `update` produce a lock `apply --prepare` accepts?**

Run `dotpkg update --config C:\Users\kln\pkg.toml --lock <fresh path>` over the real 25 declared packages, then `dotpkg apply --prepare` against it. Record the exit code of each and the whole diff `update` printed.

- [ ] **Step 4: Question 2 — timing**

Time the `update` run. The 153× in the measurements is synthetic and proves nothing about a 78,000-commit repository. Record the wall clock and, separately, the count of declared packages resolved.

- [ ] **Step 5: Question 3 — does the fetch actually change an answer?**

This is the property that most needs proving and is invisible when the buckets are already current. Induce it: reset a bucket's remote-tracking ref back a few commits (`git update-ref refs/remotes/origin/master <older>`), run `update`, confirm the pin moves back; then run it again and confirm the pin moves forward. **Restore the ref afterwards and verify it.** If this cannot be done without touching a bucket scoop owns in a way that cannot be undone, do not do it — report it as unexercised.

- [ ] **Step 6: Question 4 — does `update` disagree with the Phase 2b-1 rehearsal script?**

The prediction recorded in the design says it will, on at least one package, because the rehearsal resolved the *installed* version by walking history — `adopt`'s algorithm — while `update` resolves *latest*. Seven of 25 already had a matching commit that was not their bucket's HEAD. **If the two agree on all 25**, that is a claim about the machine (every declared package is already at its bucket's latest), not agreement, and it must be checked separately rather than accepted.

- [ ] **Step 7: Question 5 — is any declared package in more than one declared bucket?**

If none is, the ambiguity refusal never fires on this machine and `[scoop.opts] bucket` is documentation rather than a working path. **Say so.** Do not present an untriggered guard as a tested one.

- [ ] **Step 8: Question 6 — `adopt` on a genuinely unmanaged package**

`aichat` or `antigravity` are the candidates: installed, undeclared, unowned. Confirm afterwards that exactly three files changed, that `pkg.toml` is byte-identical except for the added line (comments and all), that `state.json` carries `"adopted"` and not `"installed"`, and that `dotpkg status` shows it as managed rather than as a prune.

Then **undo it**: restore `pkg.toml` from its `.bak`, remove the lock entry, and remove the state entry — leaving `%LOCALAPPDATA%\dotpkg` absent as it was. A `state.json` left behind saying dotpkg owns a package it did not install is exactly the trap Stage 2 of the 2b-2 dogfood caught.

- [ ] **Step 9: Question 7 — does `adopt` refuse cleanly?**

Find or construct a package whose installed version is not in its bucket's history. If it cannot be found on this machine, say that the failure mode had to be constructed rather than encountered, and construct it in a throwaway `$env:SCOOP` root. Confirm **no file at all** is written.

- [ ] **Step 10: Verify the machine**

Every app's version, the app count, the cache count, kanata's PID: identical to Step 1. Anything that moved is recorded as an observation, not explained away — Stage 2 of the 2b-2 dogfood recorded `explorer`'s PID changing without claiming to know why, and that is the standard.

- [ ] **Step 11: Clean up, and verify each removal individually**

`Test-Path` each path afterwards. Keep `C:\Users\kln\dotpkg-build` and `C:\Users\kln\pkg.toml`, matching every previous phase.

- [ ] **Step 12: Write `docs/dogfood-phase3-2026-08-XX.md`**

Frame it so it can fail. **A dogfood that confirms every expectation is a dogfood that was not trying** — that sentence is in the Phase 2b-2 document because it was earned. Record every falsified prediction by name, every contaminated measurement, and everything deliberately not covered.

If a14 is unreachable, **report BLOCKED and write nothing.** The Phase 2b-1 document exists because a previous attempt correctly produced no document rather than a guess.

- [ ] **Step 13: Commit**

```bash
git add -A && git commit -F - <<'EOF'
Record the Phase 3 dogfood

<Fill in from what actually happened, including the prediction about update
disagreeing with the 2b-1 rehearsal script -- confirmed or falsified by name --
and everything that could not be exercised.>
EOF
```

---

## Self-Review

Run against `docs/specs/2026-08-09-phase3-update-adopt-design.md`.

**1. Spec coverage.**

| Spec section | Task |
|---|---|
| Definition of `commit` | 4, 5 (the two flags), and the measurement doc |
| `update` fetches, never pulls | 3 (`fetch`, `tip`), 9 (driver + the branch-did-not-move test) |
| `--offline` | 9 |
| Bucket precedence and ambiguity | 6 |
| Filename resolution at the right rev | 3 (`resolve_spelling`), 4 (`manifest_path`) |
| `update`'s self-check against the tip | 4 |
| Lock validated by the reader's guard before writing | 7 |
| Failed re-resolve keeps the previous entry | 8 |
| `update`'s diff, including the `=` line | 8 (the variant), 9 (the rendering) |
| winget reported, not dropped | 8, 9 |
| `adopt` writes all three files | 12 |
| `adopt`'s `--full-history` + `cat-file --batch` | 5, 11 |
| Content match then version match | 11 |
| Refusal names shallowness, writes nothing | 12 |
| `pkg.toml` edited safely | 10 |
| Bulk `adopt`, no `--all` | 12 |
| `download` behind `Mutator` | 1 |
| Content-addressed staging | 13 |
| `status` runs the guard | 2 |
| Testing rules, negative controls | every task, audited in 14 |
| Dogfood questions 1–7 | 15 |

No gap found.

**2. Placeholder scan.** The only deliberate placeholder is the dogfood commit message in Task 15, which cannot be written before the run and says so.

**3. Type consistency.** `Resolution`, `Change`, `Scope`, `Update` (Task 8) are consumed unchanged in Task 9. `Latest` (Task 4), `BucketChoice`/`Tip` (Tasks 3, 6) are consumed in Tasks 9 and 12. `Found`/`Matched` (Task 11) are consumed in Task 12. `Outcome` in `adopt` is deliberately a different type from `apply::Outcome`; both are referred to by their module path at every use site in this plan.

One naming collision to watch during implementation: `src/render.rs` will import `crate::update::Change` and `crate::adopt::{Matched, Outcome as AdoptOutcome}` alongside the existing `crate::apply::Outcome`. The alias is in the code above; do not drop it.

**4. Ambiguity.** `update`'s exit code is 1 when any package could not be resolved and 0 otherwise, including when the lock was already current. `adopt`'s is 1 when any package was refused. Neither uses 2: neither command has a whole-run refusal that leaves the machine untouched in the way `apply`'s exit 2 means, except the `--state` relative-path check, which calls `refuse` and therefore exits 2 — consistent with `apply`.

