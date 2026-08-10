//! The executor: the only part of dotpkg that changes installed software.
//!
//! One seam is faked in tests — `Mutator`, the scoop subprocess. Everything
//! else, including every observation of the result, runs against a real
//! directory tree, because a fake that both performs and reports the mutation
//! proves only that it is self-consistent.

use crate::backend::winget_exec::WingetMutator;
use crate::config::BucketDecl;
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

/// Every scoop invocation this crate makes.
///
/// It was "every invocation that changes installed software" until `download`
/// joined it: that one changes nothing, but it is still a scoop process, and
/// leaving it outside the seam meant no test on any platform could reach the
/// code that builds its argv. The seam is about which calls are OBSERVABLE in
/// a test, not about which of them mutate.
///
/// `Err` means the process could not be run at all. It does **not** mean the
/// operation failed — that is `verify::verdict`'s answer, and only its answer.
pub trait Mutator {
    fn uninstall(&self, app: &Name) -> Result<CommandReport>;
    fn install(&self, manifest: &Path, arch: Option<&str>) -> Result<CommandReport>;
    /// Fetch and hash-verify. Not a mutation of installed software, but it is
    /// the third scoop invocation and it belongs behind the same seam: until
    /// it was here, no test on any platform could produce an
    /// `Outcome::ReadyToFetch` from production code, or see the argv that
    /// carries the resolved architecture.
    fn download(&self, manifest: &Path, arch: Option<&str>) -> Result<CommandReport>;
    /// Add a declared bucket that is missing on disk. Not a mutation of
    /// installed software either, but it is the fourth scoop invocation, and
    /// `clone_missing_buckets` calling a `Scoop`-only `run` directly bypassed
    /// this seam the same way `download` used to: until this joined the
    /// trait, no test on any platform could make a bucket add report success
    /// while having cloned nothing -- the exact silent-success shape
    /// `clone_missing_buckets`'s own doc comment measures `scoop bucket add`
    /// for, and the one its post-run `.git` check exists to catch.
    fn bucket_add(&self, bucket: &BucketDecl) -> Result<CommandReport>;
}

/// One mutation, already resolved against the plan and the preparation --
/// split by backend so a winget action cannot reach scoop's executor (or the
/// reverse) by construction.
///
/// Before this split, `Step` named only `app`, `staged` and `arch`, and
/// `plan_to_steps` matched `Action::Install { name, arch, .. }` while
/// ignoring `backend` entirely. The only thing that kept a winget action out
/// of scoop's executor was `stage_and_fetch`'s `backend != SCOOP` check at
/// the *staging* layer -- the wrong layer for the guard, since it says
/// nothing about `execute` itself. Splitting the type makes the mistake
/// unwritable, the same move this crate already makes with `Resolution`
/// carrying a `Pin` and with `is_outstanding`'s wildcard-free match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Scoop(ScoopStep),
    Winget(WingetStep),
}

/// Every scoop mutation `execute` can be asked to perform. Named `ScoopStep`
/// rather than `Step` now that a winget sibling exists; its three variants
/// and their fields are otherwise unchanged from before the split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoopStep {
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

impl ScoopStep {
    pub fn app(&self) -> &Name {
        match self {
            ScoopStep::Install { app, .. }
            | ScoopStep::Replace { app, .. }
            | ScoopStep::Remove { app } => app,
        }
    }
}

/// Every winget mutation `execute` can be asked to perform.
///
/// **No `Replace`, and that is the point.** `ScoopStep::Replace` exists
/// because scoop *cannot* change a version any other way -- `install` over
/// an installed app is a measured no-op -- so it needs an uninstall half and
/// therefore a window where the package is absent. Measured, winget's
/// `install --version <pin>` performs the upgrade directly (0.24.1 ->
/// 0.26.1, exit 0), so a winget version change opens **no such window**, and
/// `run_step`'s `touched` bookkeeping has no uninstall half to reason about.
/// One call, either direction, covers both a fresh install and a version
/// change -- hence `Set`, not `Install`/`Replace`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WingetStep {
    /// Install OR version-change: one `install --version` call either way.
    Set {
        id: Name,
        version: String,
        guard: Vec<String>,
    },
    Remove {
        id: Name,
        version: String,
        guard: Vec<String>,
    },
}

