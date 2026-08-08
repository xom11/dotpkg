//! The executor against real directory trees and a fake that lies exactly the
//! way scoop was measured to lie.

use dotpkg::execute::*;
use dotpkg::model::{Name, Running, SCOOP};
use dotpkg::state::{Ownership, State};
use std::cell::RefCell;
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

    let StepOutcome::Failed(why) = out else {
        panic!("a silent no-op must be a failure, got {out:?}")
    };
    assert!(why.contains("install did not happen"), "{why}");
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

    assert!(matches!(out, StepOutcome::Failed(_)), "{out:?}");
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

    let StepOutcome::Failed(why) = out else {
        panic!("got {out:?}")
    };
    assert!(why.contains("uninstall did not happen"), "{why}");
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
    assert!(matches!(out, StepOutcome::Failed(_)), "{out:?}");
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
        &Running::default(),
        &ExecOptions::default(),
    );

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
        &Running::default(),
        &ExecOptions::default(),
    );

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
    // `running` is sampled once, before roughly two dozen downloads. A user
    // who opens their editor during the prefetch must not have it uninstalled.
    let t = Tree::new();
    t.install("nvim-ish", BODY_A);
    let fake = Fake::honest(&t);
    let mut state = State::default();
    state.set(SCOOP, &Name::new("nvim-ish"), Ownership::Installed);
    let running = Running::new(
        std::collections::BTreeSet::from(["nvim-ish".to_string()]),
        Default::default(),
    );

    let ex = execute(
        t.root(),
        vec![Step::Remove {
            app: Name::new("nvim-ish"),
        }],
        &fake,
        &mut state,
        &running,
        &ExecOptions::default(),
    );

    assert_eq!(ex.held(), 1, "{:?}", ex.results);
    assert_eq!(
        fake.calls(),
        Vec::<String>::new(),
        "nothing may be run for it"
    );
    assert!(
        state.owns(SCOOP, &Name::new("nvim-ish")),
        "and it stays owned"
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
            (Name::new("b"), ItemResult::Failed("no".into())),
        ],
        dropped_ghosts: Vec::new(),
    };
    assert_eq!(
        mixed.exit_code(false),
        1,
        "something changed and something failed"
    );
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
        &Running::default(),
        &ExecOptions::default(),
    );

    assert_eq!(ex.failed(), 1, "{:?}", ex.results);
    assert!(
        matches!(&ex.results[0].1, ItemResult::Failed(_)),
        "{:?}",
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
