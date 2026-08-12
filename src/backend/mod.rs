pub mod scoop;
pub mod winget;
pub mod winget_exec;

use crate::config::Config;
use crate::lock::Lock;
use crate::model::{Installed, Name};
use crate::update::Resolution;
use anyhow::Result;
use std::cell::RefCell;
use std::collections::BTreeMap;
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
/// other call site is inside `model.rs`'s own `#[cfg(test)] mod tests`.
///
/// Its only `src/` caller is `apply::sample_fence_with_roots`, which is how the
/// three production sites (`apply::load_everything`, `main.rs`'s `status` arm,
/// `main.rs`'s per-step re-sampler closure) reach it -- they call
/// `apply::sample_fence` and never this function directly. It stays `pub`
/// because three tests in `tests/scoop_scan.rs` call it to exercise scoop's two
/// halves against an empty winget side.
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
/// testable **on any OS with fabricated `Process` values**, which is the property
/// that matters: `tests/cli.rs` can and does drive the real binary, but only
/// against a live process under a real scoop root, and never against winget at
/// all -- see `apply::sample_fence` for the mechanism that rules the winget half
/// out there.
///
/// `winget_ids` is the winget scan's `installed` names and never its `opaque`
/// ones. **Structural:** `plan()` only ever reaches `Running::covers` through an
/// `Installed` (`src/plan.rs::plan_backend`, both passing an `&Installed`), and
/// an `opaque` id is turned into `SkipReason::Opaque` and `continue`d at
/// `src/plan.rs::plan_backend`, before either fence check is reached.
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

