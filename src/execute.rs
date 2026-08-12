//! The executor: the only part of dotpkg that changes installed software.
//!
//! Two seams are faked in tests, and they are **not** equally strong.
//!
//! `Mutator` — the scoop subprocess — is faked and nothing else on that path
//! is: every observation of a scoop result runs against a real directory tree,
//! because a fake that both performs and reports the mutation proves only that
//! it is self-consistent.
//!
//! `WingetMutator` cannot be held to that standard, and this module must not
//! pretend otherwise. **Not because winget has no hash** — it has one and
//! verifies it: `winget show` prints `Installer SHA256`, which is exactly the
//! correction `docs/specs/2026-08-09-phase4-backend-winget-design.md` made
//! to `docs/specs/2026-08-08-design.md`'s "winget pins a version, not a
//! hash", and a correction this module must not quietly undo. What winget
//! lacks is an **on-disk
//! manifest and hash dotpkg can read back after the install**, the way
//! `verify::verdict` compares scoop's installed manifest bytes against the
//! staged file. With no such handle there is nothing independent of winget to
//! check winget's own write against, so `run_winget_step` verifies by
//! re-asking the very seam that just performed the mutation, and a fake there
//! really is only self-consistent. That is a structural weakness, said plainly
//! here and at `WingetState`'s own doc comment rather than dressed up as equal
//! to scoop's.

use crate::backend::winget_exec::{
    winget_verdict, WingetMutator, WingetState, CANNOT_UNINSTALL_ELEVATED, NO_AVAILABLE_UPGRADE,
};
use crate::config::BucketDecl;
use crate::model::{Name, Running, SCOOP, WINGET};
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
/// `run_winget_step`'s `touched` bookkeeping has no uninstall half to reason
/// about. One call, either direction, covers both a fresh install and a
/// version change -- hence `Set`, not `Install`/`Replace`.
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
    /// behind it. `Step::app()` is its only caller in the crate:
    /// `run_winget_step`, the later task this comment used to predict would
    /// want it too, matches each variant and destructures `id` out of it
    /// directly instead.
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
    /// failure happened. On the scoop side two shapes set it: a
    /// `ScoopStep::Replace` whose uninstall verified `Absent` before its
    /// install then failed, and any install (fresh, or the second half of a
    /// replace) whose `verdict` disagreement is evidence of residue --
    /// `HalfInstalled`, `ContentDiffers`, or `Unreadable` (which means
    /// "unknown" and is treated as touched in the safe direction, so an
    /// operator looks). `NotInstalled` alone means the machine is genuinely
    /// as it was. A `ScoopStep::Remove` sets it the mirror way: `StillPresent`
    /// is untouched, `Unreadable` is touched.
    ///
    /// A `WingetStep` sets it for exactly one epistemic state -- *the
    /// mutation ran and dotpkg cannot see the result* -- reached two ways:
    /// the rescan came back `Unconfirmable`, or the rescan could not run at
    /// all (`winget_verdict` returning `Err`, which means only that `winget
    /// list` could not be spawned). Both are the same "unknown is touched"
    /// rule as `Unreadable` above, and they must not disagree merely because
    /// a different call was the one that failed to answer.
    ///
    /// Everywhere else `run_winget_step` reports `touched: false`, for two
    /// different kinds of reason -- two paths structurally (winget never ran
    /// at all) and four because those failing shapes were measured to leave
    /// the package where it was. Its own doc comment says which is which,
    /// and deliberately claims no more than the measurements support.
    ///
    /// Either way, the package this describes is neither "done" nor "as it
    /// was", and `Execution::exit_code` must not fold that into "nothing
    /// changed".
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

