//! The three safety behaviours of `apply` that only exist in `main.rs`, and
//! that nothing else in the suite can reach.
//!
//! Measured before this file existed: deleting the `--prepare` guard, the
//! mass-prune guard call, and the `exit(1)` from `main.rs` -- all three, at
//! once -- left the suite at 127/127 green. "`apply` without `--prepare`
//! changes nothing" is the single promise that makes this branch safe to merge
//! before the executor exists, and Phase 2b-2 rewrites that exact `match` arm.
//!
//! The binary is invoked for real via `CARGO_BIN_EXE_dotpkg`, which Cargo sets
//! for integration tests, so no new dependency is needed. Every run is
//! hermetic: `SCOOP` and `LOCALAPPDATA` point at temporary directories and the
//! working directory is a temporary directory, so nothing reads the developer's
//! own machine and nothing is written outside the fixture.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

/// A scoop root, a state directory and a working directory, wired together.
struct Fixture {
    work: TempDir,
    scoop: TempDir,
    local: TempDir,
}

impl Fixture {
    /// `config` is written as `pkg.toml`; `state` as `state.json` under
    /// `%LOCALAPPDATA%\dotpkg`. No `pkg.lock` is written -- these tests are
    /// about the paths that never get that far, plus the unlocked case.
    fn new(config: &str, state: &str) -> Fixture {
        let f = Fixture {
            work: tempfile::tempdir().unwrap(),
            scoop: tempfile::tempdir().unwrap(),
            local: tempfile::tempdir().unwrap(),
        };
        fs::write(f.work.path().join("pkg.toml"), config).unwrap();
        let state_dir = f.local.path().join("dotpkg");
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(state_dir.join("state.json"), state).unwrap();

        // The sentinel that makes "did it scan?" observable. An app directory
        // whose manifest.json is unreadable JSON makes `scan()` emit
        // `warning: scoop: ghost: manifest.json is not usable` on stderr --
        // and nothing else in a run produces the word "ghost". A run that
        // reaches the scan says it; a run that stopped earlier cannot.
        let ghost = f.scoop.path().join("apps").join("ghost").join("current");
        fs::create_dir_all(&ghost).unwrap();
        fs::write(ghost.join("manifest.json"), "{ this is not json").unwrap();
        f
    }

    fn run(&self, args: &[&str]) -> Output {
        // A `#!/bin/sh` file at this path would buy a green "end-to-end" test
        // on macOS, where `execve` ignores the `.cmd`, that means something
        // entirely different on a Windows runner. Checked here rather than by
        // scanning the test sources, which cannot tell a comment from a call.
        assert!(
            !self.scoop.path().join("shims").join("scoop.cmd").exists(),
            "no test may provide a fake scoop binary"
        );
        Command::new(env!("CARGO_BIN_EXE_dotpkg"))
            .args(args)
            .current_dir(self.work.path())
            .env("SCOOP", self.scoop.path())
            .env("LOCALAPPDATA", self.local.path())
            .env_remove("XDG_STATE_HOME")
            .output()
            .expect("the dotpkg binary must be runnable")
    }

    /// Proof that nothing was installed, uninstalled or otherwise written into
    /// the scoop root or the state directory by the run.
    fn assert_nothing_was_touched(&self, before: Snapshot) {
        assert_eq!(
            before,
            Snapshot::of(self.scoop.path(), self.local.path()),
            "the run changed something on disk"
        );
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot::of(self.scoop.path(), self.local.path())
    }
}

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

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

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

#[test]
fn apply_without_prepare_refuses_and_names_the_phase_that_will_add_the_executor() {
    // The promise the whole branch rests on. Without this guard, `apply`
    // would fall through to the prepare path and behave as `--prepare`
    // without being asked to -- doing work the user did not request, and
    // reporting a plan as though it had been carried out.
    let f = Fixture::new(
        "[scoop]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"fzf":"installed"}}"#,
    );
    let before = f.snapshot();

    let out = f.run(&["apply"]);
    let stderr = text(&out.stderr);
    let stdout = text(&out.stdout);

    assert!(
        !out.status.success(),
        "apply with no executor must fail, not quietly succeed: {stdout}"
    );
    assert!(
        stderr.contains("2b-2"),
        "say which phase brings the executor: {stderr}"
    );
    assert!(
        stderr.contains("--prepare"),
        "say what the user can do instead: {stderr}"
    );
    assert!(
        !stdout.contains("Nothing has been changed."),
        "it must stop before the preparation runs at all, not report on one: {stdout}"
    );
    assert!(
        !stderr.contains("ghost"),
        "it must refuse before scanning the machine: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn an_empty_config_is_refused_before_the_machine_is_even_scanned() {
    // Ordering is the property, not just the refusal: the guard exists
    // because a truncated pkg.toml turns every owned package into a prune,
    // and it must fire before anything else reads the machine or does work.
    let f = Fixture::new("", r#"{"scoop":{"fzf":"installed","bat":"adopted"}}"#);
    let before = f.snapshot();

    let out = f.run(&["apply", "--prepare"]);
    let stderr = text(&out.stderr);

    assert!(!out.status.success(), "an empty config must not proceed");
    assert!(
        stderr.contains("--allow-empty-config"),
        "say how to override it: {stderr}"
    );
    assert!(
        stderr.contains('2'),
        "the owned count is the whole point of the message: {stderr}"
    );
    assert!(
        !stderr.contains("ghost"),
        "the guard must run BEFORE the scan -- the scan's own warning proves it did not: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn a_preparation_that_could_not_be_completed_exits_non_zero_and_says_nothing_changed() {
    // A declared package with no lock entry. `apply --prepare` cannot resolve
    // a version itself, so the run has failed -- and a caller (a script, CI)
    // learns that only from the exit code.
    //
    // This also proves the "ghost" sentinel the two tests above rely on
    // negatively is observable at all: this run does reach the scan, and does
    // print it.
    let f = Fixture::new("[scoop]\npackages = [\"fzf\"]\n", "{}");
    let before = f.snapshot();

    let out = f.run(&["apply", "--prepare"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a preparation that is not ok must exit 1; stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("Nothing has been changed."),
        "the promise must be printed even on the failing path: {stdout}"
    );
    assert!(
        stdout.contains("no lock entry"),
        "say what is wrong with fzf: {stdout}"
    );
    assert!(
        stderr.contains("ghost"),
        "the sentinel must be reachable, or the two negative assertions above prove nothing: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}
