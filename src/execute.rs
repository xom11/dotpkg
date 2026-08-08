//! The executor: the only part of dotpkg that changes installed software.
//!
//! One seam is faked in tests — `Mutator`, the scoop subprocess. Everything
//! else, including every observation of the result, runs against a real
//! directory tree, because a fake that both performs and reports the mutation
//! proves only that it is self-consistent.

use crate::model::{Name, SCOOP};
use crate::state::{Ownership, State};
use crate::verify::{verdict, Disagreement, Expected};
use anyhow::Result;
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
