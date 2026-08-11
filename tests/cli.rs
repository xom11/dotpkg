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

mod common;

use common::fake_winget_mutator::FakeWingetMutator;
use dotpkg::execute::{ExecOptions, Step, WingetStep};
use dotpkg::model::{Name, Running};
use dotpkg::state::State;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use tempfile::TempDir;

/// The `PATH` this fixture hands to the spawned `dotpkg` process, with every
/// directory that carries a `winget`/`winget.exe` binary removed.
///
/// Task 14's review caught two tests asserting "winget is absent" as a
/// property of the DEVELOPER'S machine rather than of the fixture:
/// `Fixture::run` already overrides `SCOOP`/`LOCALAPPDATA`/`XDG_STATE_HOME`
/// but inherited the host `PATH` unfiltered, so on any machine that
/// genuinely has winget installed -- which includes the Windows machine a
/// later task runs this suite on -- `winget list` would succeed for real,
/// and both `Winget::scan` warning assertions would go red, one of them
/// (`status`'s "nothing to do") for a reason that has nothing to do with
/// winget at all: every real installed winget package would be undeclared
/// and unowned and become a stray `Action::Unmanaged` report.
///
/// Filtered by directory CONTENT, not by a hardcoded typical install path
/// (`winget.exe` usually lives under `%LOCALAPPDATA%\Microsoft\WindowsApps`,
/// but nothing requires that): this stays correct regardless of where
/// winget actually lives, and leaves every OTHER tool on `PATH` --
/// `git`, which the `dotpkg` binary itself shells out to for real from
/// `src/bucket.rs` during staging -- untouched. On a machine with no winget
/// anywhere on `PATH` at all (every machine this suite has actually run on
/// so far), this is a no-op: nothing is filtered out.
fn path_without_winget() -> std::ffi::OsString {
    let original = std::env::var_os("PATH").unwrap_or_default();
    let filtered: Vec<_> = std::env::split_paths(&original)
        .filter(|dir| !dir.join("winget.exe").exists() && !dir.join("winget").exists())
        .collect();
    // `join_paths` only fails if a directory itself contains the platform
    // path-separator byte -- fall back to the unfiltered original rather
    // than an empty PATH, which would break `git` resolution too.
    std::env::join_paths(filtered).unwrap_or(original)
}

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
            .env("PATH", path_without_winget())
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