/// Perform one winget step and prove by rescan that it happened.
///
/// **The rescan is the verdict, never the exit code.** `winget_verdict`'s own
/// doc comment carries the measurements; the consequence here is that this
/// function reads `out.code` only to *disambiguate* two rescan answers that
/// would otherwise be identical, and never to decide success on its own.
///
/// **`touched` is `false` on 7 of the 11 failure paths here, and the reasons
/// are of two different kinds. Which is which matters, because only one of
/// them is a measurement.** (Recounted for the post-merge audit's M2: this
/// comment used to say "all but two" and "the two `touched: true` paths" --
/// stale counts in the one comment whose entire subject is which paths are
/// touched.)
///
/// *Structural, not measured* -- the two `Err(e)` arms on `m.set`/`m.remove`
/// themselves. `RealWingetMutator::run` returns `Err` only when the process
/// could not be spawned at all (`CmdError::NotFound`, or an `io::Error` from
/// `Command::output`), so winget never ran and nothing can have changed. That
/// is provable from `RealWingetMutator::run`, and no measurement is claimed
/// for it.
///
/// *Measured* -- the five shapes where winget **did** run, reported failure,
/// and left the package exactly where it was.
/// `docs/measurements-2026-08-10-winget-write-path.md` records which
/// observation says so for each: `0x8A15002B` declining a downgrade (§2, the
/// "unchanged" column on every row), `0x8A150017` refusing a
/// version-mismatched uninstall (§8, "still installed? **yes**"),
/// `0x8A150014` against a package that was absent to begin with (§8, nothing
/// there to change), `0x8A15007D` refusing an elevated uninstall (§5 -- the
/// de-elevated control then uninstalled that same package successfully,
/// which is what proves the refusal had removed nothing), and `0x8A150017`
/// again -- this time from `install` itself, against a pin no longer in
/// winget's index (§6). That fifth one was missing from this list before the
/// audit found the count short: it is the `Set` arm's own generic `At(v)`
/// case below, and that arm's own doc comment names it as the gap
/// `NO_AVAILABLE_UPGRADE`'s ordering cannot close -- this is where such a run
/// actually lands.
///
/// Those five are per-probe observations of *the package's state*. The
/// document does also hash `winget list` itself, but **aggregate only, never
/// per failing probe**, so "left `winget list` byte-identical" is not a claim
/// this comment may make for any of the five: W1's bracket (`:33`) covers all
/// 12 probes at once, and every one of those 12 targeted an absent package or
/// a nonexistent version rather than any shape above; W2's (`:39`-`:40`) runs
/// start-to-cleanup and spans the *successful* installs in between.
///
/// **The four `touched: true` paths** are the mutation having run with its
/// result unseeable -- see the `Unconfirmable` and rescan-`Err` arms, in both
/// the `Set` and `Remove` blocks below, which carry the reasoning where it
/// applies.
///
/// A winget version change is also **one** call -- `install --version`
/// performs the upgrade directly -- so unlike `ScoopStep::Replace` there is no
/// uninstall half that could leave the package absent mid-step, and therefore
/// no window this function has to reason about.
pub fn run_winget_step(m: &dyn WingetMutator, state: &mut State, step: &WingetStep) -> StepOutcome {
    match step {
        WingetStep::Set { id, version, .. } => {
            let out = match m.set(id, version) {
                Ok(out) => out,
                Err(e) => {
                    return StepOutcome::Failed {
                        why: format!("{id}: could not run winget install: {e}"),
                        touched: false,
                    }
                }
            };
            match winget_verdict(m, id) {
                // `touched: true`, and NOT because anything was seen to
                // change. `winget_verdict` returns `Err` from exactly one
                // place -- `m.list_one(id)?` -- because every other problem it
                // can hit becomes `Ok(Unconfirmable)` instead. So this arm has
                // exactly one meaning: `winget install` **already ran** (its
                // exit code is in the message right there) and then the rescan
                // could not be spawned. The machine may well have changed and
                // dotpkg cannot look.
                //
                // That is the same epistemic state as the `Unconfirmable` arm
                // below, and it must not get the opposite answer just because
                // a different call was the one that failed to answer.
                // `render_execution` reads `Execution::touched()` to choose
                // between "nothing was changed" and "some packages were
                // changed and some were not"; `false` here would tell an
                // operator nothing happened directly after a mutation that
                // did.
                Err(e) => StepOutcome::Failed {
                    why: format!(
                        "{id}: install ran (exit {}) but the rescan could not: {e}",
                        out.code
                    ),
                    touched: true,
                },
                // **This arm must stay above the `NO_AVAILABLE_UPGRADE` one
                // below, and the order is the whole rule, not a style
                // choice.** Measured, `0x8A15002B` comes back for BOTH a
                // converged machine (already at exactly `version` -- a
                // success) and a declined downgrade (a failure); only the
                // rescan tells them apart. Asking "is the machine where the
                // pin says?" first makes the converged case `Done` whatever
                // the exit code was.
                //
                // Verified by reversing them rather than reasoned about:
                // `a_converged_package_is_done_even_though_winget_exited_
                // nonzero` then fails with `installed 0.24.1, pinned 0.24.1
                // -- dotpkg will not downgrade`, which is a converged machine
                // being reported as a failure every single run. That test is
                // what pins this order; nothing in the type system does.
                Ok(WingetState::At(v)) if v == *version => {
                    // Claim only now, and preserve an existing `adopt` --
                    // the same rule `run_scoop_step` follows, for the same
                    // reason: ownership is intent, and a version change is
                    // an implementation detail of honouring it.
                    if state.ownership(WINGET, id).is_none() {
                        state.set(WINGET, id, Ownership::Installed);
                    }
                    StepOutcome::Done
                }
                // Reached only when the rescan disagrees with the pin, so
                // `out.code` here can only mean the declined downgrade.
                //
                // **A measured gap this arm cannot close, keyed as it is on
                // `NO_AVAILABLE_UPGRADE`.** A machine ahead of its pin *and* a
                // pin no longer in the index exits `NO_VERSION_FOUND`
                // (`0x8A150017`) instead -- the package-level and
                // version-level failures are resolved before any upgrade
                // comparison happens (measured, §6: "the package-level
                // failure takes precedence over the version-level one"). Such
                // a run falls through to the generic arm below and gets
                // neither "will not downgrade" nor `dotpkg update`, even
                // though a stale pin is exactly when the advice would help
                // most. Widening this arm to `NO_VERSION_FOUND` is the wrong
                // fix, and that is why the gap is documented instead of
                // closed: that code proves only "this version is not
                // available", which is equally what a machine *behind* an
                // unavailable pin gets, so keying on it would print "dotpkg
                // will not downgrade" at runs that were never downgrades.
                Ok(WingetState::At(v)) if out.code == NO_AVAILABLE_UPGRADE => StepOutcome::Failed {
                    why: format!(
                        "{id}: installed {v}, pinned {version} -- dotpkg will not downgrade \
                         a winget package. Measured: `winget install --version` only ever \
                         moves a package up, and reports \"No available upgrade found\" \
                         instead. Run `dotpkg update` to move the pin forward."
                    ),
                    touched: false,
                },
                Ok(WingetState::At(v)) => StepOutcome::Failed {
                    why: format!(
                        "{id}: asked winget for {version} (exit {}), rescan reports {v}",
                        out.code
                    ),
                    touched: false,
                },
                Ok(WingetState::Absent) => StepOutcome::Failed {
                    why: format!(
                        "{id}: install did not happen -- winget exited {} and the rescan finds \
                         nothing installed: {}",
                        out.code,
                        out.stdout.lines().next().unwrap_or("(no output)")
                    ),
                    touched: false,
                },
                Ok(WingetState::Unconfirmable(why)) => StepOutcome::Failed {
                    why: format!(
                        "{id}: winget exited {}, and the rescan cannot confirm the result -- {why}",
                        out.code
                    ),
                    // The rescan ran and could not establish anything -- the
                    // second of the two ways this function reaches "the
                    // mutation ran and dotpkg cannot see the result", the
                    // rescan-`Err` arm above being the first. Unknown, so
                    // treated as touched in the safe direction, and an
                    // operator looks instead of being told nothing happened.
                    // Same rule as `verify::Disagreement::Unreadable` on the
                    // scoop side.
                    touched: true,
                },
            }
        }
        WingetStep::Remove { id, version, .. } => {
            let out = match m.remove(id, version) {
                Ok(out) => out,
                Err(e) => {
                    return StepOutcome::Failed {
                        why: format!("{id}: could not run winget uninstall: {e}"),
                        touched: false,
                    }
                }
            };
            match winget_verdict(m, id) {
                // `touched: true`, for the identical reason as the `Set` arm
                // above: the uninstall already ran and the rescan could not be
                // spawned, so the package may be gone and dotpkg cannot look.
                // The removal direction makes this if anything sharper -- an
                // operator told "nothing was changed" would not go looking for
                // a package that is no longer there.
                Err(e) => StepOutcome::Failed {
                    why: format!(
                        "{id}: uninstall ran (exit {}) but the rescan could not: {e}",
                        out.code
                    ),
                    touched: true,
                },
                // Above every failure arm for the mirror of the reason the
                // `Set` arms are ordered the way they are: `0x8A150014` from
                // `uninstall` means "no *installed* package", which for a
                // `Remove` is the DESIRED end state and is indistinguishable
                // by exit code from "that id is wrong". Nothing being there
                // is done, whatever winget exited.
                Ok(WingetState::Absent) => {
                    state.remove(WINGET, id);
                    StepOutcome::Done
                }
                Ok(WingetState::At(v)) if out.code == CANNOT_UNINSTALL_ELEVATED => {
                    StepOutcome::Failed {
                        why: format!(
                            "{id}: still installed at {v}. winget refuses to uninstall a \
                             user-scope package while dotpkg is running elevated. Re-run \
                             without elevation."
                        ),
                        touched: false,
                    }
                }
                Ok(WingetState::At(v)) => StepOutcome::Failed {
                    why: format!(
                        "{id}: uninstall did not happen -- winget exited {} and the rescan \
                         still reports {v}: {}",
                        out.code,
                        out.stdout.lines().next().unwrap_or("(no output)")
                    ),
                    touched: false,
                },
                Ok(WingetState::Unconfirmable(why)) => StepOutcome::Failed {
                    why: format!(
                        "{id}: winget exited {}, and the rescan cannot confirm the removal -- {why}",
                        out.code
                    ),
                    // Same reasoning as the `Set` arm above: unknown is
                    // touched, so an operator looks.
                    touched: true,
                },
            }
        }
    }
}

