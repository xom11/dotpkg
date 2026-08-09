use dotpkg::config;
use dotpkg::lock;
use dotpkg::model::{Installed, Name, Running, SCOOP, WINGET};
use dotpkg::plan::{plan, Action, Divergence, SkipReason};
use dotpkg::state::{Ownership, State};
use std::collections::BTreeSet;

fn installed(name: &str, version: &str) -> Installed {
    Installed {
        backend: SCOOP.into(),
        name: name.into(),
        version: version.into(),
        arch: Some("arm64".into()),
        bucket: Some("main".into()),
        bins: Vec::new(),
    }
}

/// An installed winget package: no `arch`, no `bucket` -- winget exposes
/// neither (see `Installed`'s own doc comment) -- and no `bins`, since there
/// is no winget-side manifest to read executable names from.
fn installed_winget(id: &str, version: &str) -> Installed {
    Installed {
        backend: WINGET.into(),
        name: Name::new(id),
        version: version.into(),
        arch: None,
        bucket: None,
        bins: Vec::new(),
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
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Install {
            backend: SCOOP.into(),
            name: "fzf".into(),
            version: "0.74.1".into(),
            arch: None,
        }]
    );
}

#[test]
fn a_package_already_at_the_locked_version_produces_no_action() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1")],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert!(p.actions.is_empty(), "got {:?}", p.actions);
}

#[test]
fn a_newer_installed_version_is_a_downgrade_because_the_lock_is_authoritative() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.2")],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Downgrade {
            backend: SCOOP.into(),
            name: "fzf".into(),
            from: "0.74.2".into(),
            to: "0.74.1".into(),
            // No declared [scoop.opts] entry for fzf, so the resolution keeps
            // what `installed()` says is already there.
            arch: Some("arm64".into()),
        }]
    );
}

#[test]
fn an_older_installed_version_is_an_upgrade() {
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.0")],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Upgrade {
            backend: SCOOP.into(),
            name: "fzf".into(),
            from: "0.74.0".into(),
            to: "0.74.1".into(),
            arch: Some("arm64".into()),
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
        &[],
        &State::default(),
        &Running::default(),
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
        &[],
        &State::default(),
        &Running::new(BTreeSet::from(["fzf".to_string()]), Default::default()),
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
    // `running = &Running::default()`, and the one that does not has a version
    // mismatch.
    //
    // On a real machine that is most of the list: kanata, nvim, brave.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1")],
        &[],
        &State::default(),
        &Running::new(BTreeSet::from(["fzf".to_string()]), Default::default()),
    );
    assert!(
        p.actions.is_empty(),
        "a running package that already matches the lock needs no line: got {:?}",
        p.actions
    );
}

#[test]
fn a_declared_package_is_not_upgraded_when_only_its_manifest_names_the_process() {
    // The realistic neovim: the package is `neovim`, the live process is
    // `nvim`, and only the manifest's bin field connects the two -- an exact
    // name match would miss it entirely. This is the version-change half of
    // `a_running_package_is_not_pruned_when_only_its_manifest_names_the_process`;
    // the finding that named this whole phase was exactly this case: "a
    // neovim upgrade planned cleanly while nvim.exe was running"
    // (docs/phase2-notes.md).
    let mut inst = installed("neovim", "0.10.0");
    inst.bins = vec!["nvim".to_string()];

    let p = plan(
        &config::parse("[scoop]\npackages = [\"neovim\"]\n").unwrap(),
        &lock::parse("[scoop.neovim]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"0.10.1\"\n")
            .unwrap(),
        &[inst],
        &[],
        &State::default(),
        &Running::new(BTreeSet::from(["nvim".to_string()]), Default::default()),
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: SCOOP.into(),
            name: "neovim".into(),
            reason: SkipReason::Running
        }],
        "a running package must never turn into an Upgrade"
    );
}

