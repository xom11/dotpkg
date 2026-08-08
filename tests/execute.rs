//! The executor against real directory trees and a fake that lies exactly the
//! way scoop was measured to lie.

use dotpkg::execute::*;
use dotpkg::model::{Name, Running, SCOOP};
use dotpkg::state::{Ownership, State};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

const BODY_A: &str = r#"{"version":"1.0.0","url":"https://good/v1.zip"}"#;

struct Tree(tempfile::TempDir);
impl Tree {
    fn new() -> Tree {
        Tree(tempfile::tempdir().unwrap())
    }
    fn root(&self) -> &Path {
        self.0.path()
    }
    fn stage(&self, app: &str, version: &str, body: &str) -> PathBuf {
        let d = self.root().join("stage").join(app).join(version);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("{app}.json"));
        std::fs::write(&p, body).unwrap();
        p
    }
    fn install(&self, app: &str, body: &str) {
        let cur = self.root().join("apps").join(app).join("current");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::write(cur.join("manifest.json"), body).unwrap();
    }
    fn half_install(&self, app: &str, version: &str) {
        let d = self.root().join("apps").join(app).join(version);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("thing.zip"), b"PK\x03\x04").unwrap();
    }
    fn empty_apps(&self) {
        std::fs::create_dir_all(self.root().join("apps")).unwrap();
    }
}

/// A fake scoop.
///
/// The two booleans exist because the executor's whole job is to disbelieve a
/// tool that reports success, and a fake that always tells the truth cannot
/// test that. Both default to scoop's measured worst behaviour: exit 0, having
/// done nothing. See `docs/specs/2026-08-08-phase2b2-executor-design.md`.
///
/// It deliberately does NOT serve its own idea of state back: observation goes
/// through the real filesystem, so a test cannot pass merely because the fake
/// is self-consistent.
struct Fake<'a> {
    tree: &'a Tree,
    uninstall_really_removes: bool,
    install_really_installs: bool,
    calls: RefCell<Vec<String>>,
}

