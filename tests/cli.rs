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
use std::process::{Child, Command, Output, Stdio};
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
    // The old assertion checked for the literal `scoop.cmd` path fragment,
    // which is macOS/Linux-only: measured on the real Windows machine, a
    // missing `<root>/shims/scoop.cmd` makes `Command::new(..).output()`
    // return `Ok` instead of `Err(NotFound)`, so the `.with_context(||
    // format!("cannot run {}", ...))` message that contains that string is
    // never produced there, for any run, refused or not. Its absence proved
    // nothing on Windows.
    //
    // `FAILED` is the marker `render_execution` prints for a genuine
    // mutation failure, on every platform (only the reason text after it is
    // platform-specific). This fixture is the exact same one-PRUNE shape as
    // `a_prune_authorised_by_both_flags_runs_and_records_the_release` (this
    // test's positive sibling): if the confirmation refusal below did not
    // fire, `execute` would call `Mutator::uninstall` on this same `aichat`
    // and that sibling test proves that produces a `FAILED` line on every
    // platform. Its absence here is therefore real, portable evidence that
    // `execute` never ran.
    assert!(
        !stderr.contains("FAILED") && !stdout.contains("FAILED"),
        "it must refuse before running scoop at all: stdout: {stdout} stderr: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn the_scoop_cmd_sentinel_is_reachable_or_the_assertion_above_proves_nothing() {
    // Production records a `Failed` outcome for `fzf` only once
    // `stage_and_fetch` has actually called `scoop.download` and
    // `download_verdict` (or the `Command` spawn itself) has ruled on it.
    // Without this sibling, the negative assertion above stays green even if
    // the whole executor is deleted.
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

    // The old assertion looked for the literal `scoop.cmd` path fragment
    // that `Scoop::download` puts into its error via
    // `.with_context(|| format!("cannot run {}", self.scoop_exe().display()))`
    // -- reached only when `Command::new(missing_path).output()` returns
    // `Err`. Measured on the real Windows machine (rustc 1.97.1):
    // `Command::new("C:\definitely\nope\scoop.cmd").arg("download").output()`
    // returns `Ok(status=Some(1), stdout="", stderr="The system cannot find
    // the path specified.")` instead of `Err(NotFound)` -- `cmd.exe` itself
    // swallows the missing file and reports it on stderr, which `download()`
    // never reads. So on Windows the `.with_context` branch never fires,
    // `download_verdict("")` finds no marker at all and returns `Unproven`,
    // and the message becomes "scoop download did not report a verified
    // hash for <manifest> (...): scoop printed nothing at all" -- sharing no
    // substring, `scoop.cmd` included, with the macOS/Linux "cannot run
    // <root>/shims/scoop.cmd: ..." message. Neither of those two strings is
    // usable as a cross-platform sentinel.
    //
    // What IS identical on both platforms: this fixture's bucket is a real
    // git commit matching the lock's pin, so `scoop.stage` succeeds
    // deterministically (relied on elsewhere, e.g.
    // `keep_going_does_not_report_success_when_a_declared_package_could_not_be_prepared`),
    // which leaves `scoop.download` as the only thing that can turn fzf's
    // `Outcome` into `Failed`. `render_preparation` renders that as
    // "FAILED  scoop  fzf ..." and folds it into the "N failed" summary
    // count regardless of what OS-specific text `download()` produced --
    // that structural fact, not the wording, is the sentinel now.
    assert!(
        all.contains("FAILED") && all.contains("fzf") && all.contains("1 failed"),
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
    // exit 1 (Important 6: a failure is outstanding whether or not it
    // changed anything -- 2 is reserved for a refusal before anything was
    // attempted, and this run genuinely tried), aichat still on disk, still
    // owned.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"fzf":"installed","aichat":"adopted"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");
    f.install_app("aichat", "0.30.0");

    let out = f.run(&["apply", "--yes", "--allow-prune"]);
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));

    // The old assertion looked for the literal `scoop.cmd` path fragment
    // that `Scoop::run` puts into its error via
    // `.with_context(|| format!("cannot run {}", self.scoop_exe().display()))`
    // -- reached only when spawning the process itself fails. Measured on
    // the real Windows machine: a missing `<root>/shims/scoop.cmd` makes
    // `Command::new(..).output()` return `Ok(status=Some(1), stdout="",
    // stderr="The system cannot find the path specified.")`, not `Err`. So
    // on Windows `m.uninstall(app)` in `run_step` never errors at all --
    // execution falls through to `verify::verdict`, which correctly finds
    // `aichat` still on disk, and the message becomes "aichat: uninstall
    // did not happen -- it is still on disk at <path>". That shares no
    // substring, `scoop.cmd` included, with the macOS/Linux "aichat: could
    // not run uninstall: cannot run <root>/shims/scoop.cmd: ..." message.
    //
    // What IS identical on both platforms: aichat is the only package this
    // fixture can ever mutate, and `run_step` only ever constructs a
    // `StepOutcome::Failed` for it after `m.uninstall` has returned
    // (successfully or not) and, on the Ok path, `verdict` has ruled --
    // i.e. after a genuine attempt. `render_execution` renders that as
    // "FAILED  scoop  aichat ..." on every platform; only the reason text
    // after it differs. That marker plus the app name, not the wording, is
    // the sentinel now.
    assert!(
        all.contains("FAILED") && all.contains("aichat"),
        "it must have tried: {all}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a package failed and needs a look, even though nothing changed: {all}"
    );
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
    // Important 3: recover.cmd is the one artifact that exists to survive a
    // run that dies, and this run failed -- it must not be cleaned up.
    assert!(
        f.local.path().join("dotpkg").join("recover.cmd").exists(),
        "a failed run must leave the recovery script in place"
    );
}