#[test]
fn a_declared_unlocked_winget_package_is_reported_rather_than_silently_dropped() {
    // Winget has had a real `BackendView` since Task 14, but its capability
    // is `ReportsOnly` -- a declared package with no lock entry is
    // `ReportedOnly(Divergence::NotLocked)`, NOT `SkipReason::NotLocked`.
    // `NotLocked` fails the whole `apply` run (see `Divergence::NotLocked`'s
    // own doc comment for why that is right for scoop and wrong here); this
    // task's own review caught that using it for winget too broke `apply`
    // outright for anyone whose pkg.toml has a `[winget]` section with no
    // matching lock entries -- which, before `update` resolves winget at
    // all, is every one of them. Reporting it at all is still the point: the
    // spec's example pkg.toml declares `[winget]`, and a user who copies it
    // must not be told `nothing to do`. Silence is the one answer that is
    // indistinguishable from "dotpkg never read your file".
    let p = plan(
        &config::parse("[winget]\npackages = [\"Git.Git\", \"Brave.Brave\"]\n").unwrap(),
        &lock::Lock::default(),
        &[],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![
            Action::Skip {
                backend: WINGET.into(),
                name: "Git.Git".into(),
                reason: SkipReason::ReportedOnly(Divergence::NotLocked),
            },
            Action::Skip {
                backend: WINGET.into(),
                name: "Brave.Brave".into(),
                reason: SkipReason::ReportedOnly(Divergence::NotLocked),
            },
        ]
    );
    assert_eq!(p.skip_count(), 2);
    assert_eq!(
        p.change_count(),
        0,
        "nothing in this plan will be done, so nothing must count as a change"
    );
}

#[test]
fn declaring_winget_packages_does_not_disturb_the_scoop_plan() {
    let p = plan(
        &config::parse("[scoop]\npackages = [\"fzf\"]\n\n[winget]\npackages = [\"Git.Git\"]\n")
            .unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![
            Action::Install {
                backend: SCOOP.into(),
                name: "fzf".into(),
                version: "0.74.1".into(),
                arch: None,
            },
            Action::Skip {
                backend: WINGET.into(),
                name: "Git.Git".into(),
                reason: SkipReason::ReportedOnly(Divergence::NotLocked),
            },
        ]
    );
    assert_eq!(p.change_count(), 1);
}

#[test]
fn a_declared_locked_winget_package_with_nothing_installed_is_reported_as_a_would_be_install() {
    // The `None` arm of the same match `a_declared_locked_package_that_is_
    // absent_is_an_install` exercises for scoop -- but winget's view has
    // `Capability::ReportsOnly`, so what would have been an Install becomes
    // a Skip carrying the version it would have installed.
    let p = plan(
        &config::parse("[winget]\npackages = [\"Git.Git\"]\n").unwrap(),
        &lock::parse("[winget.\"Git.Git\"]\nversion = \"2.55.0\"\npin = \"version-only\"\n")
            .unwrap(),
        &[],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: WINGET.into(),
            name: "Git.Git".into(),
            reason: SkipReason::ReportedOnly(Divergence::Install {
                version: "2.55.0".into(),
            }),
        }]
    );
    assert_eq!(
        p.change_count(),
        0,
        "an install dotpkg cannot perform yet must not count as a change"
    );
}

#[test]
fn a_winget_package_that_differs_from_the_lock_is_reported_with_its_diff() {
    let p = plan(
        &config::parse("[winget]\npackages = [\"Brave.Brave\"]\n").unwrap(),
        &lock::parse(
            "[winget.\"Brave.Brave\"]\nversion = \"151.1.93.134\"\npin = \"version-only\"\n",
        )
        .unwrap(),
        &[installed_winget("Brave.Brave", "151.1.93.132")],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: WINGET.into(),
            name: "Brave.Brave".into(),
            reason: SkipReason::ReportedOnly(Divergence::Change {
                from: "151.1.93.132".into(),
                to: "151.1.93.134".into(),
            }),
        }]
    );
}

#[test]
fn a_reported_only_package_is_not_counted_as_a_change() {
    // change_count() prints "N changes, M skipped. Continue?" -- the one line
    // the user reads before saying yes. Counting a change that will never
    // happen puts a false number in it, which is the defect class Phase 3
    // fixed twice in render.rs.
    let p = plan(
        &config::parse("[winget]\npackages = [\"Brave.Brave\"]\n").unwrap(),
        &lock::parse(
            "[winget.\"Brave.Brave\"]\nversion = \"151.1.93.134\"\npin = \"version-only\"\n",
        )
        .unwrap(),
        &[installed_winget("Brave.Brave", "151.1.93.132")],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.change_count(), 0, "nothing in this plan will be done");
    assert_eq!(p.skip_count(), 1);
}

