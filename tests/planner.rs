use dotpkg::config;
use dotpkg::lock;
use dotpkg::model::{Installed, SCOOP};
use dotpkg::plan::{plan, Action, SkipReason};
use dotpkg::state::{Ownership, State};

fn installed(name: &str, version: &str) -> Installed {
    Installed {
        backend: SCOOP.into(),
        name: name.into(),
        version: version.into(),
        arch: Some("arm64".into()),
        bucket: Some("main".into()),
    }
}

const DECLARED_FZF: &str = "[scoop]\npackages = [\"fzf\"]\n";
const LOCK_FZF_741: &str =
    "[scoop.fzf]\nbucket = \"main\"\ncommit = \"a28d0c56\"\nversion = \"0.74.1\"\n";

#[test]
fn a_declared_locked_package_that_is_absent_is_an_install() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Install {
            backend: SCOOP.into(),
            name: "fzf".into(),
            version: "0.74.1".into()
        }]
    );
}

#[test]
fn a_package_already_at_the_locked_version_produces_no_action() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1")],
        &State::default(),
        &[],
    );
    assert!(p.actions.is_empty(), "got {:?}", p.actions);
}

#[test]
fn a_newer_installed_version_is_a_downgrade_because_the_lock_is_authoritative() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.2")],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Downgrade {
            backend: SCOOP.into(),
            name: "fzf".into(),
            from: "0.74.2".into(),
            to: "0.74.1".into()
        }]
    );
}

#[test]
fn an_older_installed_version_is_an_upgrade() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.0")],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Upgrade {
            backend: SCOOP.into(),
            name: "fzf".into(),
            from: "0.74.0".into(),
            to: "0.74.1".into()
        }]
    );
}

#[test]
fn a_declared_package_with_no_lock_entry_is_reported_not_resolved() {
    // Spec: apply must fail here rather than resolve latest itself. Phase 1 is
    // read-only, so the planner surfaces it and Phase 2 turns it fatal.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::Lock::default(),
        &[],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: SCOOP.into(),
            name: "fzf".into(),
            reason: SkipReason::NotLocked
        }]
    );
}

#[test]
fn a_running_package_is_skipped_rather_than_changed() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.2")],
        &State::default(),
        &["fzf".into()],
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: SCOOP.into(),
            name: "fzf".into(),
            reason: SkipReason::Running
        }],
        "a running package must never turn into a Downgrade"
    );
}

#[test]
fn a_running_package_already_at_the_locked_version_produces_no_line_at_all() {
    // The `running` check must stay INSIDE the branch that has a change to
    // make. Hoisting it above `match current` -- a refactor anyone might make
    // in good faith -- turns every healthy running app into a spurious `!`
    // line. Nothing else in this file would notice: every other test passes
    // `running = &[]`, and the one that does not has a version mismatch.
    //
    // On a real machine that is most of the list: kanata, nvim, brave.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1")],
        &State::default(),
        &["fzf".into()],
    );
    assert!(
        p.actions.is_empty(),
        "a running package that already matches the lock needs no line: got {:?}",
        p.actions
    );
}

#[test]
fn an_undeclared_owned_package_is_a_prune() {
    let mut state = State::default();
    state.set(SCOOP, "aichat", Ownership::Adopted);

    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1"), installed("aichat", "0.30.0")],
        &state,
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Prune {
            backend: SCOOP.into(),
            name: "aichat".into(),
            version: "0.30.0".into()
        }]
    );
}

#[test]
fn an_undeclared_unowned_package_is_reported_but_never_pruned() {
    // The whole reason dotpkg is safe to install on a populated machine.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[
            installed("fzf", "0.74.1"),
            installed("antigravity", "2.0.6"),
        ],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Unmanaged {
            backend: SCOOP.into(),
            name: "antigravity".into(),
            version: "2.0.6".into()
        }]
    );
}

#[test]
fn scoop_helpers_are_never_reported_as_strays() {
    // Measured on a14: without this, 5 differences are reported and only 2 are
    // real — a 60% false-positive rate is how a feature gets switched off.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[
            installed("fzf", "0.74.1"),
            installed("dark", "3.14"),
            installed("innounp", "0.50"),
            installed("7zip", "26.01"),
            installed("lessmsi", "2.1"),
        ],
        &State::default(),
        &[],
    );
    assert!(p.actions.is_empty(), "got {:?}", p.actions);
}

