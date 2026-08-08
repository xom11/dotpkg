//! The executor: the only part of dotpkg that changes installed software.
//!
//! One seam is faked in tests — `Mutator`, the scoop subprocess. Everything
//! else, including every observation of the result, runs against a real
//! directory tree, because a fake that both performs and reports the mutation
//! proves only that it is self-consistent.

use crate::model::{Name, Running, SCOOP};
use crate::state::{Ownership, State};
use crate::verify::{verdict, Disagreement, Expected};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// What one scoop invocation said.
///
/// `code` is recorded and never believed. Measured on a14: scoop exits 0 for a
/// hash mismatch, a dead URL, an install over a nonexistent manifest path, and
/// an uninstall of an app that is not installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandReport {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Every scoop invocation that changes installed software.
///
/// `Err` means the process could not be run at all. It does **not** mean the
/// operation failed — that is `verify::verdict`'s answer, and only its answer.
pub trait Mutator {
    fn uninstall(&self, app: &Name) -> Result<CommandReport>;
    fn install(&self, manifest: &Path, arch: Option<&str>) -> Result<CommandReport>;
}

/// One mutation, already resolved against the plan and the preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Nothing is installed: no window opens at all.
    Install {
        app: Name,
        staged: PathBuf,
        arch: Option<String>,
    },
    /// A version change, which scoop can only do as uninstall + install.
    Replace {
        app: Name,
        staged: PathBuf,
        arch: Option<String>,
    },
    Remove {
        app: Name,
    },
}

impl Step {
    pub fn app(&self) -> &Name {
        match self {
            Step::Install { app, .. } | Step::Replace { app, .. } | Step::Remove { app } => app,
        }
    }
}

/// Packages held back to the end of their group.
///
/// `git` is the binary `Scoop::stage` shells out to, and on the dogfood machine
/// it is itself scoop-managed (`where.exe git` resolves into
/// `scoop\apps\git\current`). The extraction helpers are what scoop uses to
/// unpack everything else.
pub const DEFER_LAST: &[&str] = &["git", "7zip", "dark", "innounp", "lessmsi"];

/// Installs, then replacements, then removals; `DEFER_LAST` at the end of each
/// group; alphabetical within that, so a run is reproducible.
pub fn order(mut steps: Vec<Step>) -> Vec<Step> {
    steps.sort_by_key(|s| {
        let group = match s {
            Step::Install { .. } => 0u8,
            Step::Replace { .. } => 1,
            Step::Remove { .. } => 2,
        };
        let deferred = u8::from(DEFER_LAST.contains(&s.app().key()));
        (group, deferred, s.app().key().to_string())
    });
    steps
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Done,
    Failed(String),
}

/// Perform one step and prove on disk that it happened.
///
/// State is written only after the disk agrees, and only when the answer
/// changes: an upgrade of a package dotpkg already owns writes nothing,
/// because ownership is intent and the uninstall half is an implementation
/// detail. A crash mid-window leaves the package absent and still declared,
/// and the next run's plan re-emits an `Install`.
pub fn run_step(root: &Path, m: &dyn Mutator, state: &mut State, step: &Step) -> StepOutcome {
    match step {
        Step::Install { app, staged, arch } | Step::Replace { app, staged, arch } => {
            if matches!(step, Step::Replace { .. }) {
                if let Err(e) = m.uninstall(app) {
                    return StepOutcome::Failed(format!("{app}: could not run uninstall: {e:#}"));
                }
                if let Err(d) = verdict(root, app, &Expected::Absent) {
                    return StepOutcome::Failed(format!("{app}: uninstall did not happen -- {d}"));
                }
            }
            if let Err(e) = m.install(staged, arch.as_deref()) {
                return StepOutcome::Failed(format!("{app}: could not run install: {e:#}"));
            }
            let want = Expected::Present {
                staged: staged.clone(),
            };
            if let Err(d) = verdict(root, app, &want) {
                // Retry exactly once, and only when there is nothing there at
                // all. A retry over a half-install gets `WARN ... is already
                // installed`, exit 0, and no change -- which would then pass
                // no check dotpkg has.
                if d != Disagreement::NotInstalled {
                    return StepOutcome::Failed(format!("{app}: install did not happen -- {d}"));
                }
                if let Err(e) = m.install(staged, arch.as_deref()) {
                    return StepOutcome::Failed(format!("{app}: could not run retry: {e:#}"));
                }
                if let Err(d2) = verdict(root, app, &want) {
                    return StepOutcome::Failed(format!(
                        "{app}: install did not happen, even on retry -- {d2}"
                    ));
                }
            }
            // Claim only now, and preserve an existing `adopt`.
            if state.ownership(SCOOP, app).is_none() {
                state.set(SCOOP, app, Ownership::Installed);
            }
            StepOutcome::Done
        }
        Step::Remove { app } => {
            if let Err(e) = m.uninstall(app) {
                return StepOutcome::Failed(format!("{app}: could not run uninstall: {e:#}"));
            }
            if let Err(d) = verdict(root, app, &Expected::Absent) {
                return StepOutcome::Failed(format!("{app}: uninstall did not happen -- {d}"));
            }
            state.remove(SCOOP, app);
            StepOutcome::Done
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct ExecOptions {
    /// Where to write the recovery script before the first mutation.
    pub recovery_path: Option<PathBuf>,
}

// `keep_going` is deliberately NOT a field here. It decides which steps get
// built, and `main.rs` has already applied it by the time `execute` is
// called -- carrying it in would be a flag the function receives and never
// reads, which is both dead and misleading about where the decision lives.
// If a later change needs `execute` itself to branch on it, add it then,
// with the branch.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemResult {
    Done,
    Failed(String),
    /// Not attempted, and the run is not at fault: the package started
    /// running, or removals are gated off.
    Held(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Execution {
    pub results: Vec<(Name, ItemResult)>,
    pub dropped_ghosts: Vec<Name>,
}

impl Execution {
    pub fn changed(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, r)| *r == ItemResult::Done)
            .count()
    }
    pub fn failed(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, r)| matches!(r, ItemResult::Failed(_)))
            .count()
    }
    pub fn held(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, r)| matches!(r, ItemResult::Held(_)))
            .count()
    }
    /// 0 everything verified · 1 something changed and something failed ·
    /// 2 refused, nothing changed.
    ///
    /// The distinction 2 buys is the one a caller most needs: "go look at the
    /// machine" versus "nothing to look at".
    pub fn exit_code(&self, refused: bool) -> i32 {
        if refused {
            return 2;
        }
        if self.failed() > 0 {
            if self.changed() == 0 {
                return 2;
            }
            return 1;
        }
        0
    }
}