#[test]
fn a_winget_package_already_at_the_locked_version_produces_no_action_either() {
    // The positive control for the two tests above: a `ReportsOnly` backend
    // must still recognise convergence and say nothing, exactly like scoop's
    // `a_package_already_at_the_locked_version_produces_no_action`.
    let p = plan(
        &config::parse("[winget]\npackages = [\"Brave.Brave\"]\n").unwrap(),
        &lock::parse(
            "[winget.\"Brave.Brave\"]\nversion = \"151.1.93.134\"\npin = \"version-only\"\n",
        )
        .unwrap(),
        &[installed_winget("Brave.Brave", "151.1.93.134")],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert!(p.actions.is_empty(), "got {:?}", p.actions);
}

#[test]
fn a_running_winget_package_that_differs_from_the_lock_is_skipped_as_running_not_reported() {
    // Running outranks ReportedOnly, the same way it outranks Upgrade/
    // Downgrade for scoop: a live process is the more urgent fact regardless
    // of whether this build could act on the package at all.
    let p = plan(
        &config::parse("[winget]\npackages = [\"Brave.Brave\"]\n").unwrap(),
        &lock::parse(
            "[winget.\"Brave.Brave\"]\nversion = \"151.1.93.134\"\npin = \"version-only\"\n",
        )
        .unwrap(),
        &[installed_winget("Brave.Brave", "151.1.93.132")],
        &[],
        &State::default(),
        &Running::new(
            BTreeSet::from(["brave.brave".to_string()]),
            Default::default(),
        ),
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: WINGET.into(),
            name: "Brave.Brave".into(),
            reason: SkipReason::Running,
        }],
        "a running winget package must never turn into a ReportedOnly line"
    );
}

#[test]
fn an_owned_undeclared_winget_package_is_reported_not_pruned() {
    let mut state = State::default();
    state.set(WINGET, &Name::new("OpenAI.Codex"), Ownership::Adopted);
    let p = plan(
        &config::parse("[winget]\npackages = []\n[scoop]\npackages = [\"fzf\"]\n").unwrap(),
        &lock::parse("").unwrap(),
        &[installed_winget("OpenAI.Codex", "0.145.0")],
        &[],
        &state,
        &Running::default(),
    );
    assert!(
        !p.actions.iter().any(|a| matches!(a, Action::Prune { .. })),
        "dotpkg cannot uninstall a winget package in this phase: {:?}",
        p.actions
    );
    assert!(p.actions.iter().any(|a| matches!(
        a,
        Action::Skip {
            reason: SkipReason::ReportedOnly(Divergence::Prune { .. }),
            ..
        }
    )));
}

#[test]
fn an_undeclared_owned_package_is_a_prune() {
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);

    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1"), installed("aichat", "0.30.0")],
        &[],
        &state,
        &Running::default(),
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
        &[],
        &State::default(),
        &Running::default(),
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
        &[],
        &State::default(),
        &Running::default(),
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
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Upgrade {
            backend: SCOOP.into(),
            name: "7zip".into(),
            from: "26.01".into(),
            to: "26.02".into(),
            arch: Some("arm64".into()),
        }]
    );
}

