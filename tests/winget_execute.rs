mod common;

use common::fake_winget_mutator::FakeWingetMutator;
use dotpkg::backend::winget::NO_APPLICATIONS_FOUND;
use dotpkg::backend::winget_exec::{
    list_one_argv, set_argv, winget_verdict, WingetMutator, WingetState,
};
use dotpkg::model::Name;

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
