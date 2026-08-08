//! Safety behaviours of `apply` that only exist in `main.rs`, and that
//! nothing else in the suite can reach: the mass-prune guard running before
//! the machine is scanned, `--prepare`'s "Nothing has been changed." promise,
//! the confirmation prompt, the `--allow-prune` gate, and the rule that a
//! removal only ever runs when the whole preparation came back ok.
//!
//! Phase 2b-1 also pinned a hard, unconditional refusal for `apply` without
//! `--prepare` here -- deliberate, because the executor did not exist yet.
//! Phase 2b-2 (this file, now) replaces that exact `match` arm with a real
//! executor, so that guard and the test that pinned it are both gone; what
//! replaced it is pinned by the tests below instead, most directly by
//! `apply_with_no_answer_available_refuses_and_changes_nothing`, which runs
//! the same bare `apply` that guard used to refuse outright and checks it
//! now reaches the confirmation prompt and refuses *there*.
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
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "x",
        ]);
        let sha = git(&["rev-parse", "HEAD"]).trim().to_string();
        fs::write(
            self.work.path().join("pkg.lock"),
            format!(
                "[scoop.{app}]\nbucket = \"main\"\ncommit = \"{sha}\"\nversion = \"{version}\"\n"
            ),
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

// -- the executor: the confirmation prompt, --allow-prune, and the removals
// gate -----------------------------------------------------------------
//
// Read this constraint first, because it decides every fixture below: on
// macOS there is no scoop binary, so any action needing an artifact fails at
// `download` and makes `preparation.is_ok()` false. A run that reaches the
// prompt at all must therefore have a plan containing only prunes and
// reports. Any fixture with an `Install` in it exits at the "could not be
// prepared" branch instead -- a different code path, and a test that does
// not know which one it exercised proves nothing.

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
    assert!(
        stderr.contains("--yes"),
        "say what to pass instead: {stderr}"
    );
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
    assert!(
        all.contains("scoop.cmd"),
        "the sentinel must be reachable: {all}"
    );
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

    // Tightened from `assert_ne!(.., Some(0))`, which is exactly loose
    // enough to have missed a real bug: this gate used to `anyhow::bail!`,
    // which only `main() -> Result<()>` can ever turn into exit 1 -- and
    // `assert_ne!(Some(1), Some(0))` is also true. 2 is "refused, nothing
    // changed", which is the whole reason the code exists.
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("--allow-prune"), "{stderr}");
    assert!(
        !stderr.contains("could not be prepared"),
        "this must fail on the prune gate, not on an unprepared package: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn a_prune_authorised_by_both_flags_runs_and_records_the_release() {
    // The positive control for the two tests above: without it, a main.rs
    // that refuses every run passes both of them. This is also the only test
    // in the suite that drives a real mutation end to end, and it can only be
    // a prune -- a prune is the one step that needs no scoop binary to be
    // *planned*, though it still needs one to be *performed*, so the run
    // fails at the uninstall and the assertion is that it failed HONESTLY:
    // exit 2, aichat still on disk, still owned.
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

#[test]
fn keep_going_holds_a_ready_prune_back_when_another_package_could_not_be_prepared() {
    // Negative control 2's positive complement: a plan with one Failed
    // install (fzf, staged for real but download fails -- there is no scoop
    // binary here) and one ready prune (aichat, owned and installed but
    // undeclared). `--keep-going` lets the run continue past the failed
    // install instead of refusing outright, but must NOT also let the prune
    // through -- that gate answers to `preparation.is_ok()` alone, and
    // nothing else opens it.
    //
    // Deleting `gate_removals`'s guard (or the `main.rs` call to it) makes
    // this test go red: the "held" note below stops appearing, because the
    // prune would no longer be stripped out before the confirmation
    // question is built.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"aichat":"adopted"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("aichat", "0.30.0");
    // No `assert_nothing_was_touched` here: unlike the two prune-only tests
    // above, fzf's Install action makes `prepare` stage a real manifest under
    // `%LOCALAPPDATA%\dotpkg\manifests`, which is an intended, harmless write
    // to the staging area -- not evidence anything installed changed.
    let scoop_before = std::fs::read_dir(f.scoop.path().join("apps"))
        .unwrap()
        .count();

    let out = f.run(&["apply", "--keep-going"]);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(2),
        "unreadable stdin still refuses the run once it reaches the prompt: {stderr}"
    );
    assert!(
        stderr.contains("aichat") && stderr.contains("held"),
        "say which prune was held back and that it was held: {stderr}"
    );
    assert!(
        f.scoop.path().join("apps").join("aichat").exists(),
        "the held prune must not have been removed"
    );
    assert_eq!(
        std::fs::read_dir(f.scoop.path().join("apps"))
            .unwrap()
            .count(),
        scoop_before,
        "nothing under apps/ may change while the confirmation prompt is still unanswered"
    );
}