/// The write half of a backend: everything `execute` needs in order to perform
/// one already-resolved step and prove it happened.
///
/// `backend::Backend` is the read half -- `scan` and the two `resolve`s -- and
/// it carried the whole of the design's promise that "the backend trait exists
/// from v1 so choco slots in without touching the planner". **That promise held
/// for reading and not for writing.** The write path was two unrelated
/// per-backend seams (`Mutator` for scoop, `WingetMutator` for winget) threaded
/// through `execute` and `run_step` as a hand-written pair of parameters, so a
/// third backend meant a third parameter at every call site rather than a third
/// implementation of anything. This is the contract that was missing.
///
/// **It is deliberately at the step level, not the argv level.** The two
/// process seams underneath keep their own shapes, because those shapes are
/// honestly different -- scoop installs a staged manifest path with an
/// architecture, winget sets a version by id -- and flattening them into one
/// signature would either lose that difference or lie about it. What the
/// backends genuinely have in common is not an argv; it is *run one step, and
/// answer with a `StepOutcome` that never mistakes "the command exited 0" for
/// "it happened"*.
///
/// `Step` is an associated type so one backend's executor cannot be handed
/// another backend's step: `ScoopSide::run` takes a `ScoopStep` and there is no
/// signature in which a `WingetStep` fits. Same make-the-mistake-unwritable
/// move `Step`'s own split already makes, one layer up.
pub trait Mutates {
    type Step;
    fn run(&self, state: &mut State, step: &Self::Step) -> StepOutcome;
}