#[test]
fn actions_are_ordered_installs_then_prunes_then_reports() {
    // Install before uninstall: if a run dies partway, an extra package is
    // easier to live with than a missing one. `python` is here so the
    // ArchDrift arm below is not just dead code in this match: the design
    // says drift "joins Unmanaged after the prunes", and that claim needs a
    // scenario that actually produces one.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);

    let p = plan(
        &config::parse(
            "[scoop]\npackages = [\"fzf\", \"bat\", \"python\"]\n\n\
             [scoop.opts]\npython = { arch = \"arm64\" }\n",
        )
        .unwrap(),
        &lock::parse(
            "[scoop.fzf]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"0.74.1\"\n\
             [scoop.bat]\nbucket=\"main\"\ncommit=\"b\"\nversion=\"0.26.1\"\n\
             [scoop.python]\nbucket=\"main\"\ncommit=\"c\"\nversion=\"3.14.5\"\n",
        )
        .unwrap(),
        &[
            installed("aichat", "0.30.0"),
            installed("antigravity", "2.0.6"),
            installed_arch("python", "3.14.5", Some("64bit")),
        ],
        &[],
        &state,
        &Running::default(),
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
            Action::ArchDrift { .. } => "archdrift",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["install", "install", "prune", "archdrift", "unmanaged"],
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
        &[],
        &State::default(),
        &Running::default(),
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

#[test]
fn a_case_difference_between_pkg_toml_and_disk_is_not_two_packages() {
    // Before Name, this planned Install{FZF} then Prune{fzf} -- the same app --
    // and because prune runs last, apply would have uninstalled what it had
    // just installed. Verified against the merged Phase 1 planner.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("fzf"), Ownership::Installed);

    let p = plan(
        &config::parse("[scoop]\npackages = [\"FZF\"]\n").unwrap(),
        &lock::parse("[scoop.FZF]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"0.74.1\"\n").unwrap(),
        &[installed("fzf", "0.74.1")],
        &[],
        &state,
        &Running::default(),
    );
    assert!(
        p.actions.is_empty(),
        "expected no action, got {:?}",
        p.actions
    );
}

#[test]
fn a_case_difference_in_scoop_opts_still_finds_the_package() {
    // Second instance of the same bug, in a different map.
    let cfg = config::parse(
        "[scoop]\npackages = [\"python\"]\n\n[scoop.opts]\nPython = { arch = \"64bit\" }\n",
    )
    .unwrap();
    assert!(cfg.scoop.opts.contains_key(&Name::new("python")));
}

#[test]
fn a_running_package_is_never_pruned() {
    // The prune loop did not consult `running` at all -- not a mismatched
    // comparison, an absent one. Verified against the merged Phase 1 planner
    // with an exact name match, which had no excuse to miss:
    //   kanata running + owned + removed from pkg.toml  ->  Prune{kanata}
    // Prune is worse than the upgrade case it sits beside: an upgrade puts the
    // app back, a prune does not.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("kanata"), Ownership::Installed);

    let p = plan(
        &config::parse("[scoop]\npackages = []\n").unwrap(),
        &lock::Lock::default(),
        &[installed("kanata", "1.12.0")],
        &[],
        &state,
        &Running::new(BTreeSet::from(["kanata".to_string()]), Default::default()),
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: SCOOP.into(),
            name: Name::new("kanata"),
            reason: SkipReason::Running
        }],
        "a running package must never turn into a Prune"
    );
}

#[test]
fn a_running_package_is_not_pruned_when_only_its_manifest_names_the_process() {
    // The realistic kanata: the package is `kanata`, the live process is
    // kanata_windows_tty_winIOv2_arm64.exe.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("kanata"), Ownership::Installed);

    let mut inst = installed("kanata", "1.12.0");
    inst.bins = vec!["kanata_windows_tty_winiov2_arm64".to_string()];

    let p = plan(
        &config::parse("[scoop]\npackages = []\n").unwrap(),
        &lock::Lock::default(),
        &[inst],
        &[],
        &state,
        &Running::new(
            BTreeSet::from(["kanata_windows_tty_winiov2_arm64".to_string()]),
            Default::default(),
        ),
    );
    assert!(
        matches!(
            p.actions.as_slice(),
            [Action::Skip {
                reason: SkipReason::Running,
                ..
            }]
        ),
        "got {:?}",
        p.actions
    );
}

#[test]
fn an_idle_owned_undeclared_package_is_still_pruned() {
    // The guard must not turn the prune off altogether.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);

    let p = plan(
        &config::parse("[scoop]\npackages = []\n").unwrap(),
        &lock::Lock::default(),
        &[installed("aichat", "0.30.0")],
        &[],
        &state,
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Prune {
            backend: SCOOP.into(),
            name: Name::new("aichat"),
            version: "0.30.0".into()
        }]
    );
}

const ARM64_PYTHON: &str =
    "[scoop]\npackages = [\"python\"]\n\n[scoop.opts]\npython = { arch = \"arm64\" }\n";

fn installed_arch(name: &str, version: &str, arch: Option<&str>) -> Installed {
    let mut i = installed(name, version);
    i.arch = arch.map(|a| a.to_string());
    i
}

#[test]
fn a_package_installed_for_the_wrong_architecture_is_reported() {
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert!(
        p.actions.contains(&Action::ArchDrift {
            backend: SCOOP.into(),
            name: Name::new("python"),
            have: "64bit".into(),
            want: "arm64".into(),
        }),
        "got {:?}",
        p.actions
    );
}