#[test]
fn a_ready_prune_with_nothing_held_back_gets_no_routing_bug_warning() {
    // `unrouted_warning(preparation.ready_count(), steps.len() + held.len())`
    // (main.rs:602). A single adopted, undeclared package with nothing else
    // declared routes to exactly one step and zero held items, so the two
    // numbers already agree without needing addition's identity element to
    // save them -- a product would not: with `held.len() == 0`,
    // `steps.len() * held.len()` is 0 regardless of how many steps there
    // really are, and the spurious "routing bug" warning would fire on a
    // run that routed everything correctly.
    let f = Fixture::new("", r#"{"scoop":{"aichat":"adopted"}}"#);
    f.install_app("aichat", "0.30.0");

    let out = f.run(&["apply", "--allow-empty-config"]);
    let stderr = text(&out.stderr);

    assert!(
        !stderr.contains("routing bug"),
        "one step, nothing held -- the one ready package was routed: {stderr}"
    );
}

#[test]
fn a_held_prune_with_no_other_steps_gets_no_routing_bug_warning_either() {
    // The mirror: `held.len() > 0` while `steps.len() == 0` zeroes a
    // product just as well as the other operand being zero does. Same setup
    // as `a_held_prune_appears_in_the_closing_table_not_only_as_a_stderr_
    // note` above -- fzf fails to prepare (no scoop binary here), so
    // aichat's otherwise-ready prune is held rather than routed -- but this
    // test's own claim is narrower: not that the closing table is right,
    // but that nothing printed the ROUTING-bug warning along the way, which
    // a product-shaped `steps.len() + held.len()` would.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"aichat":"adopted"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("aichat", "0.30.0");

    let out = f.run(&["apply", "--keep-going"]);
    let stderr = text(&out.stderr);

    assert!(
        stderr.contains("held"),
        "sanity: the prune really was held, or this test proves nothing: {stderr}"
    );
    assert!(
        !stderr.contains("routing bug"),
        "one held prune, zero built steps -- the one ready package was still accounted \
         for: {stderr}"
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
        "  held    scoop  {:<14} running -- stop it first\n",
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
    // (deliberately -- see `Preparation::outstanding_skips`'s doc comment).
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
    // test (`src/render.rs`) pins for the identical shape. The name column is
    // one wider than it used to be (`src/render.rs:229`, Task 16's Windows
    // fix wave) so a real winget id no longer runs into what follows it.
    assert!(
        stdout.contains("  !       scoop  aichat         running -- stop it first\n"),
        "the preparation table must name the skipped package: {stdout}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn a_declared_package_whose_manifest_is_unreadable_is_outstanding_not_success() {
    // The Opaque half of the hole caught in review of Task 4's own fix:
    // `SkipReason::Opaque` -> `Intent::Skip` -> `Outcome::Skipped` has the
    // identical shape as `Running` (never fails `is_ok()`, never becomes a
    // `Step`), but `Preparation::running_skips` (as it existed right after
    // Task 4) recognised only `Running` by name -- so a package whose state
    // dotpkg could not establish vanished from the closing table and the run
    // reported exit 0, exactly the "0 lies to a scheduled task" shape
    // `floor_exit_code`'s own doc comment warns against.
    //
    // No pkg.lock is written: `plan()`'s opaque check fires before the lock
    // lookup, so aichat never needs one to reach `Action::Skip { Opaque }`.
    let f = Fixture::new("[scoop]\npackages = [\"aichat\"]\n", "{}");
    let cur = f.scoop.path().join("apps").join("aichat").join("current");
    // A DIRECTORY at manifest.json -- portable, needs no chmod, and is the
    // exact technique `tests/scoop_scan.rs`'s opaque test uses.
    fs::create_dir_all(cur.join("manifest.json")).unwrap();

    let out = f.run(&["apply", "--yes"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a declared package whose state could not be read is outstanding work, \
         not success: stdout: {stdout} stderr: {stderr}"
    );
    let closing_line = format!(
        "  held    scoop  {:<14} installed, but its state could not be read -- see the warnings above\n",
        "aichat"
    );
    assert!(
        stdout.contains(&closing_line),
        "the closing table must name the opaque package with its own reason, \
         not just count it: {stdout}"
    );
    assert!(
        stdout.contains("0 verified on disk, 0 failed, 1 held."),
        "the held count in the closing table must reflect the skip: {stdout}"
    );
    assert!(
        !stdout.contains("FAILED") && !stderr.contains("FAILED"),
        "an opaque skip is benign, not a failure: stdout: {stdout} stderr: {stderr}"
    );
    // The package that was never touched must still be exactly where it
    // was -- an opaque skip is a refusal to act, not a partial one.
    assert!(
        f.scoop.path().join("apps").join("aichat").exists(),
        "a skipped package must not have been removed"
    );
}

#[test]
fn apply_prepare_also_reports_an_opaque_skip_as_outstanding() {
    // The `--prepare` mirror of the test above, matching
    // `apply_prepare_also_reports_a_running_skip_as_outstanding`: `--prepare`
    // exits on `!preparation.is_ok()` alone, which an opaque skip never
    // fails, so this is the other place the same query had to be widened.
    let f = Fixture::new("[scoop]\npackages = [\"aichat\"]\n", "{}");
    let cur = f.scoop.path().join("apps").join("aichat").join("current");
    fs::create_dir_all(cur.join("manifest.json")).unwrap();
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
    assert!(
        stdout.contains(
            "  !       scoop  aichat         installed, but its state could not be read -- \
             see the warnings above\n"
        ),
        "the preparation table must name the opaque package: {stdout}"
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

// -- Phase 5 task 5: collapsing Unmanaged, wired into main.rs's own flag ----
//
// `render::render`'s own unit tests (`src/render.rs`) pin the collapsing and
// the summary clause as pure functions of a hand-built `Plan`. What only this
// file can pin is the wiring: that `main.rs` actually declares `--show-
// unmanaged` on `Status` and threads it into `render::render` rather than a
// hardcoded `false` -- a library-linked test binary cannot observe `main.rs`
// at all, so this is the only place a regression in that wiring could ever
// turn red.

#[test]
fn show_unmanaged_restores_the_individual_line_that_the_default_run_collapses() {
    // No package declared at all, so `status` has nothing to say about
    // anything except the one undeclared, unowned scoop app this fixture
    // installs -- `Fixture::new`'s own `ghost` app is unreadable JSON and
    // lands in `opaque`, never in `installed`, so it cannot become a second
    // `Action::Unmanaged` and confuse this fixture's count.
    let f = Fixture::new("", "{}");
    f.install_app("stray-tool", "1.0.0");

    let default_out = f.run(&["status"]);
    let shown_out = f.run(&["status", "--show-unmanaged"]);

    assert_eq!(default_out.status.code(), Some(0), "{:?}", default_out);
    assert_eq!(shown_out.status.code(), Some(0), "{:?}", shown_out);

    let default_stdout = text(&default_out.stdout);
    let shown_stdout = text(&shown_out.stdout);

    // The default run: one collapsed line naming the backend and the count,
    // a hint to pass the flag, the summary clause -- and never the package's
    // own name, which is exactly what "collapsed" promises.
    assert!(
        default_stdout.contains("? scoop    1 installed outside dotpkg"),
        "was:\n{default_stdout}"
    );
    assert!(
        default_stdout.contains("--show-unmanaged"),
        "the default output must hint at the flag that restores detail: \
         {default_stdout}"
    );
    assert!(
        !default_stdout.contains("stray-tool"),
        "collapsed means collapsed -- the package's own name must not \
         survive: {default_stdout}"
    );
    assert!(
        default_stdout.contains("0 change(s), 0 skipped, 1 unmanaged"),
        "the summary clause is mandatory, not merely the collapsed line: \
         {default_stdout}"
    );

    // `--show-unmanaged`: the individual line comes back, named, and the
    // hint -- which would now be advice to do what this run already did --
    // disappears.
    assert!(shown_stdout.contains("stray-tool"), "was:\n{shown_stdout}");
    assert!(
        shown_stdout.contains("(unmanaged -- no action)"),
        "was:\n{shown_stdout}"
    );
    assert!(
        !shown_stdout.contains("--show-unmanaged"),
        "the hint must not appear once the flag it advertises was already \
         passed: {shown_stdout}"
    );
    // The clause survives in both forms: the count is true regardless of how
    // the facts underneath it are displayed.
    assert!(
        shown_stdout.contains("0 change(s), 0 skipped, 1 unmanaged"),
        "was:\n{shown_stdout}"
    );
}

#[test]
fn apply_prepare_show_unmanaged_restores_the_individual_line_in_both_of_its_tables() {
    // Review Important 1: a full `apply` run, and `--prepare` (which prints
    // the same two tables and then stops), print `render(plan)`'s table AND
    // `render_preparation`'s table for the same run. Until this fix only the
    // first respected `--show-unmanaged` -- the second printed every
    // individual line regardless of the flag, so the default run's hint
    // ("pass --show-unmanaged to list them") sat two lines above the exact
    // list it claimed the flag alone would produce. Hardcoding `false` at
    // the `render_preparation` call site could not have turned any test red
    // before this one existed -- `grep show_unmanaged tests/` had exactly
    // one hit, and it only ever ran `status`.
    let f = Fixture::new("", "{}");
    f.install_app("stray-tool", "1.0.0");

    let default_out = f.run(&["apply", "--prepare"]);
    let shown_out = f.run(&["apply", "--prepare", "--show-unmanaged"]);

    assert_eq!(default_out.status.code(), Some(0), "{:?}", default_out);
    assert_eq!(shown_out.status.code(), Some(0), "{:?}", shown_out);

    let default_stdout = text(&default_out.stdout);
    let shown_stdout = text(&shown_out.stdout);

    // Both tables collapse the same way by default: one collapsed line and
    // one hint from `render(plan)`, one collapsed line and one hint from
    // `render_preparation` -- never the package's own name from either.
    assert_eq!(
        default_stdout
            .matches("? scoop    1 installed outside dotpkg")
            .count(),
        2,
        "one collapsed line from render(plan), one from render_preparation: \
         {default_stdout}"
    );
    assert_eq!(
        default_stdout.matches("--show-unmanaged").count(),
        2,
        "each table prints its own hint: {default_stdout}"
    );
    assert!(
        !default_stdout.contains("stray-tool"),
        "collapsed means collapsed in both of apply's tables: {default_stdout}"
    );

    // `--show-unmanaged` restores the individual line in both tables too.
    assert!(shown_stdout.contains("stray-tool"), "was:\n{shown_stdout}");
    assert_eq!(
        shown_stdout.matches("(unmanaged -- no action)").count(),
        2,
        "one line in the plan table, one in the preparation table: {shown_stdout}"
    );
    assert!(
        !shown_stdout.contains("--show-unmanaged"),
        "was:\n{shown_stdout}"
    );
}

// -- Phase 4 task 14: winget wired into the binary --------------------------
//
// Before this task nothing outside `src/backend/winget.rs` ever constructed a
// winget backend, so `dotpkg status`/`apply` were blind to it no matter what
// was on the machine. These pin the wiring end to end through the real
// compiled binary rather than through `plan()`/`Winget` directly.
//
// `Fixture::run` hands the spawned process `path_without_winget()`, so
// `winget` is absent from its `PATH` by construction, not by the accident of
// which machine happens to run this suite -- review caught the earlier
// version of this comment claiming the opposite, which would have gone red
// on the first Windows machine with winget installed to run it. That means
// these two tests still cannot exercise the "winget IS installed" branch --
// only a real Windows machine with a `PATH` this fixture does not control
// can -- so they assert the one thing guaranteed on every machine: no crash,
// and no second message on top of the one `Winget::scan` already prints.

#[test]
fn status_stays_quiet_about_winget_beyond_one_warning_when_the_binary_is_absent() {
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        "{}",
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");

    let out = f.run(&["status"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(0),
        "status stays read-only and must not fail over an absent winget: \
         stdout: {stdout} stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("warning: winget:").count(),
        1,
        "exactly one warning about winget, not a second on top of it: {stderr}"
    );
    // The scoop plan must still be printed and true, proving the wiring did
    // not break the path that already worked: fzf is declared, locked and
    // installed at the pinned version, so a converged machine has nothing to
    // do -- unless the merge silently added a stray winget report.
    assert!(
        stdout.contains("nothing to do"),
        "a converged scoop machine plus an absent winget is still nothing to do: {stdout}"
    );
}

#[test]
fn apply_prepare_also_sees_the_winget_scan_and_stays_quiet_about_it() {
    // `status` and `apply` wire winget through two different code paths --
    // `main.rs`'s own inline sequence, and `apply::load_everything`'s driver
    // -- and each must independently avoid a second message when winget is
    // absent.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
        r#"{"scoop":{"fzf":"installed"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");
    let before = f.snapshot();

    let out = f.run(&["apply", "--prepare"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("warning: winget:").count(),
        1,
        "exactly one warning about winget, not a second on top of it: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}

// -- Phase 5 task 4: `[winget.guard]` merged into the scan ------------------
//
// These three exist because nothing else in the suite can go red when
// `backend::apply_guard_overrides`, or one of its arguments, is deleted from a
// call site. `tests/planner.rs`'s `a_winget_package_is_held_by_a_guard_name_
// only_pkg_toml_knows` calls that function itself, so it stays green no matter
// what `main.rs` does; only the real binary can say whether `main.rs` calls it,
// and with what.
//
// They assert the WARNING half rather than the merge half, and that is forced
// rather than chosen: `Fixture::run` hands the spawned process
// `path_without_winget()`, so `Winget::scan`'s `NotFound` arm returns an empty
// `Scan` and there is no installed winget package on this machine for a guard
// name to be merged INTO. What an empty scan still discriminates is which guard
// keys draw a warning, which is a function of both remaining arguments -- and it
// is produced by the same call at the same point, so deleting that call takes
// the merge with it too. Proving the merge reaches the fence end-to-end needs a
// real winget package and a live process, which is a Windows-machine
// measurement, not a test.
//
// The third argument, `declared`, gets the third test rather than a share of
// the first two: it is only observable through the ABSENCE of a warning, and an
// absence is worth nothing without a present warning beside it in the same run
// to prove the code was reached at all.

#[test]
fn status_warns_when_a_winget_guard_entry_protects_nothing() {
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n\n\
         [winget.guard]\n\"Tailscale.Typo\" = [\"tailscaled\"]\n",
        "{}",
    );

    let out = f.run(&["status"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(0),
        "status stays read-only: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stderr.contains("[winget.guard]") && stderr.contains("Tailscale.Typo"),
        "status must say which guard key protects nothing: {stderr}"
    );
}

#[test]
fn apply_prepare_also_warns_about_a_winget_guard_entry_that_protects_nothing() {
    // `status` and `apply` reach the winget scan through two different code
    // paths -- `main.rs`'s own inline sequence, and `apply::load_everything`'s
    // driver -- so each needs the merge wired independently, and each needs its
    // own end-to-end witness.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n\n\
         [winget.guard]\n\"Tailscale.Typo\" = [\"tailscaled\"]\n",
        r#"{"scoop":{"fzf":"installed"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");
    let before = f.snapshot();

    let out = f.run(&["apply", "--prepare"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stderr.contains("[winget.guard]") && stderr.contains("Tailscale.Typo"),
        "apply must say which guard key protects nothing: {stderr}"
    );
    f.assert_nothing_was_touched(before);
}

#[test]
fn a_guard_entry_for_a_declared_package_stays_quiet_in_both_commands() {
    // The third argument of `apply_guard_overrides` -- `&declared.winget.packages`
    // -- is what separates "you misspelled an id" from "you have not installed
    // this yet". Replacing it with `&[]` at either call site is silent in the
    // whole rest of the suite, and its consequence is a warning on EVERY
    // `status` and EVERY `apply` for a perfectly correct pkg.toml on a machine
    // where the app is simply not installed yet.
    //
    // Both keys are in one fixture on purpose. The declared key can only be
    // observed by a warning that is ABSENT, and an absent warning proves nothing
    // on its own -- a run that died before the scan would satisfy it too. The
    // undeclared key's warning, in the same run, is what rules that out: exactly
    // one guard line must appear, and it must be the typo's.
    //
    // Both commands in one test, for the same reason: the two call sites must be
    // compared against a byte-identical fixture, so a divergence between them can
    // only come from the wiring. Which one broke is still named -- every
    // assertion message carries the argv.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n\n\
         [winget]\npackages = [\"Tailscale.Tailscale\"]\n\n\
         [winget.guard]\n\
         \"Tailscale.Tailscale\" = [\"tailscaled\"]\n\
         \"Tailscale.Typo\" = [\"tailscaled\"]\n",
        "{}",
    );

    for args in [["status"].as_slice(), ["apply", "--prepare"].as_slice()] {
        let out = f.run(args);
        let stderr = text(&out.stderr);
        let guard_lines: Vec<&str> = stderr
            .lines()
            .filter(|l| l.contains("[winget.guard]"))
            .collect();

        // Counted rather than substring-matched: this stays correct if the
        // message is reworded, and it is what makes the absence assertion below
        // non-vacuous.
        assert_eq!(
            guard_lines.len(),
            1,
            "{args:?}: exactly one guard warning -- the typo's, not the declared \
             package's. stderr: {stderr}"
        );
        assert!(
            guard_lines[0].contains("Tailscale.Typo"),
            "{args:?}: the one warning must be about the undeclared key: {stderr}"
        );
        // Belt and braces on the line above: `Tailscale.Typo` and
        // `Tailscale.Tailscale` are different strings, so a line naming the
        // declared id could not be the one counted -- but a future message that
        // named both ids at once would slip past the count alone.
        assert!(
            !guard_lines[0].contains("Tailscale.Tailscale"),
            "{args:?}: a declared package that is merely not installed yet must \
             draw no warning: {stderr}"
        );
    }
}

#[test]
fn a_declared_unlocked_winget_package_now_refuses_the_whole_run_before_execute_is_reached() {
    // **This test is inverted, on purpose, and its old name was
    // `a_declared_unlocked_winget_package_does_not_block_a_legitimate_scoop_
    // prune`.** It was written when a declared, unlocked winget package
    // becoming `SkipReason::NotLocked` was a Critical regression: `apply` could
    // not have installed the package even with a pin, so refusing the whole run
    // -- `aichat`'s entirely unrelated prune included -- punished the user for
    // a lock entry that could not have helped anyone.
    //
    // Phase 4b Task 13 removed the premise. Winget has an executor now, so
    // `Git.Git` without a pin is the same thing a scoop package without a pin
    // is: work dotpkg was asked to do and may not invent a version for.
    // Refusing is the correct answer, and quietly installing nothing would be
    // the "degrade silently" failure the spec forbids.
    //
    // **What this pins is the refusal path, and nothing else.** `main.rs`
    // prints its "could not be prepared" message and `exit(2)`s *before*
    // `gate_removals` is ever called, so no removal is held here and
    // `gate_removals` does not run at all -- an earlier revision of this
    // comment claimed this was that path end to end, which was a false claim
    // about coverage. `aichat` survives because **nothing ran**, not because a
    // prune was held. The held-removal path has its own end-to-end coverage in
    // this file, and needs `--keep-going` to be reached at all:
    // `keep_going_holds_a_ready_prune_back_when_another_package_could_not_be_
    // prepared` and `a_held_prune_appears_in_the_closing_table_not_only_as_a_
    // stderr_note`.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n\n[winget]\npackages = [\"Git.Git\"]\n",
        r#"{"scoop":{"fzf":"installed","aichat":"adopted"}}"#,
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");
    f.install_app("aichat", "0.30.0");

    let out = f.run(&["apply", "--yes", "--allow-prune"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);
    let all = format!("{stdout}{stderr}");

    assert!(
        all.contains("1 package(s) could not be prepared, so nothing has been changed"),
        "one package could not be prepared, and the count must be that one: {all}"
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "a refusal is 2, distinct from the 1 a genuine failed attempt gets: {all}"
    );
    assert!(
        all.contains("Git.Git"),
        "and it must name the package it refused over: {all}"
    );
    assert!(
        all.contains("dotpkg update"),
        "and the command that fixes it: {all}"
    );
    // `execute` must never have run: the reachability sentinel this file uses
    // everywhere else is that `aichat`'s uninstall would FAIL loudly on a
    // platform with no real `scoop.cmd`, so its absence proves nothing was
    // attempted.
    assert!(
        !all.contains("FAILED"),
        "nothing may be attempted when the preparation is refused: {all}"
    );
    // Not "a held prune left it alone" -- the prune was never reached to be
    // held. This is the same fact the `FAILED` sentinel above asserts, from the
    // machine's side instead of the output's.
    assert!(
        f.scoop.path().join("apps").join("aichat").exists(),
        "a refusal must leave every package untouched"
    );
    // And the run really did refuse before `gate_removals`, not after it: that
    // function's own "was ready to be removed, but is held" note is the thing
    // this path skips.
    assert!(
        !all.contains("was ready to be removed, but is held"),
        "the refusal happens before any removal is held: {all}"
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
fn a_carried_forward_entry_that_blocks_the_write_is_named_and_the_advice_is_not_the_command_that_just_ran(
) {
    // `lock::save` runs `lock_coherence_guard` over the WHOLE new lock, and
    // `resolve_into_lock` carries an entry it could not re-resolve forward
    // unchanged -- so one malformed entry anywhere blocks the write and
    // discards every other package's resolution in the run.
    //
    // The guard's placement is right and stays. What was wrong was the
    // message: `apply.rs`'s context says "Run `dotpkg update` to rewrite it",
    // which is correct for `apply` and `status` and is nonsense printed BY
    // `update`. `update_resolves_a_declared_package_and_exits_zero` above is
    // the positive sibling: without it, an `update` that always refused to
    // write would satisfy every assertion here.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\", \"broken\"]\n",
        "{}",
    );
    f.write_lock_and_bucket_for("fzf", "1.0.0");
    // `broken` is declared, no bucket carries it, and its existing pin is
    // malformed -- `commit = "main"` is a revision expression, not a hash.
    // `update` keeps that pin (dropping it would turn a working package into
    // Skip{NotLocked}) and the guard then refuses the whole file.
    fs::write(
        f.work.path().join("pkg.lock"),
        "[scoop.broken]\nbucket = \"main\"\ncommit = \"main\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let before = fs::read_to_string(f.work.path().join("pkg.lock")).unwrap();

    let out = f.run(&["update"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(2),
        "refused, and pkg.lock was not touched: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stderr.contains("was NOT written"),
        "`render_update` has already printed the diff, which reads as an \
         accomplished fact -- say outright that it is not: {stderr}"
    );
    assert!(
        stderr.contains("broken"),
        "name the entry that blocked the write: {stderr}"
    );
    assert!(
        stderr.contains("hex"),
        "say what is wrong with it, not just that something is: {stderr}"
    );
    assert!(
        !stderr.contains("Run `dotpkg update`"),
        "the advice must not be the command that just failed: {stderr}"
    );
    assert!(
        stderr.contains("[scoop.<name>]") || stderr.contains("dotpkg update <name>"),
        "say what actually repairs it: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(f.work.path().join("pkg.lock")).unwrap(),
        before,
        "the refused write must leave pkg.lock exactly as it was"
    );
    assert!(
        !f.work.path().join("pkg.lock.bak").exists(),
        "a refused write must not displace the backup either"
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
fn update_named_scope_accepts_a_package_declared_only_under_winget() {
    // `main.rs`'s Named-scope pre-check used to test `declared.scoop.
    // packages.contains(n)` only, so `dotpkg update Git.Git` for a package
    // declared solely under `[winget]` was refused as "not declared" before
    // `update::run` was ever called -- the same class of scoop-only
    // assumption Task 15 exists to close. This is the CLI-level proof of the
    // fix in `main.rs`; the corresponding lower-level property (a `Scope::
    // Named` covers a winget package the same way it covers a scoop one) is
    // `fold_backend`'s own job and is not re-proven here.
    let f = Fixture::new("[winget]\npackages = [\"Git.Git\"]\n", "{}");

    let out = f.run(&["update", "Git.Git"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert!(
        !stderr.contains("is not declared in"),
        "a winget-only declared package must not be refused as undeclared: {stderr}"
    );
    // It still cannot resolve -- winget is absent from PATH by construction
    // -- but that is a later, different failure than the refusal under test.
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {stdout} stderr: {stderr}"
    );
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

// -- Task 15: update and adopt over both backends, through the real binary --
//
// `path_without_winget()` makes winget's absence fixture-enforced (see the
// Task 14 section's own doc comment above), so none of these can exercise a
// real resolve or a real winget-side adoption -- only that `update` and
// `adopt --backend winget` reach for winget for real (through `RealWinget`,
// wired in `main.rs`) and handle its absence the same graceful way `status`
// and `apply` already do for `scan`, rather than panicking or hanging.

#[test]
fn update_with_a_declared_winget_package_does_not_crash_when_winget_is_absent() {
    // `update` now calls `Winget::update_source` and `Winget::resolve_latest`
    // for real. Both fail to spawn `winget` (absent from `PATH` by
    // construction), which must become an ordinary per-package `Kept` plus a
    // warning -- not a panic, not a hang, and not the deleted phase-4
    // warning, whose absence is asserted here too, at the level a real user
    // actually runs this at.
    let f = Fixture::new("[winget]\npackages = [\"Git.Git\"]\n", "{}");

    let out = f.run(&["update"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert!(
        out.status.code().is_some(),
        "the process must exit cleanly, not be killed by a signal: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        !stderr.contains("lands in phase 4"),
        "the deleted phase-4 warning must not reappear: {stderr}"
    );
    assert!(
        !stdout.contains("panicked") && !stderr.contains("panicked"),
        "no panic: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("Git.Git"),
        "the package was genuinely reached, not silently skipped: {stdout}"
    );
    // A resolve that could not even spawn winget is a failure, so `update`
    // exits 1 -- the same code an unresolvable scoop package would.
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        !f.work.path().join("pkg.lock").exists(),
        "nothing resolved, so nothing was written"
    );
}

#[test]
fn adopt_backend_winget_refuses_gracefully_when_the_package_is_not_installed() {
    // Winget absent from `PATH` -> `Winget::scan` returns an empty scan plus
    // one warning (its own graceful-absence path, unchanged by this task) ->
    // the named package is not in `scan.installed` -> an ordinary refusal,
    // not a crash. This is also the winget-side proof of item 4: `adopt`
    // must not drop a backend's `scan.warnings` on the floor the way it once
    // did for scoop (`docs/phase3-notes.md`).
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
        r#"{"scoop":{}}"#,
    );

    let out = f.run(&["adopt", "--backend", "winget", "Git.Git"]);
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a refusal is outstanding work: stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.contains("Git.Git") && stdout.contains("not installed"),
        "{stdout}"
    );
    assert!(
        stderr.contains("warning: winget:"),
        "winget's own scan warning must reach the user, the same way scoop's \
         already does: {stderr}"
    );
    assert!(
        !f.work.path().join("pkg.lock").exists(),
        "a refused adopt writes nothing"
    );
}

#[test]
fn adopt_backend_defaults_to_scoop_so_every_pre_task_15_invocation_is_unchanged() {
    // The positive control for the two tests above: `adopt fzf` with no
    // `--backend` at all must still mean scoop, exactly as it did before
    // this flag existed. Otherwise both refusal tests above could pass for
    // the wrong reason -- a version that silently ignored `--backend`
    // entirely and always used scoop would satisfy their assertions too.
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
        r#"{"scoop":{}}"#,
    );
    bucket_only(&f, "fzf", "1.0.0");
    f.install_app("fzf", "1.0.0");

    let out = f.run(&["adopt", "fzf"]);
    let stdout = text(&out.stdout);

    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let lock = fs::read_to_string(f.work.path().join("pkg.lock")).unwrap();
    assert!(lock.contains("[scoop.fzf]"), "{lock}");
}

#[test]
fn adopt_rejects_an_unknown_backend_value_rather_than_guessing() {
    let f = Fixture::new(
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
        r#"{"scoop":{}}"#,
    );
    let before = f.snapshot();

    let out = f.run(&["adopt", "--backend", "nope", "fzf"]);
    let stderr = text(&out.stderr);

    assert_ne!(
        out.status.code(),
        Some(0),
        "an unknown backend must not silently succeed: {stderr}"
    );
    assert!(stderr.contains("nope"), "name the bad value: {stderr}");
    assert!(
        stderr.contains("scoop") && stderr.contains("winget"),
        "say what the real choices are: {stderr}"
    );
    assert!(
        !f.work.path().join("pkg.lock").exists(),
        "nothing may be written for a backend dotpkg does not recognise"
    );
    f.assert_nothing_was_touched(before);
}

// -- Task 15: `apply` refuses the winget removal it cannot perform --------
//
// Measured (`docs/measurements-2026-08-10-winget-write-path.md` §5): `winget
// install` of a user-scope package succeeds from an elevated session, and
// `winget uninstall` of that same package is then refused with `0x8A15007D`,
// three times over including `--all-versions`. The paired control at medium
// integrity -- same machine, same package, same argv, one variable changed --
// exited `0` and removed it. dotpkg's whole shape is a scheduled `apply`, so
// an elevated run can install a package and be *structurally* unable to
// remove it: every prune failing forever, not transiently.
//
// **None of these four cases can be reached by spawning the binary.** On
// every non-Windows machine `sys::elevated()` is a hardcoded `None`, and on
// Windows the answer is a property of how the test runner itself was
// launched -- so a `Fixture::run` assertion here would be green or red
// depending on which shell started `cargo test`, which is exactly the
// non-discriminating shape Phase 4's `resolve_root` test had. The elevation
// answer and the scope query are therefore parameters of the pre-check, and
// these tests drive the same three calls `main.rs`'s `apply` arm makes in the
// same order: `gate_removals`, then the pre-check, then `execute`.

/// One winget removal, the shape `plan_to_steps` emits for a prune.
fn winget_removal(id: &str) -> Step {
    Step::Winget(WingetStep::Remove {
        id: Name::new(id),
        version: "151.1.93.134".to_string(),
        guard: vec![],
    })
}

/// The scope answer the real `winget list -e --id <id> --scope user` gave for
/// these two ids on a14, one per direction
/// (`docs/measurements-2026-08-10-winget-write-path.md` §15): `Brave.Brave`
/// exits `0` under `--scope user` and `0x8A150014` under `--scope machine`
/// (19 of the 36 source-backed installed ids behave this way);
/// `Microsoft.VisualStudio.2022.BuildTools` does the exact reverse.
///
/// A closure rather than a constant `true`/`false`, and it panics on any
/// other id: a pre-check that ignored its `is_user_scope` argument entirely
/// and refused on elevation alone would satisfy the refusal case below with a
/// constant, and this makes that visible instead.
fn measured_scope(id: &Name) -> bool {
    match id.to_string().as_str() {
        "Brave.Brave" => true,
        "Microsoft.VisualStudio.2022.BuildTools" => false,
        other => panic!("no measured user/machine scope for {other} -- see §15"),
    }
}

/// A scoop `Mutator` that panics on every call. `execute`'s signature needs
/// one, and a winget-only step list must never reach it.
struct NoScoopMutator;

impl dotpkg::execute::Mutator for NoScoopMutator {
    fn uninstall(&self, app: &Name) -> anyhow::Result<dotpkg::execute::CommandReport> {
        panic!("a winget-only run reached scoop's uninstall for {app}")
    }
    fn install(
        &self,
        manifest: &Path,
        _arch: Option<&str>,
    ) -> anyhow::Result<dotpkg::execute::CommandReport> {
        panic!(
            "a winget-only run reached scoop's install for {}",
            manifest.display()
        )
    }
    fn download(
        &self,
        manifest: &Path,
        _arch: Option<&str>,
    ) -> anyhow::Result<dotpkg::execute::CommandReport> {
        panic!(
            "a winget-only run reached scoop's download for {}",
            manifest.display()
        )
    }
    fn bucket_add(
        &self,
        bucket: &dotpkg::config::BucketDecl,
    ) -> anyhow::Result<dotpkg::execute::CommandReport> {
        panic!(
            "a winget-only run reached scoop's bucket add for {}",
            bucket.name
        )
    }
}

#[test]
fn an_elevated_run_refuses_a_user_scope_winget_removal_before_anything_happens() {
    // Fail closed, before acting, the same shape and the same reasoning as
    // `execute::root_looks_like_scoop`: the refusal happens before the
    // recovery file is written and before one single step runs.
    //
    // The fake mutator is `unreachable()`, so if the pre-check does not fire
    // this test panics loudly rather than passing for the wrong reason. Its
    // panic message talks about a test that "declared no winget packages",
    // which is `FakeWingetMutator`'s own wording for the read-side rule it
    // usually enforces; here it means the narrower thing this test is about,
    // that a refused run performs no winget mutation at all.
    let steps = vec![winget_removal("Brave.Brave")];

    // `main.rs`'s order, unchanged: `gate_removals` first (a prune is only
    // ever reachable through it), then the pre-check, then `execute`.
    let (steps, held) = dotpkg::apply::gate_removals(steps, true);
    assert!(held.is_empty(), "an ok preparation holds nothing back");
    let refusal =
        dotpkg::apply::refuse_elevated_winget_removal(&steps, Some(true), &measured_scope);

    let why = match refusal {
        Err(why) => why,
        // Deleting the `refusal` binding above and this arm's guard is the
        // delete-the-pre-check experiment: what is left is the run `main.rs`
        // would perform, and it reaches a mutator that panics.
        Ok(()) => {
            let wm = FakeWingetMutator::unreachable();
            let _ = dotpkg::execute::execute(
                Path::new("/dotpkg-test/no-scoop-root"),
                steps,
                &NoScoopMutator,
                &wm,
                &mut State::default(),
                &Running::default,
                &ExecOptions::default(),
            );
            unreachable!(
                "the pre-check allowed an elevated user-scope winget removal, and the run \
                 reached `execute` without even the fake mutator noticing"
            );
        }
    };

    assert!(
        why.contains("Brave.Brave"),
        "name the package that cannot be removed: {why}"
    );
    assert!(
        why.contains("0x8A15007D"),
        "name the measured exit code, so the refusal is traceable to §5 rather \
         than reading as dotpkg's own policy: {why}"
    );
    assert!(
        why.contains("user scope"),
        "say which scope this is about -- a machine-scope removal is NOT refused: {why}"
    );
    assert!(
        why.to_lowercase().contains("elevat"),
        "say that elevation is the variable the user can change: {why}"
    );
}

#[test]
fn an_elevated_run_allows_a_machine_scope_winget_removal_because_nothing_measured_it() {
    // The narrowing that keeps this guard honest. Whether a MACHINE-scope
    // package can be removed while elevated was never measured -- §5's
    // trio is a user-scope package throughout -- so refusing it would be a
    // refusal invented rather than measured, and it would break the one
    // removal an elevated scheduled `apply` is most likely to be for.
    //
    // Also the positive control for the test above: a pre-check that refused
    // on elevation alone, ignoring scope, would satisfy that one and fail
    // here.
    let asked = std::cell::Cell::new(0);
    let scope = |id: &Name| {
        asked.set(asked.get() + 1);
        measured_scope(id)
    };
    let steps = vec![winget_removal("Microsoft.VisualStudio.2022.BuildTools")];

    let r = dotpkg::apply::refuse_elevated_winget_removal(&steps, Some(true), &scope);

    assert!(
        r.is_ok(),
        "a machine-scope removal must not be refused: {r:?}"
    );
    assert_eq!(
        asked.get(),
        1,
        "exactly one scope query per winget removal -- each one is a ~1 s \
         `winget list` subprocess"
    );
}

#[test]
fn a_run_that_is_not_elevated_allows_a_user_scope_winget_removal_and_asks_winget_nothing() {
    // The paired control from §5 itself: the identical package and argv, run
    // de-elevated in the same session, exited `0` and removed it. A refusal
    // here would refuse the one run that was measured to WORK.
    //
    // And the query count is the point of the second assertion: the scope
    // query is a `winget list` subprocess measured at roughly a second, so a
    // pre-check that asked it before looking at the elevation answer would
    // put that second onto every prune on every machine, for a question
    // whose answer cannot change the outcome.
    let asked = std::cell::Cell::new(0);
    let scope = |id: &Name| {
        asked.set(asked.get() + 1);
        measured_scope(id)
    };
    let steps = vec![winget_removal("Brave.Brave")];

    let r = dotpkg::apply::refuse_elevated_winget_removal(&steps, Some(false), &scope);

    assert!(
        r.is_ok(),
        "the measured-to-succeed case must not be refused: {r:?}"
    );
    assert_eq!(
        asked.get(),
        0,
        "a run that is not elevated must not pay winget's scope query at all"
    );
}

#[test]
fn an_unknown_elevation_answer_allows_a_user_scope_winget_removal_rather_than_refusing() {
    // `sys::elevated()` returns `None` for "could not tell" -- and returns it
    // unconditionally on every non-Windows target. A machine whose token
    // query failed is a machine dotpkg knows nothing about, and refusing
    // every winget removal there would be a refusal caused by a missing
    // answer rather than by a measured hazard. `0x8A15007D` is still
    // translated into a named failure by `run_winget_step` if it does happen:
    // a pre-check plus a translation, not either alone.
    let asked = std::cell::Cell::new(0);
    let scope = |id: &Name| {
        asked.set(asked.get() + 1);
        measured_scope(id)
    };
    let steps = vec![winget_removal("Brave.Brave")];

    let r = dotpkg::apply::refuse_elevated_winget_removal(&steps, None, &scope);

    assert!(
        r.is_ok(),
        "`None` must not refuse -- it is an absence of an answer, not a hazard: {r:?}"
    );
    assert_eq!(
        asked.get(),
        0,
        "and an answer that cannot decide anything must not cost a subprocess either"
    );
}

// -- Task 15 review: the ORDERING is the thing that was unpinned -----------
//
// The four tests above pin `refuse_elevated_winget_removal` itself: given a
// post-`gate_removals` step list, it refuses before a mutator is reached. They
// do **not** pin that `main.rs` calls it, where in the arm it sits, or what it
// is handed -- they re-implement the sequence by hand and never enter
// `src/main.rs` at all. Moving the call after the confirmation prompt, or
// passing a constant `Some(false)` for the elevation answer, left the whole
// suite green.
//
// `apply::gate_the_run` exists to shrink that: the three checks between a
// prepared step list and `execute`, in one function, in the order they must
// run, so the ORDER is tested code rather than four lines of driver nobody can
// reach. What is left unpinned is one call and its arguments.

#[test]
fn a_run_refused_for_its_flags_is_refused_before_winget_is_asked_anything() {
    // The ordering that actually costs something. The `--allow-prune` gate
    // decides from the step list alone; the elevation pre-check spends one
    // ~1 s `winget list` per winget removal. Putting the expensive one first
    // would make every misflagged elevated prune pay for an answer that
    // cannot change the outcome.
    let asked = std::cell::Cell::new(0);
    let scope = |id: &Name| {
        asked.set(asked.get() + 1);
        measured_scope(id)
    };
    let steps = vec![winget_removal("Brave.Brave")];

    let gate = dotpkg::apply::gate_the_run(&steps, &[], &[], true, false, Some(true), &scope);

    match &gate {
        dotpkg::apply::RunGate::Refuse(why) => {
            assert!(
                why.contains("--allow-prune"),
                "the flag gate is what must speak here: {why}"
            );
            assert!(
                !why.contains("0x8A15007D"),
                "and NOT the elevation pre-check, which has not run yet: {why}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(
        asked.get(),
        0,
        "a run already refused on its flags must not pay for a scope query"
    );
}

#[test]
fn a_converged_run_is_nothing_to_do_and_a_held_only_run_is_not() {
    // The hoisted early return, and the exact condition that makes it safe:
    // all THREE of steps, unusable and held empty. A version that forgot
    // `held.is_empty()` would swallow the run whose only outstanding work is a
    // held prune -- and with it the closing-table row that says so.
    let asked = std::cell::Cell::new(0);
    let scope = |id: &Name| {
        asked.set(asked.get() + 1);
        measured_scope(id)
    };

    assert_eq!(
        dotpkg::apply::gate_the_run(&[], &[], &[], false, false, Some(true), &scope),
        dotpkg::apply::RunGate::NothingToDo,
        "nothing to install, nothing to remove, nothing held, nothing unusable"
    );

    let held = vec![("winget".to_string(), Name::new("Brave.Brave"))];
    assert_eq!(
        dotpkg::apply::gate_the_run(&[], &[], &held, false, false, Some(true), &scope),
        dotpkg::apply::RunGate::Proceed,
        "a held prune is outstanding work: the run must go on to report it"
    );

    assert_eq!(
        asked.get(),
        0,
        "neither shape has a winget removal to ask about"
    );
}

#[test]
fn the_elevation_pre_check_is_reached_once_the_cheaper_gates_are_satisfied() {
    // Delegation: `gate_the_run` really does consult
    // `refuse_elevated_winget_removal`, and hands it the post-`gate_removals`
    // step list. Without this, the hoist could have dropped the pre-check
    // entirely and the three tests around it would still pass.
    let asked = std::cell::Cell::new(0);
    let scope = |id: &Name| {
        asked.set(asked.get() + 1);
        measured_scope(id)
    };
    let steps = vec![winget_removal("Brave.Brave")];

    let gate = dotpkg::apply::gate_the_run(&steps, &[], &[], true, true, Some(true), &scope);

    match &gate {
        dotpkg::apply::RunGate::Refuse(why) => {
            assert!(
                why.contains("0x8A15007D") && why.contains("Brave.Brave"),
                "the elevation refusal, naming the package and the measured code: {why}"
            );
        }
        other => panic!("expected the elevation refusal, got {other:?}"),
    }
    assert_eq!(
        asked.get(),
        1,
        "exactly one scope query, for the one removal"
    );
}

#[test]
fn a_run_that_clears_all_three_gates_proceeds() {
    // The positive control for the three above: a function that returned
    // `Refuse` unconditionally would satisfy two of them, and one that
    // returned `NothingToDo` unconditionally would satisfy the other. This is
    // the elevated machine-scope prune -- authorised by both flags, and
    // unmeasured rather than refused.
    let asked = std::cell::Cell::new(0);
    let scope = |id: &Name| {
        asked.set(asked.get() + 1);
        measured_scope(id)
    };
    let steps = vec![winget_removal("Microsoft.VisualStudio.2022.BuildTools")];

    assert_eq!(
        dotpkg::apply::gate_the_run(&steps, &[], &[], true, true, Some(true), &scope),
        dotpkg::apply::RunGate::Proceed,
        "an authorised machine-scope prune on an elevated machine must run"
    );
    assert_eq!(asked.get(), 1, "and it was asked, rather than assumed");
}

/// The one link no other test in this suite can make: the **real**
/// `sys::elevated()` answer, from a real elevated Windows session, driving the
/// real pre-check.
///
/// `#[ignore]` because its whole premise is a property of the process it runs
/// in, and `#[cfg(windows)]` because `sys::elevated()` is a hardcoded `None`
/// everywhere else. Invoke it by name from the dogfood, in an elevated shell:
///
/// ```text
/// cargo test --test cli -- --ignored on_a_real_elevated_windows_session
/// ```
///
/// It fails, loudly and with instructions, if the session is not actually
/// elevated -- so it cannot pass by being run in the wrong place, which is the
/// failure mode a prose line in a checklist has.
///
/// **It still does not pin `main.rs`'s call site.** Reaching that needs a
/// fixture whose plan contains a removal of a winget package genuinely
/// installed at user scope on the machine under test, which no hermetic
/// fixture can construct. That check stays manual; see the task report.
#[test]
#[cfg(windows)]
#[ignore = "needs an elevated Windows session; the dogfood invokes it by name"]
fn on_a_real_elevated_windows_session_the_pre_check_refuses_a_user_scope_removal() {
    let elevated = dotpkg::sys::elevated();
    assert_eq!(
        elevated,
        Some(true),
        "run this from an ELEVATED Windows shell -- `sys::elevated()` said {elevated:?}, \
         so this run proves nothing either way"
    );
    let steps = vec![winget_removal("Brave.Brave")];

    let gate = dotpkg::apply::gate_the_run(&steps, &[], &[], true, true, elevated, &measured_scope);

    match &gate {
        dotpkg::apply::RunGate::Refuse(why) => assert!(
            why.contains("0x8A15007D"),
            "the real token said elevated, so the measured refusal must fire: {why}"
        ),
        other => panic!(
            "an elevated session with a user-scope winget removal must refuse, got {other:?}"
        ),
    }
}
