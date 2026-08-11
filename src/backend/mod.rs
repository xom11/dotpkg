pub mod scoop;
pub mod winget;
pub mod winget_exec;

use crate::config::Config;
use crate::lock::Lock;
use crate::model::{Installed, Name};
use crate::update::Resolution;
use anyhow::Result;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// What one scan found, plus what it could not read.
///
/// The two fields exist because "this app is not installed" and "this app is
/// installed and I was not allowed to look at it" are different facts, and a
/// bare `Vec<Installed>` reports them identically. A scan must never abort on
/// one bad directory -- forty good ones would vanish with it -- but it must not
/// pretend the bad one was absent either.
#[derive(Debug, Default)]
pub struct Scan {
    pub installed: Vec<Installed>,
    /// Installed, but this backend could not establish its state.
    ///
    /// `plan()` must not read a name's absence from `installed` as "not
    /// installed". The scoop case is a manifest that cannot be traversed; the
    /// winget case is a row with no source, which cannot be compared against
    /// any index. Both would otherwise become `Install` and then, under
    /// `--yes`, an uninstall-and-reinstall of a package that was never absent.
    ///
    /// One field rather than two: the *cause* differs per backend and belongs
    /// in `warnings`, but the *consequence* for the planner is identical.
    pub opaque: Vec<Name>,
    /// One line per entry that was skipped for a reason the user should see.
    /// Expected-and-normal skips (a half-finished install with no manifest yet)
    /// do not appear here.
    pub warnings: Vec<String>,
}

/// What every `Backend`'s resolver needs beyond the one package it is being
/// asked about.
///
/// Before this task, `update::run` held these as plain local variables and
/// handed `scoop_root`/`declared` straight to the free functions
/// `bucket::choose_bucket` and `bucket::resolve_latest`. Phase 4 Task 13 moves that
/// call onto `Backend`, so the same values need a home that travels with the
/// trait call instead of living only in `update::run`'s stack frame --
/// `offline`, `declared` and `scoop_root` are exactly those three, moved
/// rather than redesigned.
///
/// `old` and `warnings` are the two pieces that were not visible as part of
/// that "shape" until the move was actually attempted:
///
/// - **`old`** -- `bucket::choose_bucket`'s strongest precedence signal is the
///   bucket a package is *already* pinned to (so re-resolving never silently
///   jumps a package to a second declared bucket that happens to carry the
///   same name). That used to be `old.scoop.get(name)`, read directly out of
///   `update::run`'s own `old: &Lock` parameter; moving the call means the
///   resolver needs the same lookup, so the lock it reads from has to travel
///   too.
/// - **`warnings`** -- `bucket::resolve_latest` can succeed by falling back
///   to the bucket tip when no single commit carries the manifest's current
///   content (`Latest::fell_back_to_tip`), and `update::run` turned that into
///   a warning on the *success* path. `Resolution` is deliberately just
///   `Resolved { pin }` / `Failed { why }` -- both this task's own brief and
///   every existing match on it assume exactly those two shapes, so adding a
///   field to carry this one warning would ripple through code this task has
///   no reason to touch. A `RefCell` sink is the narrower change: a resolver
///   that has nothing to say through it writes nothing, and one that does
///   (today, only `Scoop::resolve_latest`) can say it without widening
///   `Resolution` for every backend and every caller.
pub struct ResolveCtx<'a> {
    /// Whether this run skipped fetching. Carried through because it was part
    /// of `update::run`'s own state, not because either resolver in this task
    /// reads it: the fetch itself happens in `update::run`'s per-bucket loop,
    /// which runs before any resolver is called and is unaffected by this
    /// task.
    pub offline: bool,
    pub declared: &'a Config,
    pub scoop_root: &'a Path,
    /// The lock as it stood before this resolution.
    pub old: &'a Lock,
    /// Where a resolver may record something the caller should print without
    /// it changing the `Resolved`/`Failed` verdict. See this struct's own doc
    /// comment for why a sink, rather than a field on `Resolution`.
    pub warnings: &'a RefCell<Vec<String>>,
    /// The id a resolver actually matched, when that differs from -- or
    /// simply needs recording independently of -- the name it was asked
    /// about.
    ///
    /// Winget-only: measured, `winget --id <spelling>` without `--exact`
    /// folds case on the way in and hands back the canonical id in `Found
    /// <name> [<Id>]` (`docs/measurements-2026-08-09-winget.md` §3). Both
    /// `Winget::resolve_latest` and `Winget::resolve_installed` set this on a
    /// successful resolution; `Scoop`'s two resolvers never touch it. A sink
    /// rather than a field on `Resolution`, for the same reason `warnings`
    /// is one: `Resolution` is deliberately just `Resolved { pin }` / `Failed
    /// { why }`, and the caller -- `update::run` / `adopt::run` -- reads this
    /// immediately after each per-package call and clears it with `take()`,
    /// so unlike `warnings` (which accumulates across a whole loop) this one
    /// is scoped to exactly the last call.
    pub canonical: &'a RefCell<Option<Name>>,
    /// Which rule confirmed an ALREADY-INSTALLED package during
    /// `resolve_installed`, for `adopt` to report.
    ///
    /// Scoop-only: `Scoop::resolve_installed` sets it (`Matched::Content` or
    /// `Matched::Version`, from the free function `adopt::resolve_installed`
    /// it delegates to); `Winget::resolve_installed` has no such distinction
    /// (an installed version either still resolves in the index or it does
    /// not) and never touches it; neither `resolve_latest` implementation
    /// does either -- there is no "already installed" to confirm when
    /// resolving "latest". Read the same way `canonical` is: immediately
    /// after the call, via `take()`.
    pub matched: &'a RefCell<Option<crate::adopt::Matched>>,
}