impl WingetStep {
    /// Mirrors `ScoopStep::app()`: the id out of a `WingetStep`, regardless
    /// of which variant. An inherent method rather than a private free
    /// function -- the asymmetry with `ScoopStep::app()` had no reason
    /// behind it, and a later task builds argv from a `WingetStep` it holds
    /// directly, so it wants this the same way `run_scoop_step` wants
    /// `ScoopStep::app()`.
    pub fn app(&self) -> &Name {
        match self {
            WingetStep::Set { id, .. } | WingetStep::Remove { id, .. } => id,
        }
    }
}

impl Step {
    pub fn app(&self) -> &Name {
        match self {
            Step::Scoop(s) => s.app(),
            Step::Winget(w) => w.app(),
        }
    }
    /// Which package manager will perform this step.
    ///
    /// Read off the variant, so it cannot disagree with what actually runs --
    /// unlike a backend name carried alongside a step, which could. `execute`
    /// records it on every `ItemOutcome` so the closing table names the right
    /// backend; see `ItemOutcome`'s own doc comment for what it used to print
    /// instead.
    pub fn backend(&self) -> &'static str {
        match self {
            Step::Scoop(_) => crate::model::SCOOP,
            Step::Winget(_) => crate::model::WINGET,
        }
    }
    /// Names a live process might report for this step's package, for
    /// `execute`'s per-step re-sampler. Empty for scoop, whose packages are
    /// already reachable through `Running`'s `dirs` half and whose `bins` the
    /// planner consulted at plan time.
    pub fn guard_names(&self) -> &[String] {
        match self {
            Step::Scoop(_) => &[],
            Step::Winget(WingetStep::Set { guard, .. } | WingetStep::Remove { guard, .. }) => guard,
        }
    }
    pub fn is_remove(&self) -> bool {
        matches!(
            self,
            Step::Scoop(ScoopStep::Remove { .. }) | Step::Winget(WingetStep::Remove { .. })
        )
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
///
/// `WingetStep::Set` groups with installs, not replacements: it is one call
/// and opens no absent-window, so it carries none of the reason `Replace`
/// sorts after `Install`.
pub fn order(mut steps: Vec<Step>) -> Vec<Step> {
    steps.sort_by_key(|s| {
        let group = match s {
            Step::Scoop(ScoopStep::Install { .. }) | Step::Winget(WingetStep::Set { .. }) => 0u8,
            Step::Scoop(ScoopStep::Replace { .. }) => 1,
            Step::Scoop(ScoopStep::Remove { .. }) | Step::Winget(WingetStep::Remove { .. }) => 2,
        };
        // DEFER_LAST is scoop-only by construction: it holds back `git` and
        // the extraction helpers because `Scoop::stage` shells out to git and
        // scoop unpacks with 7zip/dark/innounp/lessmsi. Nothing in the
        // winget path shells out to any of them -- winget downloads and
        // extracts inside its own process -- so a winget id whose last
        // segment happens to be "git" must not be deferred for a reason that
        // does not apply to it.
        let deferred = match s {
            Step::Scoop(_) => u8::from(DEFER_LAST.contains(&s.app().key())),
            Step::Winget(_) => 0,
        };
        (group, deferred, s.app().key().to_string())
    });
    steps
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Done,
    /// `touched` is true when the machine was already altered before this
    /// failure happened. Two shapes set it: a `ScoopStep::Replace` whose
    /// uninstall verified `Absent` before its install then failed, and any
    /// install (fresh, or the second half of a replace) whose `verdict`
    /// disagreement is evidence of residue -- `HalfInstalled`,
    /// `ContentDiffers`, or `Unreadable` (which means
    /// "unknown" and is treated as touched in the safe direction, so an
    /// operator looks). `NotInstalled` alone means the machine is genuinely
    /// as it was. A `ScoopStep::Remove` sets it the mirror way: `StillPresent`
    /// is untouched, `Unreadable` is touched. Either way, the package this
    /// describes is neither "done" nor "as it was", and
    /// `Execution::exit_code` must not fold that into "nothing changed".
    Failed {
        why: String,
        touched: bool,
    },
}

/// Perform one scoop step and prove on disk that it happened.
///
/// State is written only after the disk agrees, and only when the answer
/// changes: an upgrade of a package dotpkg already owns writes nothing,
/// because ownership is intent and the uninstall half is an implementation
/// detail. A crash mid-window leaves the package absent and still declared,
/// and the next run's plan re-emits an `Install`.
fn run_scoop_step(
    root: &Path,
    m: &dyn Mutator,
    state: &mut State,
    step: &ScoopStep,
) -> StepOutcome {
    match step {
        ScoopStep::Install { app, staged, arch } | ScoopStep::Replace { app, staged, arch } => {
            // Starts false and only ever moves to true, never back: two
            // independent checks below can set it, and both mean the same
            // thing -- scoop wrote SOMETHING to disk before this step gave
            // up.
            let mut touched = false;
            if matches!(step, ScoopStep::Replace { .. }) {
                if let Err(e) = m.uninstall(app) {
                    return StepOutcome::Failed {
                        why: format!("{app}: could not run uninstall: {e:#}"),
                        touched,
                    };
                }
                // Set true only once the uninstall half is PROVEN by
                // `verdict`, not merely attempted: a failure at or before
                // this point leaves the machine exactly as it was, and a
                // failure after it leaves the package genuinely gone. Same
                // assumption as `ScoopStep::Remove`'s `StillPresent` reasoning
                // below: a failed uninstall here is read as having removed
                // nothing, which holds only if scoop's uninstall is
                // all-or-nothing -- unmeasured, and wrong if it is partial.
                if let Err(d) = verdict(root, app, &Expected::Absent) {
                    return StepOutcome::Failed {
                        why: format!("{app}: uninstall did not happen -- {d}"),
                        touched,
                    };
                }
                touched = true;
            }
            if let Err(e) = m.install(staged, arch.as_deref()) {
                return StepOutcome::Failed {
                    why: format!("{app}: could not run install: {e:#}"),
                    touched,
                };
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
                    // Important 1: every other `Disagreement` reachable here
                    // -- `HalfInstalled`, `ContentDiffers` -- means scoop
                    // wrote something to
                    // disk, even for a plain `Install` with no uninstall
                    // half of its own: a hash-mismatched half-install
                    // leaves `apps/<app>/<version>/`, and a different
                    // manifest actually installed leaves a package on the
                    // machine dotpkg never asked for. `NotInstalled` is the
                    // only variant that means the machine is genuinely as
                    // it was, which is why it alone is excluded here.
                    // `Unreadable` also reaches this arm (it is not
                    // `NotInstalled`) and means dotpkg does not know --
                    // treated as touched in the safe direction, so an
                    // operator looks instead of being told there is
                    // nothing to see.
                    touched = true;
                    return StepOutcome::Failed {
                        why: format!("{app}: install did not happen -- {d}"),
                        touched,
                    };
                }
                if let Err(e) = m.install(staged, arch.as_deref()) {
                    return StepOutcome::Failed {
                        why: format!("{app}: could not run retry: {e:#}"),
                        touched,
                    };
                }
                if let Err(d2) = verdict(root, app, &want) {
                    // Same reasoning as the first check above: only a
                    // second `NotInstalled` still means untouched.
                    if d2 != Disagreement::NotInstalled {
                        touched = true;
                    }
                    return StepOutcome::Failed {
                        why: format!("{app}: install did not happen, even on retry -- {d2}"),
                        touched,
                    };
                }
            }
            // Claim only now, and preserve an existing `adopt`.
            if state.ownership(SCOOP, app).is_none() {
                state.set(SCOOP, app, Ownership::Installed);
            }
            StepOutcome::Done
        }
        ScoopStep::Remove { app } => {
            if let Err(e) = m.uninstall(app) {
                return StepOutcome::Failed {
                    why: format!("{app}: could not run uninstall: {e:#}"),
                    touched: false,
                };
            }
            if let Err(d) = verdict(root, app, &Expected::Absent) {
                // `StillPresent` means the uninstall simply did not happen:
                // the app is exactly where it was, so the machine is
                // untouched. `Unreadable` means `verdict` could not look at
                // all -- unknown, and the safe reading is "assume touched"
                // so an operator looks rather than being told nothing
                // happened. `Expected::Absent` cannot produce any other
                // `Disagreement` variant (see `verdict`'s match arm).
                //
                // The `StillPresent` half of this assumes scoop's uninstall
                // is all-or-nothing -- it either removes the whole package or
                // fails and removes nothing -- which is unmeasured. A scoop
                // that can uninstall partially would leave real residue
                // behind a `StillPresent` verdict, and this call would then
                // be wrong to report untouched.
                let touched = matches!(d, Disagreement::Unreadable(_));
                return StepOutcome::Failed {
                    why: format!("{app}: uninstall did not happen -- {d}"),
                    touched,
                };
            }
            state.remove(SCOOP, app);
            StepOutcome::Done
        }
    }
}

