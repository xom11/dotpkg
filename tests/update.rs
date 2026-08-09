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
