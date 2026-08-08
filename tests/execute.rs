//! The executor against real directory trees and a fake that lies exactly the
//! way scoop was measured to lie.

use dotpkg::execute::*;
use dotpkg::model::{Name, SCOOP};
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