impl ResolveCtx<'static> {
    /// A context with nothing declared, no lock, and `offline: true` -- for a
    /// caller (today, only a test) that exercises a resolver reading none of
    /// `declared`/`scoop_root`/`old`. `adopt::adopt_one_winget` reads none of
    /// them either (winget has no bucket to choose and `adopt` reaches no
    /// network in the first place), but builds its own throwaway `Config`/
    /// `Lock`/`Path` locals rather than calling this: that function has a
    /// natural lifetime to borrow from (its own stack frame, once per
    /// adopted package), so there is no reason for it to leak memory the way
    /// this constructor deliberately does for a caller that has none. Every
    /// referenced value here is leaked (`Box::leak`, never freed) rather
    /// than held in a `static`: the `warnings`/`canonical`/`matched` sinks
    /// are `RefCell`s, which are not `Sync` and so cannot live in a `static`
    /// at all, and leaking a few small, short-lived allocations is cheaper
    /// than giving every field its own synchronization just so one of them
    /// could be `Sync`.
    pub fn offline() -> ResolveCtx<'static> {
        let declared: &'static Config = Box::leak(Box::new(Config::default()));
        let scoop_root: &'static Path = Box::leak(PathBuf::from(".").into_boxed_path());
        let old: &'static Lock = Box::leak(Box::new(Lock::default()));
        let warnings: &'static RefCell<Vec<String>> = Box::leak(Box::new(RefCell::new(Vec::new())));
        let canonical: &'static RefCell<Option<Name>> = Box::leak(Box::new(RefCell::new(None)));
        let matched: &'static RefCell<Option<crate::adopt::Matched>> =
            Box::leak(Box::new(RefCell::new(None)));
        ResolveCtx {
            offline: true,
            declared,
            scoop_root,
            old,
            warnings,
            canonical,
            matched,
        }
    }
}

/// One package manager. `scan` reads state that is already on disk or already
/// known; nothing here reaches the network. Mutating methods arrive in Phase 2.
///
/// `resolve_latest` and `resolve_installed` are the seam Phase 4 exists to
/// prove: before Phase 4 Task 13, `update::run` named `bucket::resolve_latest`
/// directly, so "a new backend slots in without touching the planner" was a
/// promise the code did not keep. Both methods are per-package, not
/// per-scan, because unlike `scan` (unconditionally reads everything on
/// disk) a resolver is asked about one name at a time and the caller decides
/// which names to ask about at all -- `update` skips names outside its
/// `Scope`, and a driver that only wants one package's answer should not pay
/// for every other package's resolution.
pub trait Backend {
    fn name(&self) -> &'static str;
    fn scan(&self) -> Result<Scan>;
    /// Resolve "latest" for `name`, the way `update` needs it.
    fn resolve_latest(&self, name: &Name, ctx: &ResolveCtx) -> Resolution;
    /// Confirm (or refuse) that an already-installed package is still a
    /// valid pin, the way `adopt`'s pin-liveness check needs it.
    fn resolve_installed(&self, inst: &Installed, ctx: &ResolveCtx) -> Resolution;
}