#[test]
fn a_helper_that_the_user_declared_is_managed_normally() {
    let p = plan(
        &config::parse("[scoop]\npackages = [\"7zip\"]\n").unwrap(),
        &lock::parse(
            "[scoop.\"7zip\"]\nbucket = \"main\"\ncommit = \"abc\"\nversion = \"26.02\"\n",
        )
        .unwrap(),
        &[installed("7zip", "26.01")],
        &State::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Upgrade {
            backend: SCOOP.into(),
            name: "7zip".into(),
            from: "26.01".into(),
            to: "26.02".into()
        }]
    );
}

#[test]
fn actions_are_ordered_installs_then_prunes_then_reports() {
    // Install before uninstall: if a run dies partway, an extra package is
    // easier to live with than a missing one.
    let mut state = State::default();
    state.set(SCOOP, "aichat", Ownership::Adopted);

    let p = plan(
        &config::parse("[scoop]\npackages = [\"fzf\", \"bat\"]\n").unwrap(),
        &lock::parse(
            "[scoop.fzf]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"0.74.1\"\n\
             [scoop.bat]\nbucket=\"main\"\ncommit=\"b\"\nversion=\"0.26.1\"\n",
        )
        .unwrap(),
        &[
            installed("aichat", "0.30.0"),
            installed("antigravity", "2.0.6"),
        ],
        &state,
        &[],
    );

    let kinds: Vec<&str> = p
        .actions
        .iter()
        .map(|a| match a {
            Action::Install { .. } => "install",
            Action::Upgrade { .. } => "upgrade",
            Action::Downgrade { .. } => "downgrade",
            Action::Prune { .. } => "prune",
            Action::Skip { .. } => "skip",
            Action::Unmanaged { .. } => "unmanaged",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["install", "install", "prune", "unmanaged"],
        "got {:?}",
        p.actions
    );
}

#[test]
fn counts_separate_changes_from_skips_and_reports() {
    let p = plan(
        &config::parse("[scoop]\npackages = [\"fzf\", \"bat\"]\n").unwrap(),
        &lock::parse("[scoop.fzf]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"0.74.1\"\n").unwrap(),
        &[installed("antigravity", "2.0.6")],
        &State::default(),
        &[],
    );
    // fzf install = 1 change; bat unlocked = 1 skip; antigravity = report only.
    assert_eq!(p.change_count(), 1);
    assert_eq!(p.skip_count(), 1);
}

#[test]
fn the_planner_source_performs_no_io() {
    // The planner being pure is what lets layer-1 tests run on any OS. A stray
    // subprocess or file read here would quietly make the suite Windows-only.
    //
    // This is an ALLOWLIST, deliberately. The denylist it replaced ("does the
    // source mention `std::fs`?") admitted anything nobody had thought to
    // forbid: `use std::{env, fs, io, process};` contains none of those
    // strings, because a braced group import never spells out `std::fs` at
    // all. An unfamiliar dependency must be refused by default, not admitted
    // by default -- the whole point of the guard is to catch the import whose
    // danger the person adding it did not see.
    const ALLOWED: &[&str] = &[
        "crate::",
        "std::collections",
        // `super::*` re-exports exactly what the lines above already admitted,
        // so it can smuggle nothing in. It is how the in-file test module gets
        // at `is_older`.
        "super::",
    ];
    let src = include_str!("../src/plan.rs");

    for line in src.lines() {
        let line = line.trim();
        let Some(path) = line.strip_prefix("use ") else {
            continue;
        };
        assert!(
            ALLOWED.iter().any(|a| path.starts_with(a)),
            "src/plan.rs must stay pure: `{line}` is not one of {ALLOWED:?}. \
             If this import really is pure, add it to ALLOWED and say why."
        );
    }

    // A `use` line is the usual way in, but not the only one: `std::fs::read`
    // called at full path needs no import at all. Every `std::` the planner
    // names, in code or in prose, must be one this test has vouched for.
    for (i, _) in src.match_indices("std::") {
        let tail = &src[i..];
        assert!(
            tail.starts_with("std::collections"),
            "src/plan.rs must stay pure: fully-qualified `{}` bypasses the import allowlist",
            tail.lines().next().unwrap_or(tail).trim_end()
        );
    }
}
