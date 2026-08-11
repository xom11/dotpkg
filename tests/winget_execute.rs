mod common;

use common::fake_winget::FakeWinget;
use common::fake_winget_mutator::FakeWingetMutator;
use dotpkg::backend::winget::{
    installed_at_user_scope, CmdError, NO_APPLICATIONS_FOUND, NO_VERSION_FOUND,
};
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
fn a_version_mismatch_under_a_different_exit_code_gets_the_generic_message_not_the_downgrade_one() {
    // The other side of the guard the test above pins: `out.code ==
    // NO_AVAILABLE_UPGRADE` is what selects the "will not downgrade" arm,
    // and this is a version mismatch under a DIFFERENT code -- so it must
    // fall through to the generic "asked winget for X, rescan reports Y"
    // arm instead. This is also the measured gap `run_winget_step`'s own
    // doc comment names: a machine ahead of its pin AND a pin no longer in
    // the index exits `NO_VERSION_FOUND` (0x8A150017), not
    // `NO_AVAILABLE_UPGRADE` -- so a stale pin gets neither "will not
    // downgrade" nor the `dotpkg update` advice today, and that gap is
    // exactly what this test's negative assertions pin.
    let m = FakeWingetMutator::script(vec![
        (NO_VERSION_FOUND, fixture("install-version-absent.txt")),
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
                why.contains("asked winget for 0.24.1") && why.contains("rescan reports 0.26.2"),
                "the generic message, not the downgrade-specific one: {why}"
            );
            assert!(
                !why.contains("will not downgrade"),
                "this exit code was never diagnosed as a downgrade refusal: {why}"
            );
            assert!(
                !why.contains("dotpkg update"),
                "the downgrade advice must not appear for an unrelated exit code: {why}"
            );
        }
        other => panic!("expected the generic failure message, got {other:?}"),
    }
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