/// One `scoop install` line per artifact in the run, written **before** the
/// first mutation.
///
/// A run that dies leaves a file that puts the machine back. A run that only
/// prints advice leaves nothing once the terminal is gone -- and the terminal
/// is exactly what a broken `git` or a broken shell takes with it.
///
/// Removals never appear: this file only ever puts software back.
pub fn write_recovery(path: &Path, steps: &[Step]) -> Result<()> {
    use std::fmt::Write as _;
    let mut text = String::from(
        "@echo off\r\nREM Written by dotpkg before it changed anything.\r\n\
         REM Each line reinstalls one package from the manifest dotpkg staged\r\n\
         REM and hash-verified. Safe to run more than once.\r\n",
    );
    for s in steps {
        let (staged, arch) = match s {
            Step::Install { staged, arch, .. } | Step::Replace { staged, arch, .. } => {
                (staged, arch)
            }
            Step::Remove { .. } => continue,
        };
        let a = match arch {
            Some(a) => format!("-a {a} "),
            None => String::new(),
        };
        let _ = writeln!(text, "scoop install -u {a}\"{}\"\r", staged.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
}

/// Refuse to run at all against a root that does not look like a scoop
/// install.
///
/// Found in Task 5's review. `verify::verdict` maps "no `apps/` directory" to
/// absent, and `Expected::Absent` maps absent to `Ok(())` -- so a wrong or
/// typo'd `$SCOOP` **verifies every uninstall as successful**. Installs are
/// safe in the same state (they come back `NotInstalled`, an error); it is
/// only the destructive direction that silently passes, and it is exactly the
/// direction where scoop also exits 0. One check, once, before any of it.
pub fn root_looks_like_scoop(root: &Path) -> Result<(), String> {
    if root.join("apps").is_dir() {
        return Ok(());
    }
    Err(format!(
        "{} has no apps directory, so it is not a scoop root. Refusing to run: \
         every uninstall would verify as successful against an empty tree. \
         Check $SCOOP.",
        root.display()
    ))
}

/// Run every step, in order, verifying each. One package's failure never
/// stops another's.
///
/// Callers must have passed `root_looks_like_scoop` first; `main.rs` does.
pub fn execute(
    root: &Path,
    steps: Vec<Step>,
    m: &dyn Mutator,
    state: &mut State,
    running: &Running,
    opts: &ExecOptions,
) -> Execution {
    let steps = order(steps);
    let mut ex = Execution::default();

    if let Some(p) = &opts.recovery_path {
        if let Err(e) = write_recovery(p, &steps) {
            eprintln!("warning: could not write the recovery script: {e:#}");
        }
    }

    for step in &steps {
        let app = step.app().clone();
        // Re-checked here, not only at plan time: `running` was sampled before
        // the prefetch, which takes minutes.
        if running.covers_name(&app) {
            ex.results.push((
                app,
                ItemResult::Held(
                    "started running since the plan was made -- stop it and run again".into(),
                ),
            ));
            continue;
        }
        let r = match run_step(root, m, state, step) {
            StepOutcome::Done => ItemResult::Done,
            StepOutcome::Failed(why) => ItemResult::Failed(why),
        };
        ex.results.push((app, r));
    }
    ex
}