/// What a `scan_or_warn` call established: a real `Scan` (which may still
/// carry its own `warnings`/`opaque` entries), or an outright failure to scan
/// this backend at all.
///
/// Two variants rather than one `Scan` that happens to be empty on failure --
/// the whole point of this task. Before this type existed, `scan_or_warn`
/// returned a plain `Scan`, and a genuine scan failure was indistinguishable
/// from a genuinely empty machine to every caller downstream: `Unscannable`
/// carries the cause and lets `plan()` (via its `unscannable` parameter and
/// `SkipReason::Unscannable`) treat "this backend's state is unknown"
/// differently from "this backend has nothing installed", instead of reading
/// the former as the latter.
#[derive(Debug)]
pub enum ScanOutcome {
    Scanned(Scan),
    Unscannable(String),
}

/// Turns a genuine `scan()` failure into `ScanOutcome::Unscannable` carrying
/// the cause, rather than letting the `Result::Err` propagate out of the
/// caller and abort the whole command.
///
/// Added by Task 14's review: `src/main.rs`'s `status` and
/// `apply::load_everything` both used to write `winget.scan()?`, and
/// `winget list` signals failure through its exit code far more readily than
/// scoop does -- a routine source-update failure exits nonzero, not just a
/// machine with no `winget.exe` on `PATH` at all (`Winget::scan` already
/// handles THAT case gracefully on its own, with one warning and an empty
/// `Scan` -- this is the OTHER failure shape, the one `scan()` deliberately
/// refuses to paper over itself: see `Winget::scan`'s own doc comment on why
/// an empty `Scan` returned in place of a real failure is indistinguishable
/// from a genuinely empty machine, which is exactly what `mass_prune_guard`
/// exists to catch). Before this function existed, that `Err` propagated all
/// the way out of `main`, and scoop's own half of the run -- entirely
/// unrelated to whatever winget hiccup caused it -- never happened either.
///
/// The caller prints the cause the same way every other scan warning already
/// is (see `main.rs`'s `print_scan_warnings_and_merge`) -- no new printing
/// path, and no second message stacked on top.
///
/// **Task 6's correction:** this used to return an empty `Scan` plus one
/// `warnings` entry -- the same shape `Winget::scan` uses for its OTHER
/// failure mode (an absent `winget.exe`) -- and its doc comment argued that
/// was safe: "`plan()`'s prune loop only ever iterates `installed`, so an
/// empty scan can never fabricate a prune." That argument is still true, and
/// only ever covered the prune direction. It said nothing about the other
/// one: a declared, locked package with nothing in `installed` because the
/// scan failed reads exactly like a declared, locked package that is
/// genuinely not installed. When Task 6 wrote that, the consequence for winget
/// was a wrong report line; Phase 4b Task 13 gave winget an executor, so the
/// consequence now is a real `Action::Install` for a package that may already
/// be sitting there, converged. This function was written one task ahead of
/// its own danger, and `ScanOutcome::Unscannable` is that other half's answer:
/// `plan()` takes this backend's name via its `unscannable: &[&'static str]`
/// parameter and skips the backend's declared loop entirely, emitting
/// `SkipReason::Unscannable` for every declared package instead of reading
/// its absence from `installed` as fact.
pub fn scan_or_warn(backend: &dyn Backend) -> ScanOutcome {
    match backend.scan() {
        Ok(scan) => ScanOutcome::Scanned(scan),
        Err(e) => ScanOutcome::Unscannable(format!("could not be scanned: {e:#}")),
    }
}