/// Perform one step -- scoop or winget -- and prove it happened.
///
/// Dispatches on backend and nothing else: a winget `Step` cannot reach
/// `run_scoop_step`, and a scoop `Step` cannot reach the winget arm, because
/// the match is over `Step` itself, not over some flag carried alongside it.
pub fn run_step(
    root: &Path,
    m: &dyn Mutator,
    wm: &dyn WingetMutator,
    state: &mut State,
    step: &Step,
) -> StepOutcome {
    match step {
        Step::Scoop(s) => run_scoop_step(root, m, state, s),
        // A later task replaces this. A `todo!()` would panic in a release
        // build on a real machine; a `Failed` outcome reports and continues,
        // which is this module's own contract ("one package's failure never
        // stops another's"). `wm` is unused until then -- see this crate's
        // Phase 4b plan for why the parameter is finalised here rather than
        // added alongside its first real caller.
        Step::Winget(w) => {
            let _ = wm;
            StepOutcome::Failed {
                why: format!("{}: the winget executor is not wired yet", w.app()),
                touched: false,
            }
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
    /// `touched` mirrors `StepOutcome::Failed`'s field of the same name: true
    /// when the machine was already altered before this failure happened.
    Failed {
        why: String,
        touched: bool,
    },
    /// Not attempted, and the run is not at fault: the package started
    /// running, or removals are gated off.
    Held(String),
}

/// One package's result, and **which backend it belongs to**.
///
/// A struct rather than the `(Name, ItemResult)` tuple this replaced, and the
/// `backend` is the whole reason: `render_execution` hardcoded the string
/// `"scoop"` on every line it printed. That was *correct* for as long as no
/// `Step::Winget` could reach `execute` -- until Phase 4b Task 13, every
/// winget action fell into `plan_to_steps`'s routing-bug arm and never became
/// a step at all -- and it became false the moment winget got an executor,
/// printing `FAILED scoop Brave.Brave` for a winget package. Nothing in the
/// type system objected, and no grep for a deleted sentence could have found
/// it, because the false word was a backend name.
///
/// A named struct, not a third tuple element: `(Name, &str, ItemResult)` puts
/// two strings side by side with nothing but position telling them apart,
/// which is the shape this crate splits types to avoid (see `Step`/`ScoopStep`
/// and `Outcome::ReadyToFetch`/`ReadyToRemove`).
///
/// `backend` is an owned `String` to match `Action::backend` and
/// `Installed::backend` rather than inventing a third convention -- the two
/// producers are `Step::backend()` (`&'static str`) and an `Action`'s own
/// field, and one of them has nothing static to lend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOutcome {
    pub backend: String,
    pub name: Name,
    pub result: ItemResult,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Execution {
    pub results: Vec<ItemOutcome>,
    /// Ownership records dropped because nothing by that name is installed
    /// any more, each with the backend it was dropped from -- `reconcile_ghosts`
    /// has reconciled **both** backends since Phase 4 Task 14, so a bare
    /// `Name` here was already enough to make `render_execution` mislabel a
    /// winget ghost as scoop's before Phase 4b Task 13 ever ran.
    pub dropped_ghosts: Vec<(String, Name)>,
    /// `Some(reason)` when `write_recovery` failed. Deliberately not a
    /// refusal -- the run still went ahead, because `execute` does not get
    /// to decide that a missing safety net outweighs the packages the user
    /// asked for -- but a warning printed once, above minutes of scoop
    /// output, and never recorded anywhere `Execution` is read back from, is
    /// as good as no warning at all. The caller decides what to do with it.
    pub recovery_write_failed: Option<String>,
}

impl Execution {
    pub fn changed(&self) -> usize {
        self.results
            .iter()
            .filter(|o| o.result == ItemResult::Done)
            .count()
    }
    pub fn failed(&self) -> usize {
        self.results
            .iter()
            .filter(|o| matches!(o.result, ItemResult::Failed { .. }))
            .count()
    }
    /// How many `Failed` results happened only after the machine was already
    /// altered -- see `StepOutcome::Failed`'s doc comment for the exact
    /// shapes. Separate from `changed()`, which means "verified fully done":
    /// a package counted here is neither done nor as it was.
    /// `render_execution` is the one place left that reads this directly, to
    /// choose its wording between "nothing was changed" and "some packages
    /// were changed and some were not" -- `exit_code` itself no longer needs
    /// it (Important 6): every outstanding package is exit 1 now, touched or
    /// not.
    pub fn touched(&self) -> usize {
        self.results
            .iter()
            .filter(|o| matches!(o.result, ItemResult::Failed { touched: true, .. }))
            .count()
    }
    pub fn held(&self) -> usize {
        self.results
            .iter()
            .filter(|o| matches!(o.result, ItemResult::Held(_)))
            .count()
    }
    /// Defined by what the operator must do next, not by what happened
    /// internally (Important 6):
    ///
    /// - **0** -- the plan is fully realised on disk and nothing is
    ///   outstanding.
    /// - **1** -- something is outstanding: a package failed, was held, or
    ///   (via the floor `main.rs` applies on top of this) could not be
    ///   prepared, or was skipped because its own process was running. The
    ///   machine may or may not actually have changed -- `render_execution`
    ///   is where that distinction is still drawn, in the wording, not in
    ///   the exit code.
    /// - **2** -- refused before anything was attempted; nothing changed.
    ///
    /// This used to also return 2 for a failure with `changed() == 0 &&
    /// touched() == 0` -- "nothing to look at" -- but that rule made a
    /// package held by the running re-sampler exit the same way as a
    /// converged machine: 0. `held()` folds into "outstanding" here for
    /// exactly that reason, and a plain untouched failure now reads the same
    /// as a touched one: both still need the operator to look at why, even
    /// when the answer turns out to be "nothing changed".
    ///
    /// **This method alone only ever sees a re-sampler hold** -- a process
    /// that started running *during* the run (see `execute`'s own doc
    /// comment). The far more common nightly shape -- an editor already open
    /// before `apply` was even invoked -- is `SkipReason::Running` at plan
    /// time: the package never becomes a `Step`, so it never reaches
    /// `self.results` on its own, and this method would return 0 for it.
    /// `main.rs` is what actually closes that gap, on both ends: it pushes
    /// the same `Held` shape into `Execution` before the closing table is
    /// printed, *and* floors the exit code independently of that push,
    /// straight from `Preparation`. Earlier revisions of this comment cited
    /// the editor-left-open story as something `held()` folding into
    /// "outstanding" had already handled here; it had not -- only the
    /// re-sampler case had, which is a narrower and rarer shape than the
    /// nightly one that motivated the story in the first place.
    ///
    /// `refused` and "changed something" can never both be true: a refusal
    /// means `execute` returned `Err` before performing a single step, so
    /// nothing in `self` could have changed.
    pub fn exit_code(&self, refused: bool) -> i32 {
        debug_assert!(
            !(refused && self.changed() > 0),
            "a refused run cannot also have changed something"
        );
        if refused {
            return 2;
        }
        if self.failed() > 0 || self.held() > 0 {
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
/// Removals never appear: this file only ever puts software back. Neither
/// does a winget step yet -- see the `Step::Winget` arm below.
pub fn write_recovery(path: &Path, steps: &[Step]) -> Result<()> {
    use std::fmt::Write as _;
    let mut text = String::from(
        "@echo off\r\nREM Written by dotpkg before it changed anything.\r\n\
         REM Each line reinstalls one package from the manifest dotpkg staged\r\n\
         REM and hash-verified. Safe to run more than once.\r\n",
    );
    for s in steps {
        let (staged, arch) = match s {
            Step::Scoop(ScoopStep::Install { staged, arch, .. })
            | Step::Scoop(ScoopStep::Replace { staged, arch, .. }) => (staged, arch),
            Step::Scoop(ScoopStep::Remove { .. }) => continue,
            // A later task adds a winget recovery line once this crate has
            // measured one to build and an argv builder to build it from.
            // Skipped here exactly like `ScoopStep::Remove` above: this
            // function's job is "put software back", and there is nothing
            // yet to put a winget package back FROM.
            Step::Winget(_) => continue,
        };
        // Built from `install_argv` -- the exact argv the executor itself
        // runs -- rather than typed out a second time here. A flag added to
        // `install_argv` (like `-u`, which keeps a scoop self-update out of
        // the uninstall/install window) then cannot silently drop out of
        // just this line: measured, hand-duplicating it left a mutation that
        // deleted `-u` from only the recovery line green across the whole
        // suite, while the same deletion in `install_argv` itself turned
        // red immediately.
        let argv = crate::backend::scoop::install_argv(staged, arch.as_deref());
        let last = argv.len() - 1;
        let mut line = String::from("scoop");
        for (i, part) in argv.iter().enumerate() {
            line.push(' ');
            if i == last {
                // `%` is expanded by cmd even *inside* double quotes, so an
                // unescaped `%` in a staged path (`C:\Users\a%b\...`) would
                // make the recovery line reference an undefined batch
                // variable instead of the manifest dotpkg actually staged.
                // Doubling it to `%%` is how a batch file spells a literal
                // `%`. Only the manifest path -- always argv's last element
                // -- can contain one; the rest is scoop's own flags and an
                // architecture name.
                line.push('"');
                line.push_str(&part.replace('%', "%%"));
                line.push('"');
            } else {
                line.push_str(part);
            }
        }
        let _ = writeln!(text, "{line}\r");
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
/// direction where scoop also exits 0.
///
/// `execute` calls this whenever its step list contains at least one
/// `Step::Scoop`, not unconditionally -- see Task 7. A winget-only run is
/// exempt: a winget removal is verified by re-asking winget, which never
/// reads this root at all, so a wrong or missing `$SCOOP` cannot make a
/// winget uninstall lie the way it can a scoop one. Refusing that run anyway
/// would refuse a machine that has winget and no scoop for a hazard it was
/// never exposed to.
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
/// Refuses via `root_looks_like_scoop` as soon as `steps` is ordered, before
/// anything else -- including before the recovery file is written -- but
/// only when `steps` contains at least one `Step::Scoop`; see that
/// function's own doc comment for why a winget-only run is exempt. This used
/// to be a precondition documented as the caller's job ("`main.rs` does");
/// it wasn't actually being called anywhere, which left the exact hazard it
/// exists for -- every uninstall verifying as successful against a wrong or
/// typo'd `$SCOOP` -- wide open. Defence belongs at the point of use.
///
/// `running` is a sampler, not a snapshot: it is called again immediately
/// before each step's mutation, not once at the top. A single `&Running`
/// captured before `execute` starts would only ever re-confirm what the
/// planner already knew when it built the plan -- any package running at
/// that moment was already turned into `Skip{Running}` upstream, so it never
/// reaches here as a `Step` at all. The case this sampler exists for is a
/// package that starts running *during* the run: a prefetch of two dozen
/// packages can take minutes, and a user who opens their editor partway
/// through must not have it uninstalled out from under them.
pub fn execute(
    root: &Path,
    steps: Vec<Step>,
    m: &dyn Mutator,
    wm: &dyn WingetMutator,
    state: &mut State,
    running: &dyn Fn() -> Running,
    opts: &ExecOptions,
) -> Result<Execution, String> {
    let steps = order(steps);

    // Conditional, not unconditional. The hazard this guards is scoop's
    // alone: `verify::verdict` maps "no apps/ directory" to absent and
    // `Expected::Absent` maps absent to `Ok(())`, so a wrong or typo'd
    // `$SCOOP` verifies every scoop uninstall as successful. A winget removal
    // is verified by re-asking winget, which never reads this root at all --
    // see `root_looks_like_scoop`'s own doc comment.
    if steps.iter().any(|s| matches!(s, Step::Scoop(_))) {
        root_looks_like_scoop(root)?;
    }

    let mut ex = Execution::default();

    if let Some(p) = &opts.recovery_path {
        if let Err(e) = write_recovery(p, &steps) {
            let msg = format!("{e:#}");
            eprintln!("warning: could not write the recovery script: {msg}");
            ex.recovery_write_failed = Some(msg);
        }
    }

    for step in &steps {
        let app = step.app().clone();
        // Called here, per step -- not hoisted out of the loop. See the
        // function doc for why a snapshot cannot do this job.
        if running().covers_any(&app, step.guard_names()) {
            ex.results.push(ItemOutcome {
                backend: step.backend().into(),
                name: app,
                result: ItemResult::Held(
                    "started running since the plan was made -- stop it and run again".into(),
                ),
            });
            continue;
        }
        let r = match run_step(root, m, wm, state, step) {
            StepOutcome::Done => ItemResult::Done,
            StepOutcome::Failed { why, touched } => ItemResult::Failed { why, touched },
        };
        ex.results.push(ItemOutcome {
            backend: step.backend().into(),
            name: app,
            result: r,
        });
    }
    Ok(ex)
}
