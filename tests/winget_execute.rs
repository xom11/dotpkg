mod common;

use common::fake_winget_mutator::FakeWingetMutator;
use dotpkg::backend::winget_exec::{set_argv, WingetMutator};
use dotpkg::model::Name;

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