#[test]
fn architecture_comparison_ignores_case() {
    // Scoop writes lowercase today, so this is speculative -- but the thesis
    // of this branch is that case-sensitive comparison in the decision layer
    // is how software gets removed, and in Phase 2b drift may drive a
    // reinstall.
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", Some("ARM64"))],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.drift_count(), 0, "got {:?}", p.actions);
}

#[test]
fn drift_is_reported_even_without_a_lock_entry() {
    // Otherwise the report is invisible on any machine that has not run
    // `dotpkg update` -- which is every machine today, including the one this
    // gets dogfooded on.
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.drift_count(), 1, "got {:?}", p.actions);
}

#[test]
fn an_unknown_installed_architecture_is_not_drift() {
    // install.json only appeared in later scoop versions. Treating unknown as
    // wrong would make dotpkg want to reinstall such apps on every run.
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", None)],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.drift_count(), 0, "got {:?}", p.actions);
}

#[test]
fn keep_means_never_report_whatever_is_installed() {
    let p = plan(
        &config::parse(
            "[scoop]\npackages = [\"rustup\"]\n\n[scoop.opts]\nrustup = { arch = \"keep\" }\n",
        )
        .unwrap(),
        &lock::Lock::default(),
        &[installed_arch("rustup", "1.28.0", Some("64bit"))],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.drift_count(), 0, "got {:?}", p.actions);
}

#[test]
fn an_undeclared_architecture_is_no_opinion_and_no_report() {
    let p = plan(
        &config::parse("[scoop]\npackages = [\"python\"]\n").unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.drift_count(), 0, "got {:?}", p.actions);
}

#[test]
fn drift_is_a_report_not_a_change() {
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::Lock::default(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.change_count(), 0, "drift must not count as a change");
}

