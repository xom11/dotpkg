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
        vec![Change::Added {
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
fn declared_winget_packages_are_named_in_a_warning_and_their_pins_survive() {
    // Phase 3 resolves scoop only. This warning is the only thing standing
    // between "your winget packages were skipped, on purpose, and Phase 4
    // will do them" and a user believing `update` handled the whole file.
    // Untested until the Task 14 mutation run, which found `delete !` at
    // src/update.rs:314 surviving the entire suite because no test had ever
    // declared a [winget] section at all.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    let mut old = Lock::default();
    old.winget.insert(
        Name::new("Git.Git"),
        Pin::WingetVersion {
            version: "2.55.0".into(),
        },
    );

    let with_winget = cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n\
         \n[winget]\npackages = [\"Git.Git\"]\n");
    let (u, warnings) = update::run(&f.scoop_root(), &with_winget, &old, &Scope::WholeRun, true);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("winget") && w.contains('1')),
        "name how many were skipped, and that they were winget: {warnings:?}"
    );
    assert_eq!(
        u.lock.winget, old.winget,
        "a command that cannot resolve them must not drop them either"
    );

    // The counterweight, and what makes the assertion above discriminate: the
    // same run with no [winget] section must not mention winget at all. An
    // unconditional warning would satisfy the positive test on its own.
    let (_, quiet) = update::run(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    assert!(
        !quiet.iter().any(|w| w.contains("winget")),
        "a pkg.toml with no winget packages has nothing to warn about: {quiet:?}"
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
        &config,
        &Lock::default(),
        &Scope::WholeRun,
        true,
    );
    assert_eq!(
        stale.changes,
        vec![Change::Added {
            name: Name::new("tool"),
            version: "1.0.0".into()
        }],
        "offline must see only what the clone had"
    );

    let (fresh, _) = update::run(
        &f.scoop_root(),
        &config,
        &Lock::default(),
        &Scope::WholeRun,
        false,
    );
    assert_eq!(
        fresh.changes,
        vec![Change::Added {
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