#[test]
fn a_stale_removal_version_is_not_mistaken_for_the_elevation_refusal() {
    // The mirror of the test above: `out.code == CANNOT_UNINSTALL_ELEVATED`
    // is what selects the elevation explanation, and this removal still
    // finds the package installed under a DIFFERENT code. Measured
    // (`docs/measurements-2026-08-10-winget-write-path.md` §8): `uninstall
    // --version` refuses with `NO_VERSION_FOUND` when the version this
    // crate believes is installed no longer matches what is actually there
    // -- a real way for a removal to leave a package behind that has
    // nothing to do with elevation. Nobody diagnosed elevation here, so the
    // elevation explanation must not print.
    let m = FakeWingetMutator::script(vec![
        (NO_VERSION_FOUND, fixture("uninstall-version-absent.txt")),
        (0, fixture("list-single-with-available.txt")), // still installed at 0.24.1
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
            assert!(
                why.contains("did not happen") && why.contains("0.24.1"),
                "the generic still-installed message: {why}"
            );
            assert!(
                !why.contains("elevat"),
                "nothing here diagnosed an elevation refusal: {why}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(
        st.ownership(WINGET, &Name::new("ducaale.xh")),
        Some(Ownership::Installed),
        "a failed removal must not release ownership -- the package is still there"
    );
}

// -- Task 14 review: the two findings that changed behaviour or lacked a test

#[test]
fn a_mutation_that_ran_and_could_not_be_rescanned_is_touched_in_both_directions() {
    // `winget_verdict` returns `Err` from exactly one place -- `list_one` --
    // because every other problem it can hit becomes `Ok(Unconfirmable)`. So
    // these two arms have exactly one meaning: the mutation ALREADY RAN (its
    // exit code is in the message) and then the rescan could not be spawned.
    // The machine may well have changed and dotpkg cannot look, which is the
    // same epistemic state as `Unconfirmable` and must get the same answer --
    // it must not flip merely because a different call was the one that failed
    // to answer.
    //
    // What rides on it: `render_execution` reads `Execution::touched()` to
    // choose between "nothing was changed" and "some packages were changed and
    // some were not". `touched: false` here tells an operator nothing happened
    // directly after a mutation that did -- exactly what `StepOutcome::Failed`'s
    // own doc comment forbids.
    let id = Name::new("ducaale.xh");

    let m = FakeWingetMutator::script_then_failing(
        vec![(0, fixture("install-version-fresh.txt"))],
        CmdError::NotFound,
    );
    let mut st = State::default();
    let set = WingetStep::Set {
        id: id.clone(),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    match run_winget_step(&m, &mut st, &set) {
        StepOutcome::Failed { why, touched } => {
            assert!(touched, "the install ran and cannot be confirmed: {why}");
            assert!(
                why.contains("install ran") && why.contains("rescan could not"),
                "say that the mutation ran and the rescan did not: {why}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(
        st.ownership(WINGET, &id),
        None,
        "an unconfirmed install must not claim ownership"
    );

    // The removal direction, where the stakes are if anything higher: an
    // operator told "nothing was changed" would not go looking for a package
    // that is no longer there.
    let m2 = FakeWingetMutator::script_then_failing(
        vec![(0, fixture("uninstall-success.txt"))],
        CmdError::NotFound,
    );
    let mut st2 = State::default();
    st2.set(WINGET, &id, Ownership::Installed);
    let remove = WingetStep::Remove {
        id: id.clone(),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    match run_winget_step(&m2, &mut st2, &remove) {
        StepOutcome::Failed { why, touched } => {
            assert!(touched, "the uninstall ran and cannot be confirmed: {why}");
            assert!(
                why.contains("uninstall ran") && why.contains("rescan could not"),
                "say that the mutation ran and the rescan did not: {why}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(
        st2.ownership(WINGET, &id),
        Some(Ownership::Installed),
        "an unconfirmed removal must not release ownership -- dotpkg cannot see \
         whether the package is gone, and dropping the record would strand it"
    );
}

#[test]
fn a_set_over_an_adopted_package_keeps_it_adopted() {
    // The `is_none()` guard in the `Set` arm, which mirrors
    // `a_successful_replace_of_an_adopted_package_keeps_it_adopted` on the
    // scoop side. Ownership is intent: dotpkg honouring a pin for a package the
    // operator adopted must not silently promote that record to `Installed`,
    // which would tell a later prune that dotpkg had installed it and may
    // remove it.
    //
    // Both other `Set` tests start from `State::default()`, so before this test
    // only the comment claimed the guard -- a mutant deleting the `is_none()`
    // check stayed green across the whole suite.
    let id = Name::new("ducaale.xh");
    let m = FakeWingetMutator::script(vec![
        (0, fixture("install-upgraded.txt")),
        (0, fixture("list-single-with-available.txt")), // reports 0.24.1
    ]);
    let mut st = State::default();
    st.set(WINGET, &id, Ownership::Adopted);
    let step = WingetStep::Set {
        id: id.clone(),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    assert_eq!(run_winget_step(&m, &mut st, &step), StepOutcome::Done);
    assert_eq!(
        st.ownership(WINGET, &id),
        Some(Ownership::Adopted),
        "a Set over an adopted package must leave it adopted, not claim it"
    );
}

// -- Task 15: the scope query the elevation pre-check is backed by ---------

#[test]
fn the_scope_query_asks_section_15s_argv_and_answers_its_two_measured_exit_codes() {
    // `apply::refuse_elevated_winget_removal` takes `is_user_scope` as an
    // injected closure; this is the one function that answers it for real, so
    // the whole pre-check is only as good as this argv. Both directions are
    // measured on a14 (`docs/measurements-2026-08-10-winget-write-path.md`
    // §15), which is why they are asserted together rather than in two tests:
    // a version that hardcoded either answer would satisfy one half and fail
    // the other.
    //
    // `ajeetdsouza.zoxide` is one of the 19 ids §15 found exiting `0` under
    // `--scope user` and `0x8A150014` under `--scope machine`, and
    // `list-single.txt` is that exact `list -e --id ajeetdsouza.zoxide` call's
    // real 127 bytes (`tests/fixtures/winget/PROVENANCE.md`).
    let zoxide = Name::new("ajeetdsouza.zoxide");
    let user = FakeWinget::returning(0, fixture("list-single.txt"));
    assert_eq!(
        installed_at_user_scope(&user, &zoxide),
        Some(true),
        "exit 0 with the id's own row IS the user-scope answer"
    );
    assert_eq!(
        user.calls(),
        vec![vec![
            "list".to_string(),
            "-e".to_string(),
            "--id".to_string(),
            "ajeetdsouza.zoxide".to_string(),
            "--scope".to_string(),
            "user".to_string(),
            "--disable-interactivity".to_string(),
        ]],
        "the argv §15 measured, and exactly one call: each one is a ~1 s \
         subprocess the pre-check pays per winget removal"
    );

    // The reverse direction, spelled out in §15 against the real machine:
    // `--scope user` on a machine-scoped package exits 0x8A150014 with the
    // 53-byte sentence.
    let build_tools = Name::new("Microsoft.VisualStudio.2022.BuildTools");
    let machine = FakeWinget::returning(NO_APPLICATIONS_FOUND, fixture("list-not-found.txt"));
    assert_eq!(
        installed_at_user_scope(&machine, &build_tools),
        Some(false),
        "not installed at user scope -- and a machine-scope removal must NOT be refused"
    );

    // The row's `Id` matched as a `Name`, not as bytes. `winget_verdict` -- the
    // other place a `list` answer is matched back against the id that asked for
    // it -- compares `&i.name == id`, which folds case, and the two must agree
    // about what "this row is that package" means. A byte comparison here
    // returns `None` ("could not tell") for the same row, and `main.rs` reads
    // `None` as "not blocked": the pre-check fails OPEN, which is the one
    // direction a guard must not fail in.
    let folded = FakeWinget::returning(0, fixture("list-single.txt"));
    assert_eq!(
        installed_at_user_scope(&folded, &Name::new("AjeetDSouza.Zoxide")),
        Some(true),
        "a row whose case differs from the caller's spelling is still that package"
    );
}

#[test]
fn a_scope_answer_the_measurements_do_not_cover_is_could_not_tell_not_a_verdict() {
    // `None` travels to `main.rs`, which lets the removal proceed and says so.
    // That is the same rule `sys::elevated()`'s own `None` follows: a refusal
    // must be caused by a measured hazard, never by a missing answer -- and
    // `run_winget_step`'s `CANNOT_UNINSTALL_ELEVATED` translation is what
    // catches the case this lets through.
    let id = Name::new("ajeetdsouza.zoxide");

    // The measured trap this crate has been bitten by once already: `list -s
    // msstore` prints the byte-identical 53-byte "No installed package found"
    // sentence and exits **0** (`PROVENANCE.md`). Trusting exit 0 alone would
    // read that as "installed at user scope" and refuse a removal on the
    // strength of a sentence saying the opposite.
    let sentence_at_zero = FakeWinget::returning(0, fixture("list-source-filter-empty.txt"));
    assert_eq!(
        installed_at_user_scope(&sentence_at_zero, &id),
        None,
        "exit 0 with no row for this id is not an answer"
    );

    // A row for a DIFFERENT id at exit 0: `-e --id` is not supposed to be able
    // to produce this, so it is not an answer either.
    let other_row = FakeWinget::returning(0, fixture("list-single-with-available.txt"));
    assert_eq!(
        installed_at_user_scope(&other_row, &id),
        None,
        "a row for ducaale.xh says nothing about ajeetdsouza.zoxide"
    );

    // An exit code no scope measurement covers.
    let unmeasured = FakeWinget::returning(NO_AVAILABLE_UPGRADE, String::new());
    assert_eq!(
        installed_at_user_scope(&unmeasured, &id),
        None,
        "an exit code §15 never saw must not be read as either scope"
    );

    // No winget at all. A machine with no `winget.exe` cannot have a winget
    // removal in its plan, so this is defence in depth rather than a live
    // path -- but it must not be a refusal.
    let absent = FakeWinget::failing_to_spawn();
    assert_eq!(
        installed_at_user_scope(&absent, &id),
        None,
        "winget could not be run: an absence of an answer, not a hazard"
    );
}