#[test]
fn a_package_can_be_both_an_upgrade_and_a_drift() {
    // Two true facts. Suppressing one would need a rule the reader has to
    // remember, and 2b may well fix the arch by way of the upgrade anyway.
    let p = plan(
        &config::parse(ARM64_PYTHON).unwrap(),
        &lock::parse("[scoop.python]\nbucket=\"main\"\ncommit=\"a\"\nversion=\"3.14.6\"\n")
            .unwrap(),
        &[installed_arch("python", "3.14.5", Some("64bit"))],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(p.change_count(), 1);
    assert_eq!(p.drift_count(), 1);
}

#[test]
fn an_owned_undeclared_helper_is_pruned_rather_than_silently_kept_forever() {
    // The helper list exists to stop dotpkg reporting scoop's own extraction
    // tools as strays. It must not also stop dotpkg releasing a helper it
    // installed itself: `plan.rs`'s skip sat above the ownership check, so an
    // owned, undeclared 7zip produced no line of any kind.
    let declared = config::parse("[scoop]\npackages = [\"fzf\"]\n").unwrap();
    let lock = lock::parse("").unwrap();
    let installed = vec![installed("7zip", "26.01"), installed("dark", "3.14.1")];
    let mut state = State::default();
    state.set(SCOOP, &Name::new("7zip"), Ownership::Installed);

    let p = plan(
        &declared,
        &lock,
        &installed,
        &[],
        &state,
        &Running::default(),
    );

    assert!(
        p.actions.iter().any(|a| matches!(
            a, Action::Prune { name, .. } if *name == Name::new("7zip")
        )),
        "an owned helper must be prunable: {:?}",
        p.actions
    );
    assert!(
        !p.actions.iter().any(|a| matches!(
            a, Action::Prune { name, .. } | Action::Unmanaged { name, .. }
                if *name == Name::new("dark")
        )),
        "an unowned helper must still be invisible: {:?}",
        p.actions
    );
}

/// One `[scoop.<name>]` block with a syntactically valid 40-hex commit.
/// The planner never looks at the commit, but `lock::parse` and (from Task 4)
/// `lock_coherence_guard` both do.
fn pin(name: &str, version: &str) -> String {
    format!(
        "[scoop.{name}]\nbucket = \"main\"\ncommit = \"{}\"\nversion = \"{version}\"\n\n",
        "a".repeat(40)
    )
}

#[test]
fn the_architecture_an_install_will_use_is_decided_in_the_plan_not_in_the_executor() {
    // Three cases in one: declared wins, otherwise the installed value is
    // preserved, and `keep` means "pass no -a at all".
    let declared = config::parse(
        "[scoop]\npackages = [\"python\", \"stylua\", \"kanata\"]\n\
         [scoop.opts]\npython = { arch = \"arm64\" }\nkanata = { arch = \"keep\" }\n",
    )
    .unwrap();
    let lock = lock::parse(
        &[
            pin("python", "3.14.6"),
            pin("stylua", "2.5.3"),
            pin("kanata", "1.13.0"),
        ]
        .concat(),
    )
    .unwrap();
    let installed = vec![
        installed_arch("python", "3.14.5", Some("64bit")),
        installed_arch("stylua", "2.5.2", Some("64bit")),
        installed_arch("kanata", "1.12.0", Some("arm64")),
    ];

    let p = plan(
        &declared,
        &lock,
        &installed,
        &[],
        &State::default(),
        &Running::default(),
    );

    // `Option<Option<String>>`, deliberately not flattened: `None` means "no
    // Upgrade action for this name exists at all", and `Some(None)` means "it
    // exists, and its arch is None". Collapsing those to one `Option<String>`
    // (as an earlier version of this test did, via a trailing `?`) made
    // `assert_eq!(arch_of("kanata"), None)` pass whether kanata produced an
    // Upgrade with no arch, as intended, or no action at all -- a plan defect
    // that would have gone uncaught.
    let arch_of = |n: &str| -> Option<Option<String>> {
        p.actions.iter().find_map(|a| match a {
            Action::Upgrade { name, arch, .. } if *name == Name::new(n) => Some(arch.clone()),
            _ => None,
        })
    };
    assert_eq!(
        arch_of("python"),
        Some(Some("arm64".to_string())),
        "declared wins"
    );
    assert_eq!(
        arch_of("stylua"),
        Some(Some("64bit".to_string())),
        "an undeclared package keeps the architecture it already has -- reinstalling \
         it as arm64 would be an unasked-for change"
    );
    assert_eq!(
        arch_of("kanata"),
        Some(None),
        "kanata must still appear as an Upgrade -- `keep` means pass no -a at all, \
         not that kanata is skipped"
    );
}

#[test]
fn a_declared_package_the_scan_could_not_read_is_skipped_rather_than_installed() {
    // Measured on a14: zellij and actionlint are installed at exactly the
    // pinned version, but their manifest cannot be traversed, so scan omits
    // them. plan() used to read that omission as "not installed" and emit
    // Install -- which under --yes is uninstall-then-install of a package
    // that was never absent.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[],                 // scan found nothing readable
        &[Name::new("fzf")], // ...because fzf was opaque
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: SCOOP.into(),
            name: "fzf".into(),
            reason: SkipReason::Opaque,
        }]
    );
}

#[test]
fn an_undeclared_package_the_scan_could_not_read_is_not_a_stray_and_not_a_prune() {
    // The counterweight. An entry whose state is unknown is not evidence of
    // a stray, and it must not become a Prune even when dotpkg owns it --
    // "I cannot see it" is not "it is not declared".
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1")],
        &[Name::new("aichat")],
        &state,
        &Running::default(),
    );
    assert!(p.actions.is_empty(), "got {:?}", p.actions);
}

// `plan_backend`'s `debug_assert!` fires on this exact input (two `Installed`
// entries for one `(backend, name)`), which means the assert half and the
// dedup half of the same invariant cannot both be observed from one
// execution of one test: the `debug_assert!` panics before `plan()` can
// return anything for a `prunes == 1` check to inspect. `tests/execute.rs`'s
// `exit_code_asserts_a_refused_run_changed_nothing` hit the identical shape
// of conflict for a different `debug_assert!` and resolved it by gating a
// `#[should_panic]` test to the profile where the assert exists. Following
// that precedent here for the assert half; the dedup half no longer needs a
// profile-gated twin here at all -- it is unit-tested directly, in every
// profile, as `dedupe_installed_for_backend_keeps_only_the_first_entry_for_a_duplicated_name`
// in `src/plan.rs`, which calls the extracted `dedupe_installed_for_backend`
// without going through `plan_backend`'s assert at all.

