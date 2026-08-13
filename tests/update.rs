mod common;

use common::fake_winget::FakeWinget;
use common::*;
use dotpkg::backend::winget::Winget;
use dotpkg::lock::{Lock, Pin};
use dotpkg::model::{Name, SCOOP, WINGET};
use dotpkg::update::{self, Change, Resolution, Scope};

fn cfg(text: &str) -> dotpkg::config::Config {
    dotpkg::config::parse(text).unwrap()
}

/// The winget instance for every test in this file that declares no `[winget]`
/// packages at all: `update::run`'s winget loop only ever iterates
/// `declared.winget.packages`, so with nothing declared there it must never
/// be called -- `FakeWinget::unreachable` makes that a loud panic rather than
/// a silent assumption.
fn no_winget() -> Winget<FakeWinget> {
    Winget::new(FakeWinget::unreachable())
}

/// Read a winget fixture, keeping the CRLF it was captured with -- the same
/// reason `tests/winget_resolve.rs`'s own `fixture` helper does this rather
/// than using `std::fs::read_to_string`'s default (which does not, in fact,
/// translate anything on any platform Rust runs on, but matching the
/// established helper's own doc comment here rather than re-deriving it).
fn winget_fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/winget")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

#[test]
fn a_winget_resolution_cannot_carry_a_commit() {
    // A type-level fix, in the spirit of WriteLock/WritePkgToml/WriteState:
    // the wrong program stops being a bug to be caught and becomes one that
    // cannot be written. This test documents the shape; the compiler enforces
    // it. `Resolution::Resolved { bucket, commit, version }` allowed a winget
    // pin to be built with a bucket and a commit; `Pin` does not.
    let r = Resolution::Resolved {
        pin: Pin::WingetVersion {
            version: "2.55.0".into(),
        },
    };
    let Resolution::Resolved { pin } = r else {
        panic!("built above")
    };
    assert_eq!(pin.version(), "2.55.0");
}