/// Add `[winget.guard]`'s process names to the matching `Installed.bins`, and
/// report every guard key that matched nothing.
///
/// **One merge point serves both fences. Structural**, three hops each,
/// checkable by reading:
///
/// - Plan time: `plan()` hands a whole `&Installed` to `Running::covers`
///   (`src/plan.rs::plan_backend`), and `covers`' third disjunct asks
///   `inst.bins` against the live process names.
/// - Mid-run: `apply::guard_for` clones the same `Installed.bins` into the
///   `guard` field of `WingetStep::Set`/`Remove`, `Step::guard_names` returns
///   it, and `execute`'s per-step re-sampler passes it to
///   `Running::covers_any` (`src/execute.rs::execute`).
///
/// Neither list has any other source: `guard_for` reads `bins` or falls back to
/// `winget::guard_names`, and a **winget** row's `bins` is filled only by
/// `winget::rows_to_scan` and by this function. (A scoop row's comes from
/// `scoop::declared_executables`, a third writer this function never touches --
/// `bins` as a field has three, winget's has two.) So a name added here is seen
/// by both fences, and adding it at a second point would be redundant rather
/// than necessary.
///
/// Merging inside `winget::rows_to_scan` instead would mean handing that
/// function a `Config`. It is a pure function of winget's own `list` output and
/// must stay one -- `tests/winget_scan.rs` drives it with rows alone.
///
/// Names are ADDED, never substituted: `winget::guard_names`' two guesses still
/// apply, and a declared name is a third signal beside them, not a replacement
/// for them. Values arrive already folded by `sys::normalize` at parse time
/// (see `config::WingetSection::guard`), so they are directly comparable
/// against `Running`'s `names` and are not folded again here.
///
/// **A key that matches no installed row has two distinguishable causes, and
/// they get different warnings**, because a message naming the wrong one sends
/// the user to fix the wrong thing:
///
/// - **The id is in `scan.opaque`.** Winget reported the package with no source,
///   so `rows_to_scan` gave it no `Installed` row at all -- there is nothing for
///   a guard name to be merged into -- and `plan()` turns it into
///   `SkipReason::Opaque` and `continue`s (`src/plan.rs::plan_backend`) before either
///   fence check is reached. So the guard names genuinely protect nothing, but
///   *not* because nothing is installed. **Measured**, and the ordinary shape
///   rather than an edge case: 84 of 126 ids on a14 were sourceless (see
///   `main.rs`'s `reconcile_ghosts` for that figure and its other costs).
///   Warned whether or not the id is declared, because the package IS there and
///   "not installed yet" is not available as an explanation.
/// - **Otherwise**, and only when the id is not declared in `[winget]
///   packages`: a stale or misspelled entry, protecting nothing in silence.
///   Keyed on `declared` because a declared package that is merely not
///   installed yet is the ordinary state of a fresh machine and must not warn
///   on every run.
///
/// Neither check can live in `config::parse`, which knows the declaration but
/// not the scan.
pub fn apply_guard_overrides(
    outcome: &mut ScanOutcome,
    guard: &BTreeMap<Name, Vec<String>>,
    declared: &[Name],
) -> Vec<String> {
    let ScanOutcome::Scanned(scan) = outcome else {
        // An Unscannable backend established no facts, so nothing here can say
        // whether a key matched -- and there is no `installed` row to merge
        // into either. The same rule `main.rs`'s `reconcile_ghosts` applies to
        // the same outcome, and the same one `winget_fence_ids` above applies.
        return Vec::new();
    };

    let mut warnings = Vec::new();
    for (id, names) in guard {
        let mut matched = false;
        for inst in scan.installed.iter_mut() {
            // Both conditions, and only the name half can fire today.
            // **Structural:** every `ScanOutcome` reaching this function comes
            // from `scan_or_warn(&winget)` -- its two callers are `main.rs`'s
            // `status` arm and `apply::load_everything`, both passing a
            // `Winget` -- so every row here already carries `backend ==
            // WINGET`. `a_scoop_package_of_the_same_name_takes_no_winget_guard_
            // names` has to hand-build an outcome production cannot produce.
            //
            // The backend half is therefore a guard against a future caller
            // that hands this a MERGED both-backend list, not against a hazard
            // live at this call site. That shape is not hypothetical one file
            // over: `apply::guard_for` reads exactly such a list, where the
            // same mistake is live and is pinned by
            // `guard_for_needs_both_the_right_backend_and_the_right_name_not_
            // either_alone`.
            //
            // Compared as `Name`, so pkg.toml's spelling need not match
            // winget's canonical casing.
            if inst.backend != crate::model::WINGET || &inst.name != id {
                continue;
            }
            matched = true;
            for n in names {
                if !inst.bins.contains(n) {
                    inst.bins.push(n.clone());
                }
            }
        }
        if !matched {
            // The opaque case first: it is the one where something IS installed
            // under this id, so the "nothing installed" message below would be
            // false. See this function's own doc comment for both causes.
            if scan.opaque.iter().any(|o| o == id) {
                warnings.push(format!(
                    "pkg.toml [winget.guard] {id}: winget reported this package with no source, \
                     so dotpkg cannot establish its state and skips it before any process check \
                     -- these guard names protect nothing while that is so"
                ));
            } else if !declared.contains(id) {
                warnings.push(format!(
                    "pkg.toml [winget.guard] {id}: nothing installed and nothing declared by that \
                     name, so these guard names protect nothing"
                ));
            }
        }
    }
    warnings
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

    /// One installed winget row, for the guard-merge tests below.
    fn winget_row(id: &str, bins: &[&str]) -> Installed {
        Installed {
            backend: crate::model::WINGET.to_string(),
            name: Name::new(id),
            version: "1.102.2".to_string(),
            arch: None,
            bucket: None,
            bins: bins.iter().map(|b| b.to_string()).collect(),
        }
    }

    /// The `installed` list of a `Scanned` outcome, or a panic naming what the
    /// outcome became instead.
    fn scanned(outcome: &ScanOutcome) -> &[Installed] {
        match outcome {
            ScanOutcome::Scanned(s) => &s.installed,
            ScanOutcome::Unscannable(why) => panic!("outcome changed variant: {why}"),
        }
    }

    #[test]
    fn a_guard_entry_is_merged_into_that_packages_bins() {
        let mut outcome = ScanOutcome::Scanned(Scan {
            installed: vec![winget_row("Tailscale.Tailscale", &["tailscale"])],
            ..Scan::default()
        });
        let mut guard = BTreeMap::new();
        guard.insert(
            Name::new("Tailscale.Tailscale"),
            vec!["tailscaled".to_string(), "tailscale-ipn".to_string()],
        );
        let warnings = apply_guard_overrides(&mut outcome, &guard, &[]);
        assert!(warnings.is_empty(), "warnings were: {warnings:?}");
        // The `guard_names` value survives: this ADDS signals, it does not
        // replace them.
        assert_eq!(
            scanned(&outcome)[0].bins,
            vec![
                "tailscale".to_string(),
                "tailscaled".to_string(),
                "tailscale-ipn".to_string()
            ]
        );
    }

    #[test]
    fn a_guard_key_matches_an_installed_id_by_name_not_by_exact_spelling() {
        // pkg.toml carries whatever spelling the user typed; the scan carries
        // winget's canonical id. Comparing the two as `String` would make a
        // guard entry silently protect nothing on a case difference, which is
        // the failure direction this whole phase exists to close.
        let mut outcome = ScanOutcome::Scanned(Scan {
            installed: vec![winget_row("Tailscale.Tailscale", &[])],
            ..Scan::default()
        });
        let mut guard = BTreeMap::new();
        guard.insert(
            Name::new("tailscale.tailscale"),
            vec!["tailscaled".to_string()],
        );
        let warnings = apply_guard_overrides(&mut outcome, &guard, &[]);
        assert!(warnings.is_empty(), "warnings were: {warnings:?}");
        assert_eq!(scanned(&outcome)[0].bins, vec!["tailscaled".to_string()]);
    }

    #[test]
    fn a_guard_name_already_guessed_by_guard_names_is_not_added_twice() {
        // `[winget.guard] "Brave.Brave" = ["brave"]` names exactly what
        // `winget::guard_names` already guessed. A duplicate would not break
        // matching -- `Running` only asks whether a string is in the set --
        // but `apply::guard_for` copies this list into the `Step`, and a
        // doubled entry there is a phantom nothing in pkg.toml explains.
        let mut outcome = ScanOutcome::Scanned(Scan {
            installed: vec![winget_row("Brave.Brave", &["brave"])],
            ..Scan::default()
        });
        let mut guard = BTreeMap::new();
        guard.insert(Name::new("Brave.Brave"), vec!["brave".to_string()]);
        assert!(apply_guard_overrides(&mut outcome, &guard, &[]).is_empty());
        assert_eq!(scanned(&outcome)[0].bins, vec!["brave".to_string()]);
    }

    #[test]
    fn a_scoop_package_of_the_same_name_takes_no_winget_guard_names() {
        // `[winget.guard]` is keyed by winget id, and the two backends share a
        // namespace only by accident. The same hazard `guard_for_needs_both_the_
        // right_backend_and_the_right_name_not_either_alone` pins in
        // `src/apply.rs`, one table over.
        let mut scoop_row = winget_row("Tailscale.Tailscale", &[]);
        scoop_row.backend = crate::model::SCOOP.to_string();
        let mut outcome = ScanOutcome::Scanned(Scan {
            installed: vec![scoop_row, winget_row("Tailscale.Tailscale", &[])],
            ..Scan::default()
        });
        let mut guard = BTreeMap::new();
        guard.insert(
            Name::new("Tailscale.Tailscale"),
            vec!["tailscaled".to_string()],
        );
        assert!(apply_guard_overrides(&mut outcome, &guard, &[]).is_empty());
        let installed = scanned(&outcome);
        assert!(
            installed[0].bins.is_empty(),
            "the scoop row must be untouched: {:?}",
            installed[0].bins
        );
        assert_eq!(installed[1].bins, vec!["tailscaled".to_string()]);
    }

    #[test]
    fn a_guard_entry_matching_no_installed_package_warns_once() {
        // A stale or misspelled id otherwise protects nothing, in silence.
        // This cannot be a parse error: only this point knows the scan.
        let mut outcome = ScanOutcome::Scanned(Scan::default());
        let mut guard = BTreeMap::new();
        guard.insert(Name::new("Tailscale.Typo"), vec!["tailscaled".to_string()]);
        let warnings = apply_guard_overrides(&mut outcome, &guard, &[]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Tailscale.Typo"), "was: {warnings:?}");
        assert!(warnings[0].contains("[winget.guard]"), "was: {warnings:?}");
    }

    #[test]
    fn a_guard_entry_for_a_declared_but_not_installed_package_does_not_warn() {
        // A machine where the app is merely not installed yet must not print a
        // warning on every run. `declared` is what distinguishes that from a
        // typo.
        let mut outcome = ScanOutcome::Scanned(Scan::default());
        let mut guard = BTreeMap::new();
        guard.insert(
            Name::new("Tailscale.Tailscale"),
            vec!["tailscaled".to_string()],
        );
        let warnings =
            apply_guard_overrides(&mut outcome, &guard, &[Name::new("Tailscale.Tailscale")]);
        assert!(warnings.is_empty(), "was: {warnings:?}");
    }

    #[test]
    fn a_guard_entry_for_an_opaque_package_is_told_the_real_reason_not_not_installed() {
        // A sourceless winget row lands in `opaque`, never in `installed` -- so
        // there is no row for a guard name to be merged into, and `plan()` skips
        // the package before any process check. The names really do protect
        // nothing, but saying "nothing installed" about a package that IS
        // installed sends the user to fix the wrong thing. **Measured:** 84 of
        // 126 ids on a14 were sourceless, so this is the ordinary shape of a
        // winget machine, not a corner.
        //
        // Declared as well, on purpose: unlike the typo case, this warning must
        // NOT be silenced by a declaration. "Not installed yet" cannot explain
        // an id winget just reported.
        let mut outcome = ScanOutcome::Scanned(Scan {
            opaque: vec![Name::new("Tailscale.Tailscale")],
            ..Scan::default()
        });
        let mut guard = BTreeMap::new();
        guard.insert(
            Name::new("Tailscale.Tailscale"),
            vec!["tailscaled".to_string()],
        );
        let warnings =
            apply_guard_overrides(&mut outcome, &guard, &[Name::new("Tailscale.Tailscale")]);
        assert_eq!(warnings.len(), 1, "was: {warnings:?}");
        assert!(
            warnings[0].contains("no source"),
            "name the real cause: {warnings:?}"
        );
        assert!(
            !warnings[0].contains("nothing installed"),
            "must not blame an absence that is not the problem: {warnings:?}"
        );
    }

    #[test]
    fn an_unscannable_winget_backend_yields_no_warning_and_is_left_as_it_was() {
        // Was `..._takes_no_guard_names_and_does_not_warn`, which overstated
        // itself: an `Unscannable` outcome has no `installed` row, so "takes no
        // guard names" is vacuous -- there is nothing to take them into. What is
        // pinnable is the pair below: no warning is invented about a key whose
        // match could not be established, and the outcome itself survives with
        // its cause intact rather than being replaced by an empty `Scanned`.
        //
        // Same rule `main.rs`'s `reconcile_ghosts` and `winget_fence_ids` apply
        // to the same outcome: a backend that established no facts supports no
        // conclusions.
        let mut outcome = ScanOutcome::Unscannable("winget exploded".to_string());
        let mut guard = BTreeMap::new();
        guard.insert(Name::new("Tailscale.Typo"), vec!["x".to_string()]);
        assert!(apply_guard_overrides(&mut outcome, &guard, &[]).is_empty());
        match &outcome {
            ScanOutcome::Unscannable(why) => assert_eq!(why, "winget exploded"),
            ScanOutcome::Scanned(s) => {
                panic!("a failed scan must not be downgraded to an empty one: {s:?}")
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