/// scoop's write half: the seam, plus the root every scoop verification reads.
pub struct ScoopSide<'a> {
    pub root: &'a Path,
    pub mutator: &'a dyn Mutator,
}

impl Mutates for ScoopSide<'_> {
    type Step = ScoopStep;

    fn run(&self, state: &mut State, step: &ScoopStep) -> StepOutcome {
        run_scoop_step(self.root, self.mutator, state, step)
    }
}

/// winget's write half. **No root, and that asymmetry is the point:** a winget
/// step is verified by re-asking winget, which never reads the scoop root at
/// all -- the same asymmetry `root_looks_like_scoop`'s own doc comment records,
/// and the reason a winget-only run is exempt from that check. Carrying a root
/// here would make the two sides look interchangeable when they are not.
pub struct WingetSide<'a> {
    pub mutator: &'a dyn WingetMutator,
}

impl Mutates for WingetSide<'_> {
    type Step = WingetStep;

    fn run(&self, state: &mut State, step: &WingetStep) -> StepOutcome {
        run_winget_step(self.mutator, state, step)
    }
}

/// Every backend's write half, in one value.
///
/// This exists so that adding a backend adds a **field**, not a parameter to
/// `execute` and `run_step` and therefore to all 27 of their call sites. The
/// dispatch in `run_step` stays a wildcard-free match on purpose: that arm is a
/// decision point a new backend must be made to face, exactly like
/// `plan::Capability` and `apply::is_outstanding`, and a compile error is the
/// only reliable way to ask the question.
pub struct Backends<'a> {
    pub scoop: ScoopSide<'a>,
    pub winget: WingetSide<'a>,
}