/// The `Running` every production path receives: process names, scoop's package
/// directories, and winget's package directories, unioned.
///
/// **The only producer of a `Running` outside tests, deliberately.**
/// **Structural**, checkable by grepping `Running::new` across `src/`: every
/// other call site is inside `model.rs`'s own `#[cfg(test)] mod tests`, and the
/// three production sites (`apply::load_everything`, `main.rs`'s `status` arm,
/// `main.rs`'s per-step re-sampler closure) all call this function.
///
/// `Scoop::running_set` used to be that producer and was removed here rather
/// than kept. Its doc comment carried the reasoning this union still rests on,
/// moved here verbatim in substance: name matching and path matching each cover
/// the other's blind spot -- an elevated process reports no `exe` and is caught
/// only by name; a package naming no executable at all (`nodejs`) is caught
/// only by path -- so a caller that drops either input silently loses whatever
/// only that half could see. A scoop-only producer left in place keeps exactly
/// that mistake writable. Phase 4b named the consequence: fixing the scanner
/// and not the mid-run sampler "would close the plan-time hole and leave the
/// during-the-run hole exactly as wide".
///
/// Assembling the union here rather than in `main.rs` is also what makes it
/// testable at all: `main.rs` is the binary crate, so no integration test can
/// reach a closure built there, while this function takes fabricated `Process`
/// values on any OS.
///
/// `winget_ids` is the winget scan's `installed` names and never its `opaque`
/// ones. **Structural:** `plan()` only ever reaches `Running::covers` through an
/// `Installed` (`src/plan.rs:414` and `:462`, both passing an `&Installed`), and
/// an `opaque` id is turned into `SkipReason::Opaque` and `continue`d at
/// `src/plan.rs:345`, before either fence check is reached.
pub fn running_set(
    scoop: &scoop::Scoop,
    winget_ids: &[Name],
    winget_roots: &[PathBuf],
    procs: &[crate::sys::Process],
) -> crate::model::Running {
    let names = procs.iter().map(|p| p.name.clone()).collect();
    let mut dirs = scoop.running_apps(procs);
    dirs.extend(winget::running_ids(winget_roots, procs, winget_ids));
    crate::model::Running::new(names, dirs)
}

/// The winget `installed` names a `running_set` call needs, or none when the
/// scan failed outright. An `Unscannable` winget backend contributes no fence
/// entries, which matches how the same outcome is already treated elsewhere:
/// **structural**, `main.rs`'s `reconcile_ghosts` guards its winget
/// `State::reconcile` call behind `if let ScanOutcome::Scanned(..)` and so
/// reconciles nothing for a winget scan that failed.
///
/// `installed` only, deliberately -- unlike `main.rs`'s `present_after`, which
/// unions `installed` with `opaque`. See `running_set`'s own doc comment for the
/// structural reason an `opaque` id can never be asked about here.
pub fn winget_fence_ids(outcome: &ScanOutcome) -> Vec<Name> {
    match outcome {
        ScanOutcome::Scanned(s) => s.installed.iter().map(|i| i.name.clone()).collect(),
        ScanOutcome::Unscannable(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Name;

    struct FailingBackend;
    impl Backend for FailingBackend {
        fn name(&self) -> &'static str {
            "winget"
        }
        fn scan(&self) -> Result<Scan> {
            Err(anyhow::anyhow!("list exited 1: source update failed"))
        }
        fn resolve_latest(&self, _name: &Name, _ctx: &ResolveCtx) -> Resolution {
            unreachable!("not exercised by this test")
        }
        fn resolve_installed(&self, _inst: &Installed, _ctx: &ResolveCtx) -> Resolution {
            unreachable!("not exercised by this test")
        }
    }

    struct WorkingBackend;
    impl Backend for WorkingBackend {
        fn name(&self) -> &'static str {
            "winget"
        }
        fn scan(&self) -> Result<Scan> {
            Ok(Scan {
                installed: vec![Installed {
                    backend: "winget".into(),
                    name: Name::new("Git.Git"),
                    version: "2.55.0".into(),
                    arch: None,
                    bucket: None,
                    bins: Vec::new(),
                }],
                ..Scan::default()
            })
        }
        fn resolve_latest(&self, _name: &Name, _ctx: &ResolveCtx) -> Resolution {
            unreachable!("not exercised by this test")
        }
        fn resolve_installed(&self, _inst: &Installed, _ctx: &ResolveCtx) -> Resolution {
            unreachable!("not exercised by this test")
        }
    }

    #[test]
    fn a_failed_scan_becomes_unscannable_with_the_cause_named() {
        match scan_or_warn(&FailingBackend) {
            ScanOutcome::Unscannable(why) => assert!(
                why.contains("could not be scanned") && why.contains("source update failed"),
                "the underlying error must still be named, not swallowed: {why}"
            ),
            ScanOutcome::Scanned(s) => {
                panic!("a genuine scan failure must not read as scanned: {s:?}")
            }
        }
    }

    #[test]
    fn a_successful_scan_passes_through_untouched() {
        // The positive control: without it, a version that always discarded
        // the real scan and returned Unscannable would satisfy the test
        // above too.
        match scan_or_warn(&WorkingBackend) {
            ScanOutcome::Scanned(scan) => {
                assert_eq!(scan.installed.len(), 1, "got {:?}", scan.installed);
                assert!(scan.warnings.is_empty(), "got {:?}", scan.warnings);
            }
            ScanOutcome::Unscannable(why) => {
                panic!("a successful scan must not read as unscannable: {why}")
            }
        }
    }
}