#[test]
fn update_resolves_a_declared_package_against_the_bucket_on_disk() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");
    let newest = f.commit(&dir, "tool.json", "2.0.0", "v200");

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &no_winget(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );

    assert_eq!(
        u.changes,
        vec![Change::Added {
            backend: SCOOP,
            name: Name::new("tool"),
            version: "2.0.0".into()
        }]
    );
    match &u.lock.scoop[&Name::new("tool")] {
        Pin::ScoopCommit {
            bucket,
            commit,
            version,
        } => {
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
        &no_winget(),
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
fn an_ambiguous_bucket_is_refused_rather_than_guessed_and_names_both_candidates() {
    // NOT "keeps the old pin", which is what this was called until Task 14:
    // it runs with `Lock::default()`, deliberately, because an existing lock
    // entry NAMES a bucket and so decides the question before ambiguity can
    // arise at all. What this proves is that with nothing to decide it, two
    // candidate buckets are reported rather than picked between. The
    // keeps-the-old-pin property is covered at the unit level, in
    // `src/update.rs`'s `a_failed_reresolve_keeps_the_previous_entry_rather_
    // than_dropping_it`.
    let f = Fixture::new();
    for b in ["main", "extras"] {
        let dir = f.bucket(b);
        f.commit(&dir, "tool.json", "1.0.0", "v100");
    }

    let (u, _) = update::run(
        &f.scoop_root(),
        &no_winget(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    match &u.changes[0] {
        Change::Kept { why, .. } => {
            assert!(
                why.contains("main") && why.contains("extras"),
                "name both: {why}"
            );
            assert!(why.contains("scoop.opts"), "say how to fix it: {why}");
        }
        other => panic!("ambiguity must not be guessed: {other:?}"),
    }
    assert!(u.lock.scoop.is_empty(), "nothing resolved, nothing written");
}

#[test]
fn a_declared_bucket_that_is_not_on_this_machine_is_warned_about_by_name_and_path() {
    // The fetch loop used to `continue` past an absent bucket with no warning
    // at all, so the ONLY signal a user got was every package failing with
    // "no declared bucket has it".
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    let config = cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n");
    // Both runs: the presence of a bucket is a fact about the machine, not
    // about the network, so `--offline` must not suppress it either.
    for offline in [true, false] {
        let (_u, warnings) = update::run(
            &f.scoop_root(),
            &no_winget(),
            &config,
            &Lock::default(),
            &Scope::WholeRun,
            offline,
        );
        let absent: Vec<&String> = warnings
            .iter()
            .filter(|w| w.contains("not present at"))
            .collect();
        assert_eq!(
            absent.len(),
            1,
            "exactly one bucket is absent (offline={offline}): {warnings:?}"
        );
        let w = absent[0];
        assert!(w.contains("extras"), "name the bucket: {w}");
        assert!(
            w.contains(
                &f.scoop_root()
                    .join("buckets")
                    .join("extras")
                    .display()
                    .to_string()
            ),
            "say where it was looked for, so the user can see it is a path and \
             not a typo: {w}"
        );
        assert!(
            w.contains("--clone-missing-buckets"),
            "point at the command that fixes it: {w}"
        );
        // The counterweight, and what makes the assertions above discriminate:
        // `main` IS on this machine, so it must never be reported as absent.
        // An unconditional warning would satisfy everything above on its own.
        assert!(
            !w.contains("main"),
            "a bucket that is present must not be reported as absent: {w}"
        );
    }
}

#[test]
fn the_refusal_names_only_the_buckets_actually_searched_and_says_which_were_missing() {
    // The CRITICAL, end to end. Before the fix this run produced
    // `no declared bucket has it (searched: main, extras)` -- naming `extras`,
    // which is not on this machine and was never opened.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "other.json", "1.0.0", "v100");

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &no_winget(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    match &u.changes[0] {
        Change::Kept { why, .. } => {
            assert!(
                why.contains("searched: main"),
                "name what was really searched: {why}"
            );
            // The assertion the negative control is aimed at. Reverting
            // `choose_bucket` to `searched: declared_names` puts `extras`
            // back into the searched list, and this is what catches it.
            assert!(
                !why.contains("searched: main, extras"),
                "`extras` is not on this machine and was never opened -- it must \
                 not be reported as searched: {why}"
            );
            assert!(
                why.contains("not searched: extras"),
                "name the bucket that could not be searched, and say so: {why}"
            );
            assert!(
                why.contains("--clone-missing-buckets"),
                "point at the command that fixes it: {why}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_locked_bucket_that_is_not_on_this_machine_is_not_reported_as_a_missing_manifest() {
    // The `stated` branch, end to end. `extras` really does carry `tool` --
    // upstream -- but it is not cloned here. Before the fix this printed
    // "bucket extras has no manifest for it", which is false twice over: the
    // bucket has the manifest, and dotpkg never looked.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    let mut old = Lock::default();
    old.scoop.insert(
        Name::new("tool"),
        Pin::ScoopCommit {
            bucket: "extras".into(),
            commit: "a".repeat(40),
            version: "0.9.0".into(),
        },
    );

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &no_winget(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &old,
        &Scope::WholeRun,
        true,
    );
    match &u.changes[0] {
        Change::Kept { why, .. } => {
            assert!(
                !why.contains("no manifest"),
                "a bucket that is not on this machine has not been shown to lack \
                 anything: {why}"
            );
            assert!(
                why.contains("not present at"),
                "say that the bucket itself is absent: {why}"
            );
            assert!(why.contains("extras"), "name the bucket: {why}");
            assert!(
                why.contains("--clone-missing-buckets"),
                "point at the command that fixes it: {why}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_locked_bucket_that_pkg_toml_does_not_declare_is_named_and_told_to_declare_it() {
    // `update`'s arm of the `Undeclared` branch, end to end. `extras` is
    // cloned on disk and really does carry `tool` -- but `pkg.toml` only
    // declares `main`. Before the fix this printed `no declared bucket has it
    // (searched: extras)`, false twice over: `extras` was neither declared
    // nor searched.
    let f = Fixture::new();
    let main = f.bucket("main");
    f.commit(&main, "other.json", "1.0.0", "v100");
    let extras = f.bucket("extras");
    f.commit(&extras, "tool.json", "1.0.0", "v100");

    let mut old = Lock::default();
    old.scoop.insert(
        Name::new("tool"),
        Pin::ScoopCommit {
            bucket: "extras".into(),
            commit: "a".repeat(40),
            version: "0.9.0".into(),
        },
    );

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &no_winget(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &old,
        &Scope::WholeRun,
        true,
    );
    match &u.changes[0] {
        Change::Kept { why, .. } => {
            assert!(
                !why.contains("searched"),
                "an undeclared bucket was never searched, so nothing may claim it \
                 was: {why}"
            );
            assert!(why.contains("extras"), "name the bucket: {why}");
            assert!(
                why.contains("does not declare"),
                "say that it is not declared, not that it is absent from disk: {why}"
            );
            assert!(
                why.contains("[scoop] buckets"),
                "point at the fix, which is declaring it -- not \
                 --clone-missing-buckets, which is for a bucket already \
                 declared: {why}"
            );
        }
        other => panic!("{other:?}"),
    }
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
        &no_winget(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        false,
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("main") && w.contains("upstream")),
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
        &no_winget(),
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

// -- winget, resolved rather than warned about (Task 15) ------------------
//
// `declared_winget_packages_are_named_in_a_warning_and_their_pins_survive`
// stood here through Phase 3: it asserted the exact warning this task
// deletes ("N winget package(s) were not resolved: the winget backend lands
// in phase 4") and that `u.lock.winget` came back byte-identical to `old`,
// which is what "not resolved" means. Both are now false by design, so the
// test is gone rather than adjusted -- its "no unconditional warning" half
// is subsumed by `no_winget()` itself: every scoop-only test above this
// point declares no `[winget]` section and passes a `FakeWinget` that PANICS
// if `update::run`'s winget loop is ever reached for it at all, which is a
// stronger, structural counterweight than a single `.any()` check could be.

#[test]
fn update_resolves_winget_packages_instead_of_warning_that_it_cannot() {
    // The replacement for the deleted phase-4 warning: its absence is
    // asserted, not assumed.
    let f = Fixture::new();
    let fake = FakeWinget::returning(0, winget_fixture("show-git.txt"));
    let winget = Winget::new(fake);

    let (u, warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"Git.Git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    assert!(
        !warnings.iter().any(|w| w.contains("lands in phase 4")),
        "the phase-4 warning must be gone: {warnings:?}"
    );
    assert_eq!(u.lock.winget.len(), 1, "the package really was resolved");
    assert!(
        u.lock.scoop.is_empty(),
        "nothing here declared a scoop package: {:?}",
        u.lock.scoop
    );
}

#[test]
fn a_winget_lock_entry_is_written_with_the_canonical_id_not_the_declared_case() {
    // MEASURED (`show-canonical-echo.txt`): asked as `git.git`, winget
    // answers `Found Git [Git.Git]`. The lock must record what winget
    // matched.
    let f = Fixture::new();
    let fake = FakeWinget::returning(0, winget_fixture("show-canonical-echo.txt"));
    let winget = Winget::new(fake);

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"git.git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    // `get_key_value`, not indexing by `Name::new("git.git")` and stopping
    // there: `Name`'s `Eq` folds case, so indexing alone cannot tell "Git.Git"
    // apart from "git.git" -- the key this lookup actually returns is what
    // proves which one was really stored. `.to_string()` goes through
    // `Display`, which does not fold (`clippy::cmp_owned` would rather this
    // compared `Name`'s own `PartialEq<&str>` directly, but that impl folds
    // case too and would make this assertion pass no matter which spelling
    // won).
    let (stored_key, _) = u.lock.winget.get_key_value(&Name::new("git.git")).unwrap();
    assert_eq!(
        stored_key.to_string(),
        "Git.Git",
        "the lock records what winget matched: {:?}",
        u.lock.winget.keys().collect::<Vec<_>>()
    );
}

#[test]
fn a_canonical_id_that_is_a_different_id_fails_the_package_instead_of_writing_an_unusable_pin() {
    // The boundary of the test above. `resolve_latest` deliberately omits
    // `--exact` -- that is what folds case on the way in -- which also leaves
    // `--id` a substring filter, so a declared id can match a *different*
    // package. Reusing the measured `show-canonical-echo.txt` for exactly what
    // it shows: asked for something that is a substring of `Git.Git`, winget
    // answers `Found Git [Git.Git]`.
    //
    // Recording that was worse than refusing it, and silently so. The lock
    // would be keyed `Git.Git` while `plan` looks the pin up under the
    // declared `Git`, and those two never meet -- so the next `apply` said
    // `Skip { NotLocked }` and refused the whole run at exit 2, while `update`
    // rewrote the identical unusable lock every time it was run to fix it.
    // There is no way out of that loop from inside dotpkg; only editing
    // pkg.toml escapes it, which is what the message now says to do.
    let f = Fixture::new();
    let fake = FakeWinget::returning(0, winget_fixture("show-canonical-echo.txt"));
    let winget = Winget::new(fake);

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"Git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );

    assert!(
        u.lock.winget.is_empty(),
        "no pin may be written under a key the planner cannot look up: {:?}",
        u.lock.winget.keys().collect::<Vec<_>>()
    );
    let why = u
        .changes
        .iter()
        .find_map(|c| match c {
            update::Change::Kept {
                name,
                version: None,
                why,
                ..
            } if name == &Name::new("Git") => Some(why.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "the package must be reported as unresolved: {:?}",
                u.changes
            )
        });
    assert!(
        why.contains("Git.Git") && why.contains("\"Git\""),
        "the message must name both the id winget matched and the one pkg.toml \
         declares, since the fix is to edit pkg.toml: {why}"
    );
}

#[test]
fn a_case_difference_between_pkg_toml_and_the_canonical_id_is_reported_not_rewritten() {
    // The canonical-id rule's other half: `pkg.toml` is the user's file, and
    // `update` never touches it at all (only `pkg.lock`) -- so "not silently
    // rewritten" is true by construction here, and this test is really
    // about the WARNING half: the user must be told the two spellings
    // differ, naming both.
    let f = Fixture::new();
    let fake = FakeWinget::returning(0, winget_fixture("show-canonical-echo.txt"));
    let winget = Winget::new(fake);

    let (_u, warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"git.git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("git.git") && w.contains("Git.Git")),
        "name both spellings: {warnings:?}"
    );
}

#[test]
fn a_declared_spelling_that_already_matches_the_canonical_one_is_not_warned_about() {
    // The counterweight to the test above: without it, a version that warns
    // on every resolved winget package regardless of case would satisfy the
    // positive test on its own. `show-git.txt` is `Git.Git` asked AS
    // `Git.Git` -- no case difference at all.
    let f = Fixture::new();
    let fake = FakeWinget::returning(0, winget_fixture("show-git.txt"));
    let winget = Winget::new(fake);

    let (_u, warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"Git.Git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    assert!(
        !warnings
            .iter()
            .any(|w| w.contains("pkg.lock records the canonical spelling")),
        "an exact-case match has nothing to report: {warnings:?}"
    );
}

#[test]
fn a_failed_winget_resolution_keeps_the_previous_pin_the_same_way_scoop_does() {
    let f = Fixture::new();
    let fake = FakeWinget::returning(
        dotpkg::backend::winget::NO_APPLICATIONS_FOUND,
        winget_fixture("show-package-gone.txt"),
    );
    let winget = Winget::new(fake);

    let mut old = Lock::default();
    old.winget.insert(
        Name::new("Xyzzy.NoSuch"),
        Pin::WingetVersion {
            version: "1.0.0".into(),
        },
    );

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"Xyzzy.NoSuch\"]\n"),
        &old,
        &Scope::WholeRun,
        true,
    );
    assert_eq!(
        u.lock.winget[&Name::new("Xyzzy.NoSuch")],
        old.winget[&Name::new("Xyzzy.NoSuch")],
        "the previous pin must survive a failed re-resolve, exactly as scoop's does"
    );
    match &u.changes[0] {
        Change::Kept { backend, why, .. } => {
            assert_eq!(*backend, WINGET);
            assert!(why.contains("no longer") || why.contains("not in"), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_winget_package_no_longer_declared_is_dropped_on_a_whole_run() {
    // No `[winget]` packages are DECLARED here (the config is empty), so
    // `no_winget()`'s panic-on-any-call guarantee still holds -- this proves
    // the drop side of `fold_backend`'s winget call without needing a real
    // resolution at all.
    let f = Fixture::new();
    let mut old = Lock::default();
    old.winget.insert(
        Name::new("Git.Git"),
        Pin::WingetVersion {
            version: "2.55.0".into(),
        },
    );

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &no_winget(),
        &cfg(""),
        &old,
        &Scope::WholeRun,
        true,
    );
    assert!(
        !u.lock.winget.contains_key(&Name::new("Git.Git")),
        "no longer declared, whole run: it must be dropped"
    );
    assert!(u.changes.iter().any(|c| matches!(
        c,
        Change::Dropped { backend, name, .. } if *backend == WINGET && *name == "Git.Git"
    )));
}

// -- winget source update (Task 15 / Task 1's measurement) -----------------

#[test]
fn update_refreshes_wingets_index_before_resolving_when_not_offline() {
    // Measured inert when scoped this way
    // (`docs/measurements-2026-08-09-winget.md` §9, "Repeated, scoped"), so
    // `update` may call it unconditionally whenever something is declared
    // for it to refresh. The ORDER matters: the index must be refreshed
    // before the one package that depends on it is resolved.
    let f = Fixture::new();
    let fake = FakeWinget::script(vec![
        (0, "Updating source: winget...\nDone\n".to_string()),
        (0, winget_fixture("show-canonical-echo.txt")),
    ]);
    let winget = Winget::new(fake.clone());

    let (_u, _warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"git.git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        false,
    );
    assert_eq!(
        fake.calls(),
        vec![
            vec![
                "source",
                "update",
                "--name",
                "winget",
                "--disable-interactivity"
            ],
            vec!["show", "--id", "git.git", "--disable-interactivity"],
        ],
        "the index must be refreshed before any package is resolved"
    );
}

#[test]
fn offline_skips_wingets_index_refresh_but_still_resolves_against_it() {
    // The direct counterpart to `offline_skips_the_fetch_and_says_the_
    // result_may_be_stale` (scoop's own bucket fetch) -- `--offline` skips
    // the NETWORK call, never the resolution itself.
    let f = Fixture::new();
    let fake = FakeWinget::returning(0, winget_fixture("show-canonical-echo.txt"));
    let winget = Winget::new(fake.clone());

    let (u, warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"git.git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    assert_eq!(
        fake.calls(),
        vec![vec!["show", "--id", "git.git", "--disable-interactivity"]],
        "no source-update call when offline: {:?}",
        fake.calls()
    );
    assert_eq!(
        u.lock.winget.len(),
        1,
        "offline still resolves against whatever the index already has"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("offline") && w.contains("winget")),
        "say the index was not refreshed: {warnings:?}"
    );
}

#[test]
fn a_failed_winget_index_refresh_is_a_warning_not_a_refusal() {
    let f = Fixture::new();
    let fake = FakeWinget::script(vec![
        (1, "some transient failure\n".to_string()),
        (0, winget_fixture("show-canonical-echo.txt")),
    ]);
    let winget = Winget::new(fake);

    let (u, warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"git.git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        false,
    );
    assert!(
        warnings.iter().any(|w| w.contains("could not refresh")),
        "{warnings:?}"
    );
    // The counterweight: resolution still proceeds against whatever the
    // (unrefreshed) index has, rather than refusing the whole run.
    assert_eq!(
        u.lock.winget.len(),
        1,
        "a refresh failure must not block resolution"
    );
}

/// The two halves of still-open item 20, pinned where a user would actually
/// see them.
///
/// Item 20 said the retry ships "structurally verified and live-unverified":
/// six `dotpkg update` rounds against a concurrent winget produced zero
/// warnings, and zero warnings is the *expected* output of both "the
/// contention never reproduced" and "it reproduced and the retry absorbed it".
/// `update_source` returning `Result<()>` is what made those two runs
/// identical to every observer, dogfood included. `SourceRefresh` is the fix,
/// and these are the tests that stop it from being decorative.
///
/// **This pair costs about one second of wall clock**, and knowingly: the
/// retry path sleeps `update_source`'s real 1 s delay, because `update::run`
/// calls `update_source()` and not the delay-injected `update_source_with`.
/// Threading a `Duration` through `update::run` to save that second would put
/// a test-only parameter on the one function the binary calls for real, which
/// is a worse trade than a second.
#[test]
fn a_winget_index_refresh_that_only_succeeded_on_the_retry_says_so() {
    let f = Fixture::new();
    let fake = FakeWinget::script(vec![
        // The measured contention: 0x8A150001, empty stdout, 60-72 ms
        // (docs/measurements-2026-08-11-phase5-guard-unmanaged-retry.md §5).
        (dotpkg::backend::winget::INTERNAL_ERROR, String::new()),
        (0, "Updating source: winget...\n".to_string()),
        (0, winget_fixture("show-canonical-echo.txt")),
    ]);
    let winget = Winget::new(fake);

    let (u, warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"git.git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        false,
    );
    assert!(
        warnings.iter().any(|w| w.contains("0x8A150001")),
        "the one line that makes a successful retry observable at all: {warnings:?}"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("could not refresh")),
        "an absorbed transient is not a failed refresh: {warnings:?}"
    );
    assert_eq!(
        u.lock.winget.len(),
        1,
        "the retry succeeded, so resolution must have proceeded normally"
    );
}

#[test]
fn an_ordinary_winget_index_refresh_says_nothing_at_all() {
    // THE COUNTERWEIGHT, and without it the test above is satisfied by a
    // `update_source` that reports `AfterRetry` unconditionally -- which would
    // put a contention warning in front of every user on every run, in a tool
    // that just spent a phase deleting lines from its own output. Measured
    // basis for expecting silence here: 0 nonzero exits in 10 `source update`
    // calls with no other winget process alive (§5).
    let f = Fixture::new();
    let fake = FakeWinget::script(vec![
        (0, "Updating source: winget...\n".to_string()),
        (0, winget_fixture("show-canonical-echo.txt")),
    ]);
    let winget = Winget::new(fake);

    let (_u, warnings) = update::run(
        &f.scoop_root(),
        &winget,
        &cfg("[winget]\npackages = [\"git.git\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        false,
    );
    assert!(
        !warnings.iter().any(|w| w.contains("0x8A150001")),
        "a refresh that never hit the transient must not mention it: {warnings:?}"
    );
}

#[test]
fn winget_source_update_is_never_called_when_nothing_is_declared_for_it() {
    // The gate: `update` does not reach for the network on winget's behalf
    // just because a `Winget` instance exists. `no_winget()`'s
    // panic-on-any-call already proves this for every scoop-only test above,
    // but this one is explicit about exactly this property, unconditional
    // on `offline`.
    for offline in [true, false] {
        let f = Fixture::new();
        let (_u, _warnings) = update::run(
            &f.scoop_root(),
            &no_winget(),
            &cfg("[scoop]\npackages = []\n"),
            &Lock::default(),
            &Scope::WholeRun,
            offline,
        );
        // Reaching this line at all (rather than panicking inside
        // `FakeWinget::unreachable`) is the assertion.
    }
}

#[test]
fn a_named_scoop_only_update_does_not_revert_an_existing_winget_lock_entrys_canonical_case() {
    // Review finding, Important 1, reproduced at the level a real command
    // line hits it: `pkg.lock` already canonically pins "Git.Git" (as an
    // earlier `update` run would have written it); `pkg.toml` declares it
    // as "git.git"; and `dotpkg update fzf` -- a NAMED run touching only an
    // entirely unrelated scoop package -- must not silently rewrite the
    // winget key back to the declared spelling. `no_winget()` is load-
    // bearing here, not incidental: `git.git` is outside this run's named
    // scope, so winget must never be asked about it at all -- if the fix
    // were wrong in a way that also called `resolve_latest` for an
    // out-of-scope name, THAT would panic first and this test would still
    // catch a regression, just a different one.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "fzf.json", "1.0.0", "v100");

    let mut old = Lock::default();
    old.winget.insert(
        Name::new("Git.Git"),
        Pin::WingetVersion {
            version: "2.55.0".into(),
        },
    );

    let (u, _warnings) = update::run(
        &f.scoop_root(),
        &no_winget(),
        &cfg(
            "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n\n[winget]\npackages = \
             [\"git.git\"]\n",
        ),
        &old,
        &Scope::Named(vec![Name::new("fzf")]),
        true,
    );

    let (stored_key, _) = u.lock.winget.get_key_value(&Name::new("git.git")).unwrap();
    assert_eq!(
        stored_key.to_string(),
        "Git.Git",
        "an unrelated named update must not silently rewrite a committed \
         winget lock key back to the declared spelling: {:?}",
        u.lock.winget.keys().collect::<Vec<_>>()
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
        &[
            "clone",
            "-q",
            &format!("file://{}", upstream.display()),
            &clone_dir.to_string_lossy(),
        ],
    );

    // The upstream moves after the clone. Without a fetch, update cannot see it.
    let moved = f.commit(&upstream, "tool.json", "2.0.0", "v200");

    let config = cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n");
    let (stale, _) = update::run(
        &f.scoop_root(),
        &no_winget(),
        &config,
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    assert_eq!(
        stale.changes,
        vec![Change::Added {
            backend: SCOOP,
            name: Name::new("tool"),
            version: "1.0.0".into()
        }],
        "offline must see only what the clone had"
    );

    let (fresh, _) = update::run(
        &f.scoop_root(),
        &no_winget(),
        &config,
        &Lock::default(),
        &Scope::WholeRun,
        false,
    );
    assert_eq!(
        fresh.changes,
        vec![Change::Added {
            backend: SCOOP,
            name: Name::new("tool"),
            version: "2.0.0".into()
        }],
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

/// The execution-level manifest `build.rs` embeds is present in the binary this
/// test is running in.
///
/// # Why this test is in this file specifically
///
/// This file compiles to `update-<hash>.exe`, and that filename is the whole
/// reason `build.rs` exists: Windows' UAC installer detection inspects an
/// executable's name for `install`, `setup`, `update` or `patch` and, for a
/// binary that declares no execution level, refuses to start it from an
/// ordinary session. So the binary that would fail to launch is the one
/// asserting it carries the fix.
///
/// # Why it is a test rather than a note saying it was checked once
///
/// It was checked once -- by hand, on one machine, by reading the bytes of
/// twelve test binaries -- and **nothing re-checks it**. The automatic Windows
/// gate cannot: `windows-latest` was green on `e91f4b1`, an ancestor of the
/// commit that added `build.rs`, with this file already compiling to
/// `update-<hash>.exe`. That runner started the binary happily with no manifest
/// at all, so CI has never been able to observe the failure this suppresses,
/// and a regression in `build.rs` would leave every automatic gate green.
///
/// # Why the needle is obfuscated, which is not decoration
///
/// The check is a byte search of the running executable for the level string
/// an embedded manifest puts into its resources. **The first version of this
/// test could not fail**: it spelled that string literally in its own assert
/// message, so the search always found the test's own copy of it. Built and run
/// against a deliberately neutered `build.rs`, it passed -- a gate that reports
/// success when the thing it guards is gone, which is this project's second
/// defect class, written inside the round whose document quotes that class.
///
/// So the literal appears nowhere in this file. It is reconstructed at run time
/// from a byte-shifted copy, and the assert message names it only indirectly.
/// If you edit this test, do not write the level string out.
#[cfg(windows)]
#[test]
fn this_test_binary_carries_the_execution_level_manifest_build_rs_embeds() {
    let exe = std::env::current_exe().expect("libtest always knows its own path");
    let bytes =
        std::fs::read(&exe).unwrap_or_else(|e| panic!("could not read {}: {e}", exe.display()));

    // Each byte is one more than the byte it stands for, so the string this
    // searches for does not exist anywhere in this binary except where a real
    // embedded manifest put it.
    const SHIFTED: &[u8] = b"btJowplfs";
    let needle: Vec<u8> = SHIFTED.iter().map(|b| b - 1).collect();

    // The obfuscation is only worth anything if it round-trips, and a silent
    // typo in SHIFTED would make the search look for a string nothing has and
    // turn this into a test that always fails for the wrong reason -- the back
    // side of the same defect class. Pin the reconstruction itself.
    assert_eq!(needle.len(), 9, "the shifted table is the wrong length");
    assert_eq!(
        needle[0] as char, 'a',
        "the shifted table does not decode to the level string"
    );
    assert_eq!(
        needle[2] as char, 'I',
        "the level string is capitalised mid-word"
    );

    let found = bytes.windows(needle.len()).any(|w| w == needle.as_slice());

    assert!(
        found,
        "{} carries no embedded execution-level manifest, so Windows' UAC \
         installer detection will refuse to start it from an ordinary session \
         -- the failure build.rs exists to suppress, and one no CI runner has \
         ever been able to reproduce. Check that build.rs still emits both \
         cargo::rustc-link-arg-tests lines.",
        exe.display()
    );
}