/// `plan_backend`'s `debug_assert!`, proven firing rather than merely
/// trusted.
///
/// Gated `debug_assertions`, not `not(debug_assertions)`, for the same reason
/// `tests/execute.rs`'s equivalent test is: `debug_assert!` is compiled out
/// under `cargo test --release`, so under that profile this can never panic
/// and `#[should_panic]` would fail it.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "a backend returned two Installed entries for one name")]
fn a_backend_returning_two_installed_entries_for_one_name_panics_loudly_in_a_debug_build() {
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);
    let _ = plan(
        &config::parse("[scoop]\npackages = []\n").unwrap(),
        &lock::parse("").unwrap(),
        &[installed("aichat", "0.30.0"), installed("aichat", "0.29.0")],
        &[],
        &state,
        &Running::default(),
    );
}

#[test]
fn a_running_owned_undeclared_scoop_package_still_prints_before_the_winget_view() {
    // Task 5 folded scoop's declared and undeclared loops into one
    // per-backend pass (`plan_backend`); Task 14 then deleted the winget stub
    // loop entirely, giving winget the same real `BackendView` scoop already
    // had -- both now run through the identical `for view in &backends`
    // loop in `plan()`, scoop first. The mechanism this test originally
    // pinned (a separate stub loop, positioned after both of scoop's) no
    // longer exists at all, but the OBSERVED order it verified is unchanged:
    // scoop's `Skip{Running}` is pushed straight into the shared `actions`
    // during scoop's own `plan_backend` call, and winget's declared-loop
    // skip is pushed straight into the same `actions` during winget's call,
    // which still runs second because `backends` still lists scoop first.
    //
    // `render.rs` prints `plan.actions` in order with no sort, so this is a
    // real, visible fact: with a declared winget package *and* a running,
    // owned, undeclared scoop package (the only combination that can tell
    // orderings apart), the scoop line still prints before the winget line.
    // Display order only -- the property that actually matters,
    // install-before-uninstall, is untouched (prunes and reports are still
    // appended last, unconditionally) and is pinned separately by
    // `actions_are_ordered_installs_then_prunes_then_reports`.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("kanata"), Ownership::Installed);

    let p = plan(
        &config::parse("[scoop]\npackages = []\n\n[winget]\npackages = [\"Git.Git\"]\n").unwrap(),
        &lock::Lock::default(),
        &[installed("kanata", "1.12.0")],
        &[],
        &state,
        &Running::new(BTreeSet::from(["kanata".to_string()]), Default::default()),
    );
    assert_eq!(
        p.actions,
        vec![
            Action::Skip {
                backend: SCOOP.into(),
                name: "kanata".into(),
                reason: SkipReason::Running,
            },
            Action::Skip {
                backend: WINGET.into(),
                name: "Git.Git".into(),
                // No lock at all, and winget's capability is `ReportsOnly`,
                // so this is `ReportedOnly(NotLocked)`, not the fatal
                // `SkipReason::NotLocked` a scoop package in the same shape
                // would get.
                reason: SkipReason::ReportedOnly(Divergence::NotLocked),
            },
        ],
        "got {:?}",
        p.actions
    );
}

#[test]
fn a_prerelease_suffix_does_not_reduce_to_the_release_version() {
    // src/plan.rs's own doc comment claimed 1.0.0-rc1 and 1.0.0 reduce to the
    // same [1,0,0] and compare equal. `parts` keeps every numeric run, so rc1
    // becomes [1,0,0,1], and the displayed arrow was inverted for every
    // suffixed version.
    let declared = config::parse("[scoop]\npackages = [\"tool\"]\n").unwrap();
    let lock = lock::parse(
        "[scoop.tool]\nbucket = \"main\"\ncommit = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let installed = vec![installed("tool", "1.0.0-rc1")];

    let p = plan(
        &declared,
        &lock,
        &installed,
        &[],
        &State::default(),
        &Running::default(),
    );

    assert!(
        matches!(p.actions.first(), Some(Action::Downgrade { .. })),
        "1.0.0-rc1 -> 1.0.0 is [1,0,0,1] -> [1,0,0], which this function calls a \
         downgrade; the comment claiming they compare equal was wrong: {:?}",
        p.actions
    );
}