impl<'a> Fake<'a> {
    fn honest(tree: &'a Tree) -> Fake<'a> {
        Fake {
            tree,
            uninstall_really_removes: true,
            install_really_installs: true,
            calls: RefCell::new(Vec::new()),
        }
    }
    fn silent_install(tree: &'a Tree) -> Fake<'a> {
        Fake {
            tree,
            uninstall_really_removes: true,
            install_really_installs: false,
            calls: RefCell::new(Vec::new()),
        }
    }
    fn silent_uninstall(tree: &'a Tree) -> Fake<'a> {
        Fake {
            tree,
            uninstall_really_removes: false,
            install_really_installs: true,
            calls: RefCell::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Mutator for Fake<'_> {
    fn uninstall(&self, app: &Name) -> anyhow::Result<CommandReport> {
        self.calls
            .borrow_mut()
            .push(format!("uninstall {}", app.key()));
        if self.uninstall_really_removes {
            let _ = std::fs::remove_dir_all(self.tree.root().join("apps").join(app.key()));
        }
        Ok(CommandReport {
            code: Some(0),
            stdout: "ERROR 'x' isn't installed.\n".into(),
            stderr: String::new(),
        })
    }
    fn install(&self, manifest: &Path, _arch: Option<&str>) -> anyhow::Result<CommandReport> {
        self.calls
            .borrow_mut()
            .push(format!("install {}", manifest.display()));
        if self.install_really_installs {
            let app = manifest.file_stem().unwrap().to_string_lossy().into_owned();
            let body = std::fs::read(manifest).unwrap();
            let cur = self.tree.root().join("apps").join(app).join("current");
            std::fs::create_dir_all(&cur).unwrap();
            std::fs::write(cur.join("manifest.json"), body).unwrap();
        }
        Ok(CommandReport {
            code: Some(0),
            stdout: "WARN  'fzf' (0.74.1) is already installed.\n".into(),
            stderr: String::new(),
        })
    }
}

#[test]
fn an_install_scoop_silently_did_not_perform_is_reported_and_not_claimed() {
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    let fake = Fake::silent_install(&t);
    let mut state = State::default();

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Install {
            app: Name::new("fzf"),
            staged,
            arch: Some("arm64".into()),
        },
    );

    let StepOutcome::Failed { why, touched } = out else {
        panic!("a silent no-op must be a failure, got {out:?}")
    };
    assert!(why.contains("install did not happen"), "{why}");
    assert!(
        !touched,
        "a plain Install has no uninstall half to have already touched anything"
    );
    assert!(
        !state.owns(SCOOP, &Name::new("fzf")),
        "a failed install must not be recorded as owned"
    );
    // Not decoration: under a `verdict`-always-fails mutation the substring
    // assertion above stays green and only this one fires.
    assert_eq!(
        fake.calls().len(),
        2,
        "an absent tree earns exactly one retry: {:?}",
        fake.calls()
    );
}

#[test]
fn a_successful_install_is_claimed_exactly_once() {
    // The positive control for the test above. Without it, a run_step that
    // always fails passes that test.
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    let fake = Fake::honest(&t);
    let mut state = State::default();

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Install {
            app: Name::new("fzf"),
            staged,
            arch: None,
        },
    );

    assert_eq!(out, StepOutcome::Done);
    assert_eq!(
        state.ownership(SCOOP, &Name::new("fzf")),
        Some(Ownership::Installed)
    );
    assert_eq!(fake.calls().len(), 1, "a successful install is not retried");
}

#[test]
fn a_half_install_earns_no_retry() {
    // Retrying over a half-install gets `WARN ... is already installed`, exit
    // 0, and no change -- manufacturing the silent success verification exists
    // to catch. The retry gate is therefore on "nothing there at all", not on
    // "something went wrong".
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.half_install("fzf", "1.0.0");
    let fake = Fake::silent_install(&t);
    let mut state = State::default();

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Install {
            app: Name::new("fzf"),
            staged,
            arch: None,
        },
    );

    assert!(matches!(out, StepOutcome::Failed { .. }), "{out:?}");
    assert_eq!(fake.calls().len(), 1, "{:?}", fake.calls());
}

#[test]
fn a_replace_whose_uninstall_did_nothing_never_reaches_the_install() {
    let t = Tree::new();
    let staged = t.stage("fzf", "0.74.2", r#"{"version":"0.74.2"}"#);
    t.install("fzf", r#"{"version":"0.74.1"}"#);
    let fake = Fake::silent_uninstall(&t);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("fzf"), Ownership::Adopted);

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Replace {
            app: Name::new("fzf"),
            staged,
            arch: None,
        },
    );

    let StepOutcome::Failed { why, touched } = out else {
        panic!("got {out:?}")
    };
    assert!(why.contains("uninstall did not happen"), "{why}");
    assert!(
        !touched,
        "verdict never confirmed the uninstall, so the machine is exactly as it was"
    );
    assert_eq!(
        fake.calls(),
        vec!["uninstall fzf".to_string()],
        "install must not run after an unverified uninstall"
    );
    assert_eq!(
        state.ownership(SCOOP, &Name::new("fzf")),
        Some(Ownership::Adopted),
        "a failed replace changes no ownership, and must not downgrade adopt to installed"
    );
}

#[test]
fn a_successful_replace_of_an_adopted_package_keeps_it_adopted() {
    let t = Tree::new();
    let staged = t.stage("fzf", "0.74.2", r#"{"version":"0.74.2"}"#);
    t.install("fzf", r#"{"version":"0.74.1"}"#);
    let fake = Fake::honest(&t);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("fzf"), Ownership::Adopted);

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Replace {
            app: Name::new("fzf"),
            staged,
            arch: None,
        },
    );

    assert_eq!(out, StepOutcome::Done);
    assert_eq!(
        state.ownership(SCOOP, &Name::new("fzf")),
        Some(Ownership::Adopted),
        "upgrading an adopted package must not silently reclassify it as installed"
    );
}

#[test]
fn a_prune_releases_ownership_only_after_the_disk_agrees() {
    let t = Tree::new();
    t.install("aichat", BODY_A);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);

    let liar = Fake::silent_uninstall(&t);
    let out = run_step(
        t.root(),
        &liar,
        &mut state,
        &Step::Remove {
            app: Name::new("aichat"),
        },
    );
    assert!(matches!(out, StepOutcome::Failed { .. }), "{out:?}");
    assert!(
        state.owns(SCOOP, &Name::new("aichat")),
        "a package still on disk must still be owned -- releasing here leaves it \
         installed and unmanageable, and `dotpkg adopt` does not exist"
    );

    let honest = Fake::honest(&t);
    let out = run_step(
        t.root(),
        &honest,
        &mut state,
        &Step::Remove {
            app: Name::new("aichat"),
        },
    );
    assert_eq!(out, StepOutcome::Done);
    assert_eq!(state.owned_count(SCOOP), 0);
}

#[test]
fn installs_precede_replacements_precede_removals_and_git_goes_last() {
    let s = |n: &str| PathBuf::from(format!("/stage/{n}.json"));
    let steps = vec![
        Step::Replace {
            app: Name::new("git"),
            staged: s("git"),
            arch: None,
        },
        Step::Remove {
            app: Name::new("aichat"),
        },
        Step::Replace {
            app: Name::new("bat"),
            staged: s("bat"),
            arch: None,
        },
        Step::Install {
            app: Name::new("ripgrep"),
            staged: s("ripgrep"),
            arch: None,
        },
        Step::Replace {
            app: Name::new("7zip"),
            staged: s("7zip"),
            arch: None,
        },
    ];
    let got: Vec<String> = order(steps)
        .iter()
        .map(|s| s.app().key().to_string())
        .collect();
    assert_eq!(
        got,
        vec!["ripgrep", "bat", "7zip", "git", "aichat"],
        "a pure install opens no window at all and goes first; git is the binary \
         staging needs and is itself scoop-managed, so it goes last in its group"
    );
}

#[test]
fn one_packages_failure_does_not_stop_its_neighbours() {
    let t = Tree::new();
    let good = t.stage("bat", "1.0.0", BODY_A);
    let bad = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    // A fake that installs everything except fzf.
    struct Picky<'a> {
        tree: &'a Tree,
        calls: RefCell<Vec<String>>,
    }
    impl Mutator for Picky<'_> {
        fn uninstall(&self, app: &Name) -> anyhow::Result<CommandReport> {
            self.calls
                .borrow_mut()
                .push(format!("uninstall {}", app.key()));
            let _ = std::fs::remove_dir_all(self.tree.root().join("apps").join(app.key()));
            Ok(CommandReport {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
        fn install(&self, manifest: &Path, _a: Option<&str>) -> anyhow::Result<CommandReport> {
            let app = manifest.file_stem().unwrap().to_string_lossy().into_owned();
            self.calls.borrow_mut().push(format!("install {app}"));
            if app != "fzf" {
                let cur = self.tree.root().join("apps").join(&app).join("current");
                std::fs::create_dir_all(&cur).unwrap();
                std::fs::write(cur.join("manifest.json"), std::fs::read(manifest).unwrap())
                    .unwrap();
            }
            Ok(CommandReport {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }
    let fake = Picky {
        tree: &t,
        calls: RefCell::new(Vec::new()),
    };
    let mut state = State::default();

    let ex = execute(
        t.root(),
        vec![
            Step::Install {
                app: Name::new("fzf"),
                staged: bad,
                arch: None,
            },
            Step::Install {
                app: Name::new("bat"),
                staged: good,
                arch: None,
            },
        ],
        &fake,
        &mut state,
        &|| Running::default(),
        &ExecOptions::default(),
    )
    .unwrap();

    assert_eq!(ex.failed(), 1);
    assert_eq!(
        ex.changed(),
        1,
        "bat must still be installed: {:?}",
        ex.results
    );
    assert!(state.owns(SCOOP, &Name::new("bat")));
    assert!(!state.owns(SCOOP, &Name::new("fzf")));
}

#[test]
fn a_failure_does_not_stop_a_neighbour_that_sorts_after_it() {
    // The test above uses `bat` (good) and `fzf` (bad); `order()` sorts
    // installs alphabetically, so `bat` always runs BEFORE `fzf` no matter
    // what `execute` does once `fzf` fails -- there is nothing left in the
    // list afterwards for a "stop after the first failure" regression to
    // skip. That test alone cannot catch such a regression. This one puts
    // the failing package first alphabetically, so a stop-after-failure bug
    // would leave the package after it never attempted.
    let t = Tree::new();
    let bad = t.stage("alpha-broken", "1.0.0", BODY_A);
    let good = t.stage("zulu-fine", "1.0.0", BODY_A);
    t.empty_apps();
    struct Picky<'a> {
        tree: &'a Tree,
    }
    impl Mutator for Picky<'_> {
        fn uninstall(&self, app: &Name) -> anyhow::Result<CommandReport> {
            let _ = std::fs::remove_dir_all(self.tree.root().join("apps").join(app.key()));
            Ok(CommandReport {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
        fn install(&self, manifest: &Path, _a: Option<&str>) -> anyhow::Result<CommandReport> {
            let app = manifest.file_stem().unwrap().to_string_lossy().into_owned();
            if app != "alpha-broken" {
                let cur = self.tree.root().join("apps").join(&app).join("current");
                std::fs::create_dir_all(&cur).unwrap();
                std::fs::write(cur.join("manifest.json"), std::fs::read(manifest).unwrap())
                    .unwrap();
            }
            Ok(CommandReport {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }
    let fake = Picky { tree: &t };
    let mut state = State::default();

    let ex = execute(
        t.root(),
        vec![
            Step::Install {
                app: Name::new("alpha-broken"),
                staged: bad,
                arch: None,
            },
            Step::Install {
                app: Name::new("zulu-fine"),
                staged: good,
                arch: None,
            },
        ],
        &fake,
        &mut state,
        &|| Running::default(),
        &ExecOptions::default(),
    )
    .unwrap();

    assert_eq!(ex.failed(), 1, "{:?}", ex.results);
    assert_eq!(
        ex.changed(),
        1,
        "zulu-fine sorts after the failing package and must still be attempted: {:?}",
        ex.results
    );
    assert!(state.owns(SCOOP, &Name::new("zulu-fine")));
    assert!(!state.owns(SCOOP, &Name::new("alpha-broken")));
}

#[test]
fn a_package_that_started_running_between_the_plan_and_the_mutation_is_skipped() {
    // `running` must be RE-sampled immediately before each step, not
    // snapshotted once at the top: a package that was quiet at plan time (so
    // the planner never turned it into `Skip{Running}`, and it exists here as
    // a `Step` at all) can start running while an earlier step in the same
    // run is still downloading -- a prefetch of two dozen packages can take
    // minutes. The sampler below proves the RE-sample and not just the
    // check: it reports nothing running on its first call and reports
    // `nvim-ish` from its second call onward, so only a fresh call made for
    // THIS step -- not a value captured once when `execute` started -- can
    // see it.
    let t = Tree::new();
    let staged = t.stage("aichat", "1.0.0", BODY_A);
    t.install("nvim-ish", BODY_A);
    let fake = Fake::honest(&t);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("nvim-ish"), Ownership::Installed);

    let calls = Cell::new(0usize);
    let running = || {
        let n = calls.get();
        calls.set(n + 1);
        if n == 0 {
            Running::default()
        } else {
            Running::new(
                std::collections::BTreeSet::from(["nvim-ish".to_string()]),
                Default::default(),
            )
        }
    };

    let ex = execute(
        t.root(),
        vec![
            Step::Install {
                app: Name::new("aichat"),
                staged,
                arch: None,
            },
            Step::Remove {
                app: Name::new("nvim-ish"),
            },
        ],
        &fake,
        &mut state,
        &running,
        &ExecOptions::default(),
    )
    .unwrap();

    assert_eq!(ex.held(), 1, "{:?}", ex.results);
    assert_eq!(
        ex.changed(),
        1,
        "aichat installs before nvim-ish starts running: {:?}",
        ex.results
    );
    assert!(
        !fake.calls().iter().any(|c| c.contains("nvim-ish")),
        "nothing may be run against the package that started running: {:?}",
        fake.calls()
    );
    assert!(
        state.owns(SCOOP, &Name::new("nvim-ish")),
        "and it stays owned"
    );
    assert!(
        calls.get() >= 2,
        "the sampler must be re-invoked per step to prove re-sampling, not a single snapshot"
    );
}

#[test]
fn the_recovery_file_is_written_before_anything_is_mutated_and_names_every_artifact() {
    let t = Tree::new();
    let a = t.stage("bat", "1.0.0", BODY_A);
    let b = t.stage("fzf", "0.74.1", BODY_A);
    let out = t.root().join("recover.cmd");
    write_recovery(
        &out,
        &[
            Step::Replace {
                app: Name::new("bat"),
                staged: a.clone(),
                arch: Some("arm64".into()),
            },
            Step::Install {
                app: Name::new("fzf"),
                staged: b.clone(),
                arch: None,
            },
            Step::Remove {
                app: Name::new("aichat"),
            },
        ],
    )
    .unwrap();

    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains(&a.display().to_string()), "{text}");
    assert!(text.contains(&b.display().to_string()), "{text}");
    assert!(
        text.contains("-a arm64"),
        "the architecture must be in the recovery line: {text}"
    );
    assert!(
        !text.contains("aichat"),
        "a removal has nothing to restore and must not appear: {text}"
    );
    assert!(
        !text.contains("uninstall"),
        "the recovery file only ever puts software back: {text}"
    );
}

#[test]
fn a_run_that_changed_nothing_and_a_run_that_changed_something_exit_differently() {
    let clean = Execution::default();
    assert_eq!(clean.exit_code(false), 0);
    assert_eq!(clean.exit_code(true), 2, "refused before starting");

    let mixed = Execution {
        results: vec![
            (Name::new("a"), ItemResult::Done),
            (
                Name::new("b"),
                ItemResult::Failed {
                    why: "no".into(),
                    touched: false,
                },
            ),
        ],
        dropped_ghosts: Vec::new(),
        recovery_write_failed: None,
    };
    assert_eq!(
        mixed.exit_code(false),
        1,
        "something changed and something failed"
    );

    let untouched_failure = Execution {
        results: vec![(
            Name::new("c"),
            ItemResult::Failed {
                why: "install did not happen".into(),
                touched: false,
            },
        )],
        dropped_ghosts: Vec::new(),
        recovery_write_failed: None,
    };
    // Important 6: exit codes are defined by what the operator must do next,
    // not by what happened internally. A failure is outstanding whether or
    // not it touched the machine -- the package dotpkg was asked to install
    // still is not there, and 2 is reserved for a refusal before anything
    // was attempted at all.
    assert_eq!(
        untouched_failure.exit_code(false),
        1,
        "a failure is outstanding even when it changed nothing"
    );

    // Important 3: a `Failed` result can still mean the machine changed, if
    // its uninstall half really ran before the failure -- `touched` is how
    // that reaches `exit_code` even though no result is `Done`.
    let touched_but_not_done = Execution {
        results: vec![(
            Name::new("d"),
            ItemResult::Failed {
                why: "install did not happen".into(),
                touched: true,
            },
        )],
        dropped_ghosts: Vec::new(),
        recovery_write_failed: None,
    };
    assert_eq!(
        touched_but_not_done.exit_code(false),
        1,
        "the package is genuinely gone -- 2 would say there is nothing to look at"
    );
}

#[test]
#[should_panic(expected = "cannot also have changed something")]
fn exit_code_asserts_a_refused_run_changed_nothing() {
    // Minor 7's debug_assert: `refused` and "changed something" can never
    // both be true, because a refusal means `execute` returned `Err` before
    // performing a single step.
    let bad = Execution {
        results: vec![(Name::new("a"), ItemResult::Done)],
        dropped_ghosts: Vec::new(),
        recovery_write_failed: None,
    };
    let _ = bad.exit_code(true);
}

#[test]
fn a_held_only_run_is_outstanding_and_exits_1_not_0() {
    // Important 6: under the old rule a package held by the running
    // re-sampler exited 0 -- the same code as a converged machine. The
    // nightly case this fixes is a user who leaves their editor open every
    // night: a scheduled run would report success forever. `held()` alone,
    // with `failed() == 0` and `changed() == 0`, must still be exit 1.
    let held_only = Execution {
        results: vec![(
            Name::new("kanata"),
            ItemResult::Held("started running since the plan was made".into()),
        )],
        dropped_ghosts: Vec::new(),
        recovery_write_failed: None,
    };
    assert_eq!(held_only.exit_code(false), 1);
}

#[test]
fn a_replace_whose_uninstall_really_succeeds_and_whose_install_lies_leaves_the_package_gone_but_still_owned(
) {
    // The path Task 9 left uncovered: `silent_install` means the uninstall
    // half genuinely empties `apps/fzf`, and then the install half reports
    // success without writing anything back. `run_step` must fail this
    // (verified against real disk, not against the fake's own say-so), and
    // `execute` must not have downgraded or dropped the package's ownership
    // on the way -- state must still say `Adopted`, its ORIGINAL variant, so
    // a later run plans a fresh install instead of treating the package as
    // gone-and-forgotten.
    let t = Tree::new();
    let staged = t.stage("fzf", "0.74.2", r#"{"version":"0.74.2"}"#);
    t.install("fzf", r#"{"version":"0.74.1"}"#);
    let fake = Fake::silent_install(&t);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("fzf"), Ownership::Adopted);

    let ex = execute(
        t.root(),
        vec![Step::Replace {
            app: Name::new("fzf"),
            staged,
            arch: None,
        }],
        &fake,
        &mut state,
        &|| Running::default(),
        &ExecOptions::default(),
    )
    .unwrap();

    assert_eq!(ex.failed(), 1, "{:?}", ex.results);
    assert!(
        matches!(&ex.results[0].1, ItemResult::Failed { touched: true, .. }),
        "the uninstall really ran before the install failed -- exit_code must \
         not be able to call this shape untouched: {:?}",
        ex.results
    );
    assert!(
        !t.root().join("apps").join("fzf").exists(),
        "the uninstall half really ran: the app must be genuinely absent from disk"
    );
    assert_eq!(
        state.ownership(SCOOP, &Name::new("fzf")),
        Some(Ownership::Adopted),
        "still owned, and still its ORIGINAL ownership variant -- so a later run \
         re-installs it instead of treating it as orphaned"
    );

    // Important 3: the package is genuinely gone from disk, so this must not
    // report exit 2 -- "refused, nothing changed" -- which would tell an
    // operator there is nothing to look at when there is exactly one thing
    // to look at.
    assert_eq!(
        ex.exit_code(false),
        1,
        "the machine changed even though nothing is Done: 2 would say otherwise"
    );
    let rendered = dotpkg::render::render_execution(&ex);
    assert!(
        rendered.contains("Some packages were changed and some were not"),
        "the summary must not call this 'nothing changed' either: {rendered}"
    );
}

// -- Important 1: a fresh Install that leaves residue must also be touched --
//
// The `Replace` shape (uninstall really ran, install then lied) was already
// covered by the test above. A fresh `Install` has no uninstall half at all,
// so before this fix `touched` stayed false no matter what the install half
// left behind -- reporting `Failed { touched: false }` for a package that is
// now genuinely, if imperfectly, on disk.

#[test]
fn a_fresh_install_that_leaves_half_install_residue_is_touched() {
    // Demonstrated live with scoop's measured half-install residue: a hash
    // mismatch leaves `apps/<app>/<version>/` with no `current/manifest.json`
    // and still exits 0. Before this fix: `changed()=0 failed()=1 touched()=0`,
    // `exit_code(false) = 2`, and the FAILED line right above the summary
    // directly contradicted "Nothing was changed".
    struct HalfInstaller<'a> {
        tree: &'a Tree,
    }
    impl Mutator for HalfInstaller<'_> {
        fn uninstall(&self, _app: &Name) -> anyhow::Result<CommandReport> {
            unreachable!("a plain Install has no uninstall half")
        }
        fn install(&self, manifest: &Path, _arch: Option<&str>) -> anyhow::Result<CommandReport> {
            let app = manifest.file_stem().unwrap().to_string_lossy().into_owned();
            let version = manifest
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            self.tree.half_install(&app, &version);
            Ok(CommandReport {
                code: Some(0),
                stdout: "Checking hash of fzf.zip ... ERROR Hash check failed!\n".into(),
                stderr: String::new(),
            })
        }
    }
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    let fake = HalfInstaller { tree: &t };
    let mut state = State::default();

    let ex = execute(
        t.root(),
        vec![Step::Install {
            app: Name::new("fzf"),
            staged,
            arch: None,
        }],
        &fake,
        &mut state,
        &|| Running::default(),
        &ExecOptions::default(),
    )
    .unwrap();

    assert_eq!(ex.failed(), 1, "{:?}", ex.results);
    assert!(
        matches!(&ex.results[0].1, ItemResult::Failed { touched: true, .. }),
        "the half-install really left residue on disk: {:?}",
        ex.results
    );
    assert_eq!(
        ex.exit_code(false),
        1,
        "residue on disk means something to look at, not exit 2's nothing-changed"
    );
    let rendered = dotpkg::render::render_execution(&ex);
    assert!(
        !rendered.contains("Nothing was changed"),
        "the summary must not contradict the FAILED line above it: {rendered}"
    );
}

#[test]
fn a_fresh_install_of_a_different_manifest_than_staged_is_touched_and_not_owned() {
    // The worse of the two shapes Important 1 names: scoop installed A
    // manifest, just not the one dotpkg staged and hash-verified. The
    // package really is on the machine now -- installed with an unverified
    // artifact -- and before this fix it reported `Failed { touched: false }`,
    // so dotpkg would neither flag it for the operator nor ever record it.
    struct WrongManifestInstaller<'a> {
        tree: &'a Tree,
    }
    impl Mutator for WrongManifestInstaller<'_> {
        fn uninstall(&self, _app: &Name) -> anyhow::Result<CommandReport> {
            unreachable!("a plain Install has no uninstall half")
        }
        fn install(&self, manifest: &Path, _arch: Option<&str>) -> anyhow::Result<CommandReport> {
            let app = manifest.file_stem().unwrap().to_string_lossy().into_owned();
            let cur = self.tree.root().join("apps").join(app).join("current");
            std::fs::create_dir_all(&cur).unwrap();
            // Deliberately NOT `std::fs::read(manifest)`: a different
            // manifest is exactly what scoop, measured, can install over a
            // rev-locked bucket tip (see `verify.rs`'s own content-swap
            // test).
            std::fs::write(
                cur.join("manifest.json"),
                r#"{"version":"1.0.0","url":"https://evil/v1.zip"}"#,
            )
            .unwrap();
            Ok(CommandReport {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    let fake = WrongManifestInstaller { tree: &t };
    let mut state = State::default();

    let ex = execute(
        t.root(),
        vec![Step::Install {
            app: Name::new("fzf"),
            staged,
            arch: None,
        }],
        &fake,
        &mut state,
        &|| Running::default(),
        &ExecOptions::default(),
    )
    .unwrap();

    assert_eq!(ex.failed(), 1, "{:?}", ex.results);
    assert!(
        matches!(&ex.results[0].1, ItemResult::Failed { touched: true, .. }),
        "a different manifest was really installed: {:?}",
        ex.results
    );
    assert_eq!(ex.exit_code(false), 1);
    let rendered = dotpkg::render::render_execution(&ex);
    assert!(!rendered.contains("Nothing was changed"), "{rendered}");
    assert!(
        !state.owns(SCOOP, &Name::new("fzf")),
        "a failed install must not be recorded as owned, even though something landed on disk"
    );
}

// -- Important 5: the resolved architecture must reach Mutator::install ----
//
// `verdict` structurally cannot catch a wrong-architecture install:
// architecture is not in manifest.json, so the bytes match and `verdict`
// returns `Ok` regardless. `install.json` is the only record, and nothing in
// this suite asserted the `arch` argument that reaches `Mutator::install` at
// all before this test -- `m.install(staged, arch.as_deref())` and
// `m.install(staged, None)` were equally green.

#[test]
fn the_resolved_architecture_reaches_mutator_install() {
    struct ArchSpy<'a> {
        tree: &'a Tree,
        seen: RefCell<Vec<Option<String>>>,
    }
    impl Mutator for ArchSpy<'_> {
        fn uninstall(&self, app: &Name) -> anyhow::Result<CommandReport> {
            let _ = std::fs::remove_dir_all(self.tree.root().join("apps").join(app.key()));
            Ok(CommandReport {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
        fn install(&self, manifest: &Path, arch: Option<&str>) -> anyhow::Result<CommandReport> {
            self.seen.borrow_mut().push(arch.map(str::to_string));
            let app = manifest.file_stem().unwrap().to_string_lossy().into_owned();
            let cur = self.tree.root().join("apps").join(app).join("current");
            std::fs::create_dir_all(&cur).unwrap();
            std::fs::write(cur.join("manifest.json"), std::fs::read(manifest).unwrap()).unwrap();
            Ok(CommandReport {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    let fake = ArchSpy {
        tree: &t,
        seen: RefCell::new(Vec::new()),
    };
    let mut state = State::default();

    let out = run_step(
        t.root(),
        &fake,
        &mut state,
        &Step::Install {
            app: Name::new("fzf"),
            staged,
            arch: Some("arm64".into()),
        },
    );

    assert_eq!(out, StepOutcome::Done, "{out:?}");
    assert_eq!(
        fake.seen.borrow().as_slice(),
        [Some("arm64".to_string())],
        "the resolved architecture must reach Mutator::install"
    );
}

#[test]
fn a_root_with_no_apps_directory_does_not_look_like_scoop() {
    let t = Tree::new();
    // Deliberately no `t.empty_apps()`: a fresh tempdir has no `apps` at all,
    // which is exactly the wrong-or-typo'd-`$SCOOP` shape this guards.
    let err = root_looks_like_scoop(t.root()).unwrap_err();
    assert!(err.contains("apps directory"), "{err}");
    assert!(err.contains(&t.root().display().to_string()), "{err}");
}

#[test]
fn a_root_with_an_apps_directory_looks_like_scoop() {
    // Positive control for the test above: without it, a function that
    // always returns `Err` would pass it too.
    let t = Tree::new();
    t.empty_apps();
    root_looks_like_scoop(t.root()).unwrap();
}

// -- Critical 1: the recovery file exists BEFORE execute's first mutation --
//
// `write_recovery` alone (tested above) proves content and that removals are
// excluded from it -- nothing about ORDER, and nothing about `execute` ever
// calling it at all. Replacing the whole recovery block in `execute` with
// `let _ = &opts.recovery_path;` left every other test green.

#[test]
fn the_recovery_file_exists_on_disk_before_executes_first_mutation() {
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    let recovery_path = t.root().join("recover.cmd");

    // A mutator whose every call asserts the recovery file is already
    // there. If `execute` writes it after the fact, or not at all, the
    // very first call -- `install`, since there is nothing to uninstall for
    // a plain `Install` -- panics instead of the assertion below firing.
    struct AssertRecoveryAlreadyExists<'a> {
        tree: &'a Tree,
        recovery_path: PathBuf,
    }
    impl Mutator for AssertRecoveryAlreadyExists<'_> {
        fn uninstall(&self, app: &Name) -> anyhow::Result<CommandReport> {
            assert!(
                self.recovery_path.exists(),
                "the recovery file must exist before ANY mutation, including uninstall"
            );
            let _ = std::fs::remove_dir_all(self.tree.root().join("apps").join(app.key()));
            Ok(CommandReport {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
        fn install(&self, manifest: &Path, _arch: Option<&str>) -> anyhow::Result<CommandReport> {
            assert!(
                self.recovery_path.exists(),
                "the recovery file must exist before the FIRST mutation"
            );
            let app = manifest.file_stem().unwrap().to_string_lossy().into_owned();
            let cur = self.tree.root().join("apps").join(&app).join("current");
            std::fs::create_dir_all(&cur).unwrap();
            std::fs::write(cur.join("manifest.json"), std::fs::read(manifest).unwrap()).unwrap();
            Ok(CommandReport {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }
    let fake = AssertRecoveryAlreadyExists {
        tree: &t,
        recovery_path: recovery_path.clone(),
    };
    let mut state = State::default();

    let ex = execute(
        t.root(),
        vec![Step::Install {
            app: Name::new("fzf"),
            staged,
            arch: None,
        }],
        &fake,
        &mut state,
        &|| Running::default(),
        &ExecOptions {
            recovery_path: Some(recovery_path.clone()),
        },
    )
    .unwrap();

    assert_eq!(ex.changed(), 1, "{:?}", ex.results);
    assert!(recovery_path.exists());
}

// -- Critical 2: root_looks_like_scoop must actually be called -------------

#[test]
fn execute_refuses_a_root_that_does_not_look_like_scoop_before_calling_the_mutator_even_once() {
    let t = Tree::new();
    // Deliberately no `t.empty_apps()`: this is the wrong-or-typo'd-`$SCOOP`
    // shape `root_looks_like_scoop` exists to catch.
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    let fake = Fake::honest(&t);
    let mut state = State::default();

    let err = execute(
        t.root(),
        vec![Step::Install {
            app: Name::new("fzf"),
            staged,
            arch: None,
        }],
        &fake,
        &mut state,
        &|| Running::default(),
        &ExecOptions::default(),
    )
    .unwrap_err();

    assert!(err.contains("apps directory"), "{err}");
    assert_eq!(
        fake.calls(),
        Vec::<String>::new(),
        "the mutator must never be called against a root that does not look like scoop"
    );
    assert_eq!(state.owned_count(SCOOP), 0);
}

// -- Important 6: a recovery-write failure is recorded, not just printed ---

#[test]
fn a_recovery_file_that_cannot_be_written_is_recorded_but_does_not_stop_the_run() {
    let t = Tree::new();
    let staged = t.stage("fzf", "1.0.0", BODY_A);
    t.empty_apps();
    // `path.parent()` is a FILE, not a directory: `create_dir_all` over it
    // fails, so `write_recovery` fails for real rather than by mocking.
    let blocker = t.root().join("blocker");
    std::fs::write(&blocker, b"not a directory").unwrap();
    let recovery_path = blocker.join("recover.cmd");

    let fake = Fake::honest(&t);
    let mut state = State::default();

    let ex = execute(
        t.root(),
        vec![Step::Install {
            app: Name::new("fzf"),
            staged,
            arch: None,
        }],
        &fake,
        &mut state,
        &|| Running::default(),
        &ExecOptions {
            recovery_path: Some(recovery_path),
        },
    )
    .unwrap();

    assert!(
        ex.recovery_write_failed.is_some(),
        "the failure must be recorded, not just printed: {:?}",
        ex.recovery_write_failed
    );
    assert_eq!(
        ex.changed(),
        1,
        "a missing recovery file must not stop the run -- that decision belongs to the caller: {:?}",
        ex.results
    );
}

// -- Minor 8: a literal `%` in a staged path survives the recovery file ----

#[test]
fn a_percent_in_a_staged_path_is_escaped_and_the_path_stays_quoted() {
    // `%` is expanded by cmd even *inside* double quotes, so an unescaped
    // one turns the recovery line into a reference to an undefined batch
    // variable instead of the staged manifest. The previous test's
    // assertions were bare `contains` checks that pass whether or not the
    // path is quoted at all, so quoting itself was unasserted.
    let t = Tree::new();
    let staged = t
        .root()
        .join("stage")
        .join("weird app%name")
        .join("1.0.0")
        .join("weird.json");
    let out = t.root().join("recover.cmd");

    write_recovery(
        &out,
        &[Step::Install {
            app: Name::new("weird"),
            staged: staged.clone(),
            arch: None,
        }],
    )
    .unwrap();

    let text = std::fs::read_to_string(&out).unwrap();
    let escaped = staged.display().to_string().replace('%', "%%");
    let quoted = format!("\"{escaped}\"");
    assert!(
        text.contains(&quoted),
        "expected an escaped, quoted path: {text}"
    );
    assert!(
        !text.contains(&format!("\"{}\"", staged.display())),
        "the raw % must not survive unescaped inside quotes: {text}"
    );
}