// -- fix round 1 ---------------------------------------------------------

#[test]
fn apply_on_a_converged_machine_exits_zero_without_asking() {
    // Important 4. Verified live before this fix: an empty plan still built
    // the question "0 packages will be uninstalled and reinstalled, 0
    // installed, 0 removed. Continue?", got an unreadable stdin exactly like
    // every other unattended run, and exited 2 -- "go look, something's
    // wrong" -- about a machine with nothing wrong with it, every night.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"fzf":"installed"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");
    let before = f.snapshot();

    let out = f.run(&["apply"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(0),
        "a converged machine must exit 0, not ask an unanswerable question: \
         stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        !stderr.contains("Continue?"),
        "there is nothing to decide, so nothing to ask: {stderr}"
    );
    assert!(
        stdout.contains("already matches"),
        "say plainly that there was nothing to do: {stdout}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn a_held_prune_appears_in_the_closing_table_not_only_as_a_stderr_note() {
    // Minor 6. Verified live before this fix: under `--keep-going --yes`
    // the closing table reported "0 held" while a prune really was held --
    // the `eprintln!` note satisfied "printed as held" at the moment it
    // happened, but the table a user actually reads at the end of the run
    // contradicted it minutes later.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"aichat":"adopted"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("aichat", "0.30.0");

    let out = f.run(&["apply", "--keep-going", "--yes"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert!(
        stdout.contains("held") && stdout.contains("aichat"),
        "the closing table must list the held prune, not just say 0 held: \
         stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        !stdout.contains("0 verified on disk, 0 failed, 0 held."),
        "the held count must reflect the real held prune: {stdout}"
    );
}

// -- fix round 2 ---------------------------------------------------------

#[test]
fn keep_going_does_not_report_success_when_a_declared_package_could_not_be_prepared() {
    // The gap this crate's own Task 12 report flagged and the coordinator's
    // re-review confirmed: a package that fails to PREPARE lands only in
    // `unusable` and never becomes a `Step`, so `Execution` -- and
    // therefore `ex.exit_code(false)` -- cannot see it at all. Here the
    // plan is exactly one Install (fzf, declared and locked but not
    // installed), which stages for real and fails at download because
    // there is no scoop binary on this platform. Nothing else is declared,
    // so `steps` ends up empty and `execute()` has nothing of its own to
    // fail on -- `ex.failed() == 0` is genuinely true, and the run must
    // still not report success.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        "{}",
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");

    // `--yes` is required to reach the exit-code computation at all: with
    // `unusable` non-empty the run does not take the converged-machine
    // short-circuit, and without `--yes` it would refuse at the
    // confirmation prompt on the closed stdin first (exit 2, a different
    // code entirely, and not what this test is about).
    let out = f.run(&["apply", "--keep-going", "--yes"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a run that left a declared package unprepared must not report success (0): \
         stdout: {stdout} stderr: {stderr}"
    );
}

#[test]
fn a_relative_state_path_is_refused_before_anything_runs() {
    // The one refusal path in `main.rs` with no automated test before this
    // -- the re-reviewer had to run the binary by hand to confirm it exits
    // 2. Cheap to close.
    let f = Fixture::new(
        "[scoop]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"fzf":"installed"}}"#,
    );
    let before = f.snapshot();

    let out = f.run(&["apply", "--state", "some/relative/path.json"]);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(2),
        "a relative --state path is a refusal, and a refusal exits 2: {stderr}"
    );
    assert!(
        stderr.contains("--state"),
        "name the flag that needs an absolute path: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}