// -- Important 3: recover.cmd is only ever removed on a zero-failure run ---

#[test]
fn a_run_with_no_failures_removes_a_stale_recover_cmd() {
    // The positive control for the recover.cmd assertion above: without the
    // `if ex.failed() == 0` guard in main.rs, this file would survive every
    // run forever, offering to reinstall packages nobody touched tonight.
    // fzf has no lock entry, so the plan cannot become a step
    // (`Intent::NotLocked`, no scoop.cmd call anywhere) and `--keep-going`
    // lets the run proceed instead of refusing outright -- `execute` runs
    // for real, with zero steps, so `ex.failed()` is genuinely 0 even though
    // the overall preparation was not ok. `execute` itself overwrites
    // whatever is already at `recovery_path` the moment it runs (it writes
    // before the first mutation, unconditionally), so the marker content
    // seeded here only proves a file genuinely existed there for the
    // zero-failure guard to remove -- it is not what the assertion below is
    // checking for.
    let f = Fixture::new("[scoop]\npackages = [\"fzf\"]\n", "{}");
    let recover = f.local.path().join("dotpkg").join("recover.cmd");
    fs::write(&recover, "@echo off\r\nREM marker seeded by the test\r\n").unwrap();

    let out = f.run(&["apply", "--keep-going", "--yes"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "fzf could not be prepared, so the run is still outstanding: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        !recover.exists(),
        "execute() ran with zero step failures, so the stale recovery script must be removed"
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

// -- whole-branch review fix wave ----------------------------------------

#[test]
fn a_preparation_that_could_not_be_completed_refuses_before_execute_ever_runs() {
    // Important 2: deleting `!preparation.is_ok() && !keep_going` from
    // main.rs leaves the whole suite green. Live proof: with the gate
    // deleted, `apply --yes` with one unpreparable package reaches
    // `execute()` and writes state.json, where the real code exits 2 having
    // attempted nothing.
    //
    // fzf is declared with no lock entry, so `classify` returns
    // `Intent::NotLocked` and `prepare` never calls `stage_and_fetch` --
    // let alone `scoop.download` -- for it. There is nothing in this fixture
    // that could produce a `FAILED` marker on its own (fzf never becomes a
    // `Step` either way -- see below), so its total absence from the run's
    // output is part of the proof that this refusal fired before `execute`
    // (and before `prepare`'s download half) ever ran. The other part is
    // the exit code itself: with the gate deleted, `steps` and `held` both
    // stay empty (fzf never becomes a `Step`, so there is nothing for
    // `--keep-going`-without-the-gate to hold or run), `execute` is reached
    // with zero steps, and `main` still applies the could-not-be-prepared
    // floor -- exit 1, not 2. `the_scoop_cmd_sentinel_
    // is_reachable_or_the_assertion_above_proves_nothing` and
    // `a_prune_authorised_by_both_flags_runs_and_records_the_release` are
    // this test's positive siblings: both already prove a `FAILED` outcome
    // (naming `fzf` and `aichat` respectively, with a "N failed" count on
    // the first) DOES appear once `prepare` or `execute` genuinely reaches
    // scoop, so its absence here is not vacuous. `scoop.cmd` itself is not
    // usable for that proof -- see the comment on each sibling's assertion
    // for why the string differs (and on Windows, disappears) by platform.
    let f = Fixture::new("[scoop]\npackages = [\"fzf\"]\n", "{}");
    let before = f.snapshot();

    let out = f.run(&["apply", "--yes"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(2),
        "an unpreparable package without --keep-going must refuse before \
         execute runs, not report it as outstanding: stdout: {stdout} stderr: {stderr}"
    );
    // Same platform problem as every other `scoop.cmd` assertion in this
    // file (see `the_scoop_cmd_sentinel_is_reachable_or_the_assertion_above_
    // proves_nothing`'s assertion for the measured Windows behaviour): that
    // string is macOS/Linux-only, so checking its absence proved nothing on
    // Windows. `FAILED` is what both named siblings above prove is
    // producible, on every platform, the moment `prepare`'s download half
    // or `execute`'s mutator half is actually reached.
    assert!(
        !stdout.contains("FAILED") && !stderr.contains("FAILED"),
        "execute() -- and prepare()'s download half -- must never have run: \
         stdout: {stdout} stderr: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}

// -- a declared package skipped as running must not exit 0 --------------
//
// `SkipReason::Running`'s path-based signal (`Scoop::running_apps`, read
// straight from `src/backend/scoop.rs`) keys off a live process whose real
// executable sits under `<root>/apps/<name>/...`. That is read from the
// actual OS process table (`sysinfo`, via `dotpkg::sys::running_processes`),
// which is not a seam this suite may fake -- doing so would prove nothing
// about the code path this test exists to exercise. What follows instead
// spawns a REAL, live process at exactly that path: a second copy of the
// `dotpkg` binary itself, pointed at a second, disposable scoop root where
// it has one owned prune ready and no `--yes`, so it blocks forever inside
// `apply::confirm`'s `read_line` -- on a stdin pipe this fixture never
// writes to or closes -- for as long as `RunningMarker` is kept alive.

/// A real, live process whose own executable resolves under
/// `<scoop_root>/apps/<app>/...`, for as long as this value lives. `Drop`
/// kills it, so a failed assertion in the test that owns one never leaks a
/// blocked background process past the end of the run.
struct RunningMarker {
    child: Child,
    _work: TempDir,
    _scoop: TempDir,
    _local: TempDir,
}

impl Drop for RunningMarker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Copies the `dotpkg` binary under test to
/// `<scoop_root>/apps/<app>/current/`, then spawns that copy pointed at its
/// own, entirely separate scoop root and state -- one owned, installed,
/// undeclared package ("old"), which plans as a single ready `Prune` and
/// nothing else, so the spawned process reaches `apply`'s confirmation
/// prompt (no `--yes` is passed to it) and blocks there. Its own plan is
/// irrelevant to the test that calls this; only its executable's path and
/// the fact that it stays alive are.
fn spawn_running_marker(scoop_root: &Path, app: &str) -> RunningMarker {
    // Canonicalized before use, not just for tidiness: `Scoop::new` (what the
    // `apply` under test actually runs against) canonicalizes `$SCOOP` too
    // (`resolve_root`, `src/backend/scoop.rs`), and on macOS a `tempfile`
    // directory lives under `/var`, which is itself a symlink to
    // `/private/var`. `sysinfo` reports a live process's executable path
    // fully resolved -- `/private/var/...` -- so without matching that here,
    // the marker's own reported path and the scoop root `running_apps`
    // compares it against would differ only in that one symlink hop, and the
    // prefix match this whole fixture depends on would silently never fire.
    let scoop_root = scoop_root
        .canonicalize()
        .expect("the fixture's scoop root must already exist");
    let marker_dir = scoop_root.join("apps").join(app).join("current");
    fs::create_dir_all(&marker_dir).unwrap();
    let marker_bin = marker_dir.join("dotpkg.exe");
    fs::copy(env!("CARGO_BIN_EXE_dotpkg"), &marker_bin)
        .expect("the compiled dotpkg binary must be copyable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&marker_bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let work = tempfile::tempdir().unwrap();
    let scoop = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    fs::write(work.path().join("pkg.toml"), "").unwrap();
    let state_dir = local.path().join("dotpkg");
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(
        state_dir.join("state.json"),
        r#"{"scoop":{"old":"adopted"}}"#,
    )
    .unwrap();
    let cur = scoop.path().join("apps").join("old").join("current");
    fs::create_dir_all(&cur).unwrap();
    fs::write(cur.join("manifest.json"), r#"{"version":"0.1.0"}"#).unwrap();

    let child = Command::new(&marker_bin)
        .args(["apply", "--allow-empty-config"])
        .current_dir(work.path())
        .env("SCOOP", scoop.path())
        .env("LOCALAPPDATA", local.path())
        .env_remove("XDG_STATE_HOME")
        // Piped and never written to or closed: `confirm()` reading EOF
        // (`Ok(0)`) would read as a "no" answer and let the child exit
        // immediately, which is the one thing this fixture must not do
        // while the outer `apply` under test is sampling the process table.
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the copied dotpkg binary must be spawnable");
    RunningMarker {
        child,
        _work: work,
        _scoop: scoop,
        _local: local,
    }
}

#[test]
fn a_declared_package_skipped_as_running_is_outstanding_not_success() {
    // The hole: `SkipReason::Running` -> `Intent::Skip` -> `Outcome::Skipped`
    // never fails `Preparation::is_ok()` (deliberately -- a running package
    // must not gate a removal or refuse the run) and never becomes a `Step`,
    // so `execute` never sees it either. Before the `main.rs` fix, nothing
    // put it in `ex.results`, and `ex.exit_code(false)` returned 0 for a run
    // that left a declared package entirely untouched.
    //
    // aichat is declared and locked at 2.0.0 but installed at 1.0.0, so a
    // version change would normally be planned -- except its own executable
    // is a genuinely live process under this fixture's own scoop root, which
    // routes it to `Action::Skip { reason: Running }` instead.
    let f = Fixture::new("[scoop]\npackages = [\"aichat\"]\n", "{}");
    fs::write(
        f.work.path().join("pkg.lock"),
        format!(
            "[scoop.aichat]\nbucket = \"main\"\ncommit = \"{}\"\nversion = \"2.0.0\"\n",
            "a".repeat(40)
        ),
    )
    .unwrap();
    f.install_app("aichat", "1.0.0");
    let _marker = spawn_running_marker(f.scoop.path(), "aichat");

    // `--yes` is safe here and changes nothing about what is under test: the
    // plan has zero installs, replacements or removals (aichat is Skipped,
    // not Replaced), so there is no prune for `--yes` to fast-path around --
    // only the trivial "0/0/0, continue?" question, which this bypasses.
    let out = f.run(&["apply", "--yes"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a declared package left skipped-as-running is outstanding work, not success: \
         stdout: {stdout} stderr: {stderr}"
    );
    // The preparation table (printed before the confirmation question) is
    // not what this pins -- it already named the skip before this fix
    // existed. This is the CLOSING table, `render_execution`'s output,
    // printed after `execute` returns: the exact line `main.rs` must push
    // into `Execution` for the exit code to be explainable at all.
    let closing_line = format!(
        "  held    scoop  {:<13}running -- stop it first\n",
        "aichat"
    );
    assert!(
        stdout.contains(&closing_line),
        "the closing table must name the skipped package, not just count it: {stdout}"
    );
    assert!(
        stdout.contains("0 verified on disk, 0 failed, 1 held."),
        "the held count in the closing table must reflect the skip: {stdout}"
    );
    assert!(
        !stdout.contains("FAILED") && !stderr.contains("FAILED"),
        "a running skip is benign, not a failure: stdout: {stdout} stderr: {stderr}"
    );
    // The package that was never touched must still be exactly where it
    // was -- a running skip is a refusal to act, not a partial one.
    assert!(
        f.scoop.path().join("apps").join("aichat").exists(),
        "a skipped package must not have been removed"
    );
}

#[test]
fn apply_prepare_also_reports_a_running_skip_as_outstanding() {
    // The same hole, one command over: `--prepare` exits on
    // `!preparation.is_ok()` alone, which a running skip never fails
    // (deliberately -- see `Preparation::running_skips`'s doc comment).
    // Before this fix, `--prepare` against the exact same pkg.toml, lock and
    // machine as the full-apply test above reported exit 0 while the full
    // run reported 1 -- the same fact, the same skip, two disagreeing exit
    // codes for a user who reasonably expects `status`/`--prepare`/`apply`
    // to agree.
    let f = Fixture::new("[scoop]\npackages = [\"aichat\"]\n", "{}");
    fs::write(
        f.work.path().join("pkg.lock"),
        format!(
            "[scoop.aichat]\nbucket = \"main\"\ncommit = \"{}\"\nversion = \"2.0.0\"\n",
            "a".repeat(40)
        ),
    )
    .unwrap();
    f.install_app("aichat", "1.0.0");
    let _marker = spawn_running_marker(f.scoop.path(), "aichat");
    let before = f.snapshot();

    let out = f.run(&["apply", "--prepare"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "--prepare must agree with the full apply run over the identical \
         pkg.toml and machine: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("Nothing has been changed."),
        "--prepare's own promise must still hold -- it genuinely changed \
         nothing, so 2 would be as wrong as 0 here: {stdout}"
    );
    // `prepared_line`'s `Outcome::Skipped` arm already named this before
    // either fix existed -- pinned here so the exit code stays explainable
    // on this branch too, matching the format `render_preparation`'s own
    // test (`src/render.rs`) pins for the identical shape.
    assert!(
        stdout.contains("  !       scoop  aichat       running -- stop it first\n"),
        "the preparation table must name the skipped package: {stdout}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn status_says_so_when_the_lock_is_one_apply_would_refuse() {
    // The worst pairing available before this: status prints an actionable
    // plan, apply exits 2 on the same two files. `status` still prints the
    // plan -- it is read-only and its whole product is telling the truth --
    // but it no longer does so in silence.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n",
        "{}",
    );
    fs::write(
        f.work.path().join("pkg.lock"),
        "[scoop.tool]\nbucket = \"main\"\ncommit = \"main\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();

    let out = f.run(&["status"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(0),
        "status stays read-only and never refuses: {stderr}"
    );
    assert!(
        stderr.contains("not a commit hash"),
        "name the diagnosis: {stderr}"
    );
    assert!(
        stderr.contains("dotpkg update"),
        "name the command that fixes it -- it exists now: {stderr}"
    );
    // The guard's own error (printed above) already says the lock is
    // malformed and names "dotpkg update" -- it does not say anything about
    // `apply`. This substring exists only in the second `eprintln!`, so it is
    // the one thing here that pins that line rather than being satisfied by
    // the first line's text alone.
    assert!(
        stderr.contains("not what apply would do"),
        "name the relationship between the two commands, not just the lock's own defect: {stderr}"
    );
    // The plan is still printed. Without this, making status bail would pass
    // the assertions above while removing the thing status is for.
    assert!(
        stdout.contains("tool"),
        "the plan must still be printed: {stdout}"
    );
}

// -- Task 14: `update` and `adopt` end to end -----------------------------
//
// Added by the whole-branch review, and found by MUTATION rather than by
// reading: before these, `tests/cli.rs` invoked only `apply` and `status`, so
// every exit-code decision in the `Update` and `Adopt` arms of `main.rs` was
// unreachable from the suite. cargo-mutants reported five survivors there --
// `main.rs:438` (the undeclared-package refusal), `main.rs:459` (three
// mutants on `failed_count() > 0`), `main.rs:470` (the relative `--state`
// refusal) and `main.rs:496` (the refusal exit) -- all of which are killed by
// the tests below.
//
// The exit code IS the product for these commands: `update` runs unattended
// and a scheduled task learns "a package could not be re-resolved" only from
// exit 1. This is the ledger's THIRD PATTERN -- the coverage hole sits
// exactly where the output meets a human or the next command -- so the pairs
// below are deliberate: a refusal test on its own is satisfied by an
// implementation that always fails, so each is paired with a positive
// sibling that must stay green.

/// A bucket with no `pkg.lock` alongside it: `update` writes its own.
fn bucket_only(f: &Fixture, app: &str, version: &str) {
    f.write_lock_and_bucket_for(app, version);
    fs::remove_file(f.work.path().join("pkg.lock")).unwrap();
}

#[test]
fn update_resolves_a_declared_package_and_exits_zero() {
    // The positive sibling for both refusal tests below. Without it, an
    // `update` that always exited 1 -- or always refused -- would satisfy
    // every assertion the two negative tests make.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        "{}",
    );
    bucket_only(&f, "fzf", "1.0.0");

    let out = f.run(&["update"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(0),
        "a run in which everything resolved must exit 0: stdout: {stdout} stderr: {stderr}"
    );
    let lock = fs::read_to_string(f.work.path().join("pkg.lock"))
        .expect("update must have written pkg.lock");
    assert!(lock.contains("[scoop.fzf]"), "{lock}");
    // `bucket_only` commits exactly one version, `1.0.0`. The `|| "0.74.1"`
    // this used to carry could never fire and only made the assertion weaker.
    assert!(lock.contains("1.0.0"), "{lock}");
}

#[test]
fn update_exits_one_when_a_declared_package_could_not_be_reresolved() {
    // `failed_count() > 0`. A package that is declared but that no declared
    // bucket carries is kept, not dropped -- and the run is not a success.
    // Unattended, the exit code is the only way anyone finds out.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"nothere\"]\n",
        "{}",
    );
    bucket_only(&f, "fzf", "1.0.0");

    let out = f.run(&["update"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a package that could not be re-resolved is outstanding work: \
         stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("nothere"),
        "name the package that failed: {stdout}"
    );
}

#[test]
fn update_refuses_a_package_pkg_toml_does_not_declare_and_writes_nothing() {
    // `update` re-resolves what pkg.toml asks for; it is not a way to add a
    // package. A refusal exits 2, not 1: the machine was not touched.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        "{}",
    );
    bucket_only(&f, "fzf", "1.0.0");
    let before = f.snapshot();

    let out = f.run(&["update", "nothere"]);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(2),
        "an undeclared package is a refusal: {stderr}"
    );
    assert!(
        stderr.contains("nothere") && stderr.contains("pkg.toml"),
        "name the package and where to declare it: {stderr}"
    );
    // The counterweight: a refusal that had already rewritten the lock would
    // satisfy the assertions above unchanged.
    assert!(
        !f.work.path().join("pkg.lock").exists(),
        "a refused update must not write pkg.lock"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn adopt_brings_an_installed_package_under_management_and_exits_zero() {
    // The positive sibling for the two adopt refusals below, and the only
    // end-to-end proof that the three writes happen through the real CLI.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
        r#"{"scoop":{}}"#,
    );
    bucket_only(&f, "fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");

    let out = f.run(&["adopt", "fzf"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(0),
        "a successful adopt exits 0: stdout: {stdout} stderr: {stderr}"
    );
    let lock = fs::read_to_string(f.work.path().join("pkg.lock"))
        .expect("adopt must have written pkg.lock");
    assert!(lock.contains("[scoop.fzf]"), "{lock}");
    let cfg = fs::read_to_string(f.work.path().join("pkg.toml")).unwrap();
    assert!(cfg.contains("fzf"), "pkg.toml must now declare it: {cfg}");
    let state = fs::read_to_string(f.local.path().join("dotpkg").join("state.json")).unwrap();
    assert!(state.contains("fzf"), "state.json must own it: {state}");
}

#[test]
fn adopt_exits_one_when_a_package_is_refused() {
    // A refusal is per package and reported, but the run as a whole did not
    // do what was asked, so it is not a success.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
        r#"{"scoop":{}}"#,
    );
    bucket_only(&f, "fzf", "1.0.0");

    let out = f.run(&["adopt", "nothere"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a refused adopt is outstanding work: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("nothere") || stderr.contains("nothere"),
        "name the package: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        !f.work.path().join("pkg.lock").exists(),
        "a refused adopt writes nothing"
    );
}

#[test]
fn adopt_prints_what_the_scan_could_not_read_before_calling_a_package_uninstalled() {
    // Found by the Phase 3 dogfood on a14, not by review: `dotpkg adopt
    // antigravity` printed "antigravity is not installed" about a package
    // that was installed. The scan could not traverse its junction, so it was
    // absent from the scan, and `adopt` was the one command that dropped
    // `scan.warnings` -- `status`, `apply` and `update` have each printed
    // them since Phase 2a. The refusal line is not wrong given what dotpkg
    // could see; it is unactionable without the warning that says why.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
        r#"{"scoop":{}}"#,
    );
    bucket_only(&f, "fzf", "1.0.0");

    // `ghost` is installed -- `Fixture::new` created it -- but its
    // manifest.json is not JSON, so `scan` cannot see it. That is the
    // portable stand-in for the junction the dogfood actually hit.
    let out = f.run(&["adopt", "ghost"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "still a refusal: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("is not installed"),
        "the refusal itself is unchanged: {stdout}"
    );
    assert!(
        stderr.contains("warning: scoop: ghost: manifest.json is not usable"),
        "the refusal above is false on its own -- naming what could not be \
         read is what makes it actionable: stderr: {stderr}"
    );
}

#[test]
fn adopt_prints_no_warning_when_the_scan_read_everything() {
    // The counterweight to the test above. Without it, an implementation
    // that printed a warning on every run would satisfy that `contains` and
    // teach the user to ignore the line.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
        r#"{"scoop":{}}"#,
    );
    fs::remove_dir_all(f.scoop.path().join("apps").join("ghost")).unwrap();
    bucket_only(&f, "fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");

    let out = f.run(&["adopt", "fzf"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        !stderr.contains("warning:"),
        "a run that read everything has nothing to warn about: {stderr}"
    );
}

#[test]
fn adopt_refuses_a_relative_state_path_before_anything_runs() {
    // The same rule `apply` has, on the other command that writes state.json.
    // Verified as a refusal (exit 2), not a `?` propagation (exit 1).
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
        r#"{"scoop":{}}"#,
    );
    bucket_only(&f, "fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");
    let before = f.snapshot();

    let out = f.run(&["adopt", "--state", "some/relative/path.json", "fzf"]);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(2),
        "a relative --state path is a refusal, and a refusal exits 2: {stderr}"
    );
    assert!(
        stderr.contains("absolute"),
        "say what is wrong with it: {stderr}"
    );
    assert!(
        !f.work.path().join("pkg.lock").exists(),
        "the refusal must land before any write"
    );
    f.assert_nothing_was_touched(before);
}
