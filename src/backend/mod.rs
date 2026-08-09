pub mod scoop;
pub mod winget;

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
/// `bucket::choose_bucket` and `bucket::resolve_latest`. Task 13 moves that
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
}

impl ResolveCtx<'static> {
    /// A context with nothing declared, no lock, and `offline: true` -- for a
    /// caller (today, only a test) that exercises a resolver reading none of
    /// `declared`/`scoop_root`/`old`. Every referenced value is leaked
    /// (`Box::leak`, never freed) rather than held in a `static`: the
    /// `warnings` sink is a `RefCell`, which is not `Sync` and so cannot live
    /// in a `static` at all, and leaking a few small, short-lived test
    /// allocations is cheaper than giving every field its own synchronization
    /// just so one of them could be `Sync`.
    pub fn offline() -> ResolveCtx<'static> {
        let declared: &'static Config = Box::leak(Box::new(Config::default()));
        let scoop_root: &'static Path = Box::leak(PathBuf::from(".").into_boxed_path());
        let old: &'static Lock = Box::leak(Box::new(Lock::default()));
        let warnings: &'static RefCell<Vec<String>> = Box::leak(Box::new(RefCell::new(Vec::new())));
        ResolveCtx {
            offline: true,
            declared,
            scoop_root,
            old,
            warnings,
        }
    }
}

/// One package manager. `scan` reads state that is already on disk or already
/// known; nothing here reaches the network. Mutating methods arrive in Phase 2.
///
/// `resolve_latest` and `resolve_installed` are the seam Phase 4 exists to
/// prove: before Task 13, `update::run` named `bucket::resolve_latest`
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