impl<'a> Backends<'a> {
    pub fn new(root: &'a Path, scoop: &'a dyn Mutator, winget: &'a dyn WingetMutator) -> Self {
        Backends {
            scoop: ScoopSide {
                root,
                mutator: scoop,
            },
            winget: WingetSide { mutator: winget },
        }
    }
}

/// Perform one step -- scoop or winget -- and prove it happened.
///
/// Dispatches on backend and nothing else: a winget `Step` cannot reach
/// scoop's executor, and a scoop `Step` cannot reach the winget arm, because
/// the match is over `Step` itself, not over some flag carried alongside it,
/// and each side's `Mutates::Step` refuses the other's type outright.
pub fn run_step(b: &Backends, state: &mut State, step: &Step) -> StepOutcome {
    match step {
        Step::Scoop(s) => b.scoop.run(state, s),
        Step::Winget(w) => b.winget.run(state, w),
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
/// `Step::Winget` could reach `execute`, which held until Phase 4b Task 13 --
/// under `Capability::ReportsOnly` a winget difference was
/// `Action::Skip { reason: ReportedOnly }`, so it became `Intent::Skip`, then
/// `Outcome::Skipped`, and was routed by `plan_to_steps`'s **`Skipped`** arm
/// into `unusable`. It never became a step, but it never touched the
/// routing-bug arm either: that arm needs a ready outcome, and was reachable
/// only from a hand-built `Preparation`. It became false the moment winget got
/// an executor, printing `FAILED scoop Brave.Brave` for a winget package.
/// Nothing in the type system objected, and no grep for a deleted sentence
/// could have found it, because the false word was a backend name.
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
    /// any more, each with the backend it was dropped from. `reconcile_ghosts`
    /// reconciled `SCOOP` alone until Phase 4b Task 8 added the winget half, so
    /// the backend is here from the first commit that could produce a winget
    /// entry -- not a repair for a mislabelled line some earlier run printed,
    /// because no winget record was ever dropped before this branch.
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

/// One reinstall line per artifact in the run, written **before** the first
/// mutation.
///
/// A run that dies leaves a file that puts the machine back. A run that only
/// prints advice leaves nothing once the terminal is gone -- and the terminal
/// is exactly what a broken `git` or a broken shell takes with it.
///
/// Removals never appear: this file only ever puts software back.
/// `ScoopStep::Remove` and `WingetStep::Remove` are both skipped for that
/// reason.
///
/// The two backends do NOT carry the same promise, and the file's own header
/// says so rather than leaving a human whose run just died to assume they do.
/// A scoop line names a manifest dotpkg **staged and hash-verified on local
/// disk**; replaying it puts back bytes dotpkg already proved. A winget line
/// is a **request re-resolved against an index dotpkg does not hold** --
/// winget, not dotpkg, decides at replay time whether that exact version is
/// still there to install. Measured
/// (`docs/measurements-2026-08-09-winget.md` §4): version retention is a
/// publisher policy, not a winget guarantee, spanning 8 versions
/// (`BurntSushi.ripgrep.MSVC`) to 828 (`JanDeDobbeleer.OhMyPosh`) -- so a
/// winget recovery line can fail in a way a scoop one, replaying a file
/// already on disk, cannot.
pub fn write_recovery(path: &Path, steps: &[Step]) -> Result<()> {
    use std::fmt::Write as _;
    let mut text = String::from(
        "@echo off\r\nREM Written by dotpkg before it changed anything.\r\n\
         REM A scoop line below reinstalls a manifest dotpkg staged and\r\n\
         REM hash-verified on local disk: replaying it puts back bytes\r\n\
         REM dotpkg already proved. A winget line is a different promise: \
         it is a request \
         re-resolved against an index dotpkg does not hold, and if that \
         exact version has since fallen out of the index -- a publisher's \
         call, not a winget guarantee -- the line fails. Safe to run more \
         than once.\r\n",
    );
    for s in steps {
        match s {
            Step::Scoop(ScoopStep::Install { staged, arch, .. })
            | Step::Scoop(ScoopStep::Replace { staged, arch, .. }) => {
                // Built from `install_argv` -- the exact argv the executor
                // itself runs -- rather than typed out a second time here. A
                // flag added to `install_argv` (like `-u`, which keeps a
                // scoop self-update out of the uninstall/install window)
                // then cannot silently drop out of just this line: measured,
                // hand-duplicating it left a mutation that deleted `-u` from
                // only the recovery line green across the whole suite, while
                // the same deletion in `install_argv` itself turned red
                // immediately.
                let argv = crate::backend::scoop::install_argv(staged, arch.as_deref());
                let last = argv.len() - 1;
                let mut line = String::from("scoop");
                for (i, part) in argv.iter().enumerate() {
                    line.push(' ');
                    if i == last {
                        // `%` is expanded by cmd even *inside* double
                        // quotes, so an unescaped `%` in a staged path
                        // (`C:\Users\a%b\...`) would make the recovery line
                        // reference an undefined batch variable instead of
                        // the manifest dotpkg actually staged. Doubling it
                        // to `%%` is how a batch file spells a literal `%`.
                        // Only the manifest path -- always argv's last
                        // element -- can contain one; the rest is scoop's
                        // own flags and an architecture name.
                        line.push('"');
                        line.push_str(&part.replace('%', "%%"));
                        line.push('"');
                    } else {
                        line.push_str(part);
                    }
                }
                let _ = writeln!(text, "{line}\r");
            }
            Step::Scoop(ScoopStep::Remove { .. }) => continue,
            Step::Winget(WingetStep::Set { id, version, .. }) => {
                // Built from `set_argv`, for the same reason the scoop line
                // is built from `install_argv` and not typed out a second
                // time: a flag dropped from `set_argv` must be missing here
                // too, not silently preserved by a hand-written copy.
                let argv = crate::backend::winget_exec::set_argv(id, version);
                // No `%`-doubling and no quoting here, unlike the scoop line
                // above: every element is one of winget's own flags, a
                // package id, or a dotted version, and none of those three
                // shapes can contain a space or a `%` the way an arbitrary
                // staged filesystem path can.
                let line = format!("winget {}", argv.join(" "));
                let _ = writeln!(text, "{line}\r");
            }
            Step::Winget(WingetStep::Remove { .. }) => continue,
        }
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
    b: &Backends,
    steps: Vec<Step>,
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
        root_looks_like_scoop(b.scoop.root)?;
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
        let r = match run_step(b, state, step) {
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
