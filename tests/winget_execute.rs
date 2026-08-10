mod common;

use common::fake_winget_mutator::FakeWingetMutator;
use dotpkg::backend::winget::NO_APPLICATIONS_FOUND;
use dotpkg::backend::winget_exec::{
    list_one_argv, set_argv, winget_verdict, WingetMutator, WingetState, CANNOT_UNINSTALL_ELEVATED,
    NO_AVAILABLE_UPGRADE,
};
use dotpkg::execute::{run_winget_step, StepOutcome, WingetStep};
use dotpkg::model::{Name, WINGET};
use dotpkg::state::{Ownership, State};

fn fixture(name: &str) -> String {
    // Rust does no newline translation, so this keeps the CRLF the fixture was
    // captured with -- which is the point: a parser tested only against \n
    // passes here and fails on the one platform this tool runs on.
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/winget")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

#[test]
fn the_fake_records_the_argv_the_real_mutator_would_have_run() {
    // A fake nobody can inspect proves nothing about the argv, and the argv
    // is the whole contract -- exit codes are trusted only for these shapes.
    let f = FakeWingetMutator::returning(0, String::new());
    f.set(&Name::new("Brave.Brave"), "151.1.93.134").unwrap();
    assert_eq!(
        f.calls(),
        vec![set_argv(&Name::new("Brave.Brave"), "151.1.93.134")],
        "the fake must record the same argv the builder produces"
    );
}

#[test]
fn a_rescan_that_finds_the_pinned_version_confirms_it() {
    let m = FakeWingetMutator::returning(0, fixture("list-single-with-available.txt"));
    assert_eq!(
        winget_verdict(&m, &Name::new("ducaale.xh")).unwrap(),
        WingetState::At("0.24.1".to_string())
    );
    assert_eq!(m.calls(), vec![list_one_argv(&Name::new("ducaale.xh"))]);
}

#[test]
fn a_rescan_that_finds_nothing_is_absent_not_an_error() {
    // Measured: `list -e --id <absent>` exits 0x8A150014 and prints "No
    // installed package found matching input criteria." For a Remove step
    // this is the DESIRED end state, so it must be a state, not a failure.
    let m = FakeWingetMutator::returning(NO_APPLICATIONS_FOUND, fixture("list-not-found.txt"));
    assert_eq!(
        winget_verdict(&m, &Name::new("ducaale.xh")).unwrap(),
        WingetState::Absent
    );
}

#[test]
fn a_rescan_whose_row_is_opaque_cannot_confirm_anything() {
    // The rule that keeps the executor from being more credulous than the
    // scanner. `> 17.14.37` is winget saying *at least*; a version it will
    // not commit to cannot confirm a mutation.
    let m = FakeWingetMutator::returning(0, fixture("list-greater-prefix.txt"));
    match winget_verdict(&m, &Name::new("Microsoft.VisualStudio.2022.BuildTools")).unwrap() {
        WingetState::Unconfirmable(why) => {
            assert!(why.contains("> 17.14.37"), "the reason must name it: {why}")
        }
        other => panic!("expected Unconfirmable, got {other:?}"),
    }
}

#[test]
fn a_rescan_of_an_id_installed_at_two_versions_cannot_confirm_either() {
    // 7zip.7zip, measured twice on a14: two rows, two different versions.
    // Picking one would be inventing a fact -- the same reason
    // `rows_to_scan` sends it to `opaque`.
    let m = FakeWingetMutator::returning(0, fixture("list-duplicate-id.txt"));
    assert!(matches!(
        winget_verdict(&m, &Name::new("7zip.7zip")).unwrap(),
        WingetState::Unconfirmable(_)
    ));
}

// -- Task 14: `run_winget_step` -- where the measurements land -------------

#[test]
fn an_install_confirmed_by_the_rescan_is_done() {
    let m = FakeWingetMutator::script(vec![
        (0, fixture("install-version-fresh.txt")),
        (0, fixture("list-single-with-available.txt")), // reports 0.24.1
    ]);
    let mut st = State::default();
    let step = WingetStep::Set {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec!["xh".to_string()],
    };
    assert_eq!(run_winget_step(&m, &mut st, &step), StepOutcome::Done);
    assert_eq!(
        m.calls().len(),
        2,
        "one mutation, one rescan: {:?}",
        m.calls()
    );
    assert_eq!(
        st.ownership(WINGET, &Name::new("ducaale.xh")),
        Some(Ownership::Installed)
    );
}

#[test]
fn a_converged_package_is_done_even_though_winget_exited_nonzero() {
    // Measured: asking for the version already installed returns
    // 0x8A15002B "No available upgrade found." That is a SUCCESS -- the
    // machine is exactly where the pin says. Reading nonzero as failure
    // would report a failure on a converged machine every run.
    let m = FakeWingetMutator::script(vec![
        (
            NO_AVAILABLE_UPGRADE,
            fixture("install-already-installed-no-upgrade.txt"),
        ),
        (0, fixture("list-single-with-available.txt")), // still 0.24.1
    ]);
    let mut st = State::default();
    let step = WingetStep::Set {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    assert_eq!(run_winget_step(&m, &mut st, &step), StepOutcome::Done);
}

#[test]
fn a_machine_ahead_of_its_pin_is_a_named_downgrade_refusal_not_a_bare_failure() {
    // The measured Brave.Brave shape. Same exit code as the converged case
    // above; the rescan is what tells them apart -- which is the whole
    // reason the exit code is never the verdict.
    let m = FakeWingetMutator::script(vec![
        (
            NO_AVAILABLE_UPGRADE,
            fixture("install-already-installed-no-upgrade.txt"),
        ),
        (0, fixture("list-single-ahead-of-pin.txt")), // reports 0.26.2
    ]);
    let mut st = State::default();
    let step = WingetStep::Set {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    match run_winget_step(&m, &mut st, &step) {
        StepOutcome::Failed { why, touched } => {
            assert!(!touched, "nothing was changed: {why}");
            assert!(
                why.contains("0.26.2") && why.contains("0.24.1"),
                "both versions: {why}"
            );
            assert!(why.contains("will not downgrade"), "the rule: {why}");
            assert!(
                why.contains("dotpkg update"),
                "the actionable advice: {why}"
            );
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
    assert_eq!(
        st.ownership(WINGET, &Name::new("ducaale.xh")),
        None,
        "a refused step must not claim ownership"
    );
}

#[test]
fn a_removal_whose_rescan_finds_nothing_is_done_even_at_0x8a150014() {
    // Measured: `uninstall` of an absent package exits 0x8A150014 and prints
    // "No installed package found matching input criteria." For a Remove,
    // "already gone" is the desired end state -- and the exit code cannot be
    // told apart from "that id is wrong", so the rescan decides.
    let m = FakeWingetMutator::script(vec![
        (
            NO_APPLICATIONS_FOUND,
            fixture("uninstall-package-absent.txt"),
        ),
        (NO_APPLICATIONS_FOUND, fixture("list-not-found.txt")),
    ]);
    let mut st = State::default();
    st.set(WINGET, &Name::new("ducaale.xh"), Ownership::Installed);
    let step = WingetStep::Remove {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    assert_eq!(run_winget_step(&m, &mut st, &step), StepOutcome::Done);
    assert_eq!(st.ownership(WINGET, &Name::new("ducaale.xh")), None);
}

#[test]
fn the_elevation_refusal_says_what_to_do_about_it() {
    // Measured: install succeeds elevated, uninstall of that same user-scope
    // package is refused with 0x8A15007D. A scheduled apply at high
    // integrity can install and never remove.
    let m = FakeWingetMutator::script(vec![
        (
            CANNOT_UNINSTALL_ELEVATED,
            fixture("uninstall-refused-elevated.txt"),
        ),
        (0, fixture("list-single-with-available.txt")), // still installed
    ]);
    let mut st = State::default();
    st.set(WINGET, &Name::new("ducaale.xh"), Ownership::Installed);
    let step = WingetStep::Remove {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    match run_winget_step(&m, &mut st, &step) {
        StepOutcome::Failed { why, touched } => {
            assert!(!touched, "the package is still there, untouched: {why}");
            assert!(why.contains("elevat"), "name the cause: {why}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(
        st.ownership(WINGET, &Name::new("ducaale.xh")),
        Some(Ownership::Installed),
        "a failed removal must not release ownership -- the package is still there"
    );
}
