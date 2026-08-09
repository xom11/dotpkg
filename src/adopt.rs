//! `dotpkg adopt` — bringing an already-installed package under management.
//!
//! Reaches no network and changes no installed software. Its whole job is to
//! find the commit whose manifest is the one this machine is actually running,
//! and then to write the three files that make the package managed rather than
//! merely known about.

use crate::bucket;
use crate::model::Name;
use anyhow::Result;
use std::path::Path;

/// Which rule found the commit -- or, for winget, confirmed the pin at all.
/// Reported, because the strength of the evidence differs and a user is
/// entitled to know which one answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matched {
    /// The installed manifest and the bucket blob are the same file. Exact,
    /// and the only rule that can tell two same-version commits apart.
    Content,
    /// Only the version agreed. Weaker: measured, when a bucket amends a
    /// manifest without bumping the version, this picks the newer of the two.
    Version,
    /// Winget has no commit history to search and no local manifest to
    /// compare -- the installed version either still resolves in winget's
    /// own index (`Backend::resolve_installed`) or it does not. Neither
    /// `Content` nor `Version` describes that: both are scoop's two-tier
    /// evidence over a bucket's git history, which winget has no analogue
    /// of at all.
    WingetConfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub commit: String,
    pub version: String,
    pub matched: Matched,
}

/// Find the commit that carries what is installed.
///
/// `Ok(None)` means no commit in this bucket's history for this app carries
/// the installed version -- an ordinary answer while searching, and the caller
/// turns it into a refusal that writes nothing.
///
/// Content is tried across the whole history before version is tried at all,
/// rather than per commit: an exact match anywhere beats an approximate match
/// higher up. Measured, the difference is which of two same-version commits
/// gets pinned, and the version rule picks the wrong one.
pub fn resolve_installed(
    bucket_dir: &Path,
    app: &Name,
    installed_version: &str,
    installed_manifest: &[u8],
    rev: &str,
) -> Result<Option<Found>> {
    let Some(path_in_repo) = bucket::manifest_path(bucket_dir, app, rev) else {
        return Ok(None);
    };
    // --full-history: measured, the default walk hides a version that reached
    // the bucket only on a branch whose change was superseded at merge time.
    let commits = bucket::history(bucket_dir, &path_in_repo, rev)?;
    let blobs = bucket::blobs(bucket_dir, &commits, &path_in_repo)?;

    let want = crate::verify::normalise(installed_manifest);
    for (commit, blob) in commits.iter().zip(blobs.iter()) {
        let Some(body) = blob else { continue };
        if crate::verify::normalise(body) == want {
            return Ok(Some(Found {
                commit: commit.clone(),
                version: blob_version(body).unwrap_or_else(|| installed_version.to_string()),
                matched: Matched::Content,
            }));
        }
    }
    for (commit, blob) in commits.iter().zip(blobs.iter()) {
        let Some(body) = blob else { continue };
        if blob_version(body).as_deref() == Some(installed_version) {
            return Ok(Some(Found {
                commit: commit.clone(),
                version: installed_version.to_string(),
                matched: Matched::Version,
            }));
        }
    }
    Ok(None)
}

fn blob_version(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get("version")?.as_str().map(str::to_string)
}

use crate::backend::scoop::Scoop;
use crate::backend::winget::{Winget, WingetCmd};
use crate::backend::{Backend, ResolveCtx};
use crate::config::Config;
use crate::lock::{Lock, Pin};
use crate::model::{SCOOP, WINGET};
use crate::state::{Ownership, State};
use std::cell::RefCell;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The third element is the version `pkg.lock` pinned for this name
    /// before this run, if any -- `None` when there was no previous entry.
    ///
    /// `adopt_one` refuses when the package is not installed or is already
    /// owned, but not when `pkg.lock` already carries an unowned pin for it
    /// (reachable: hand-write `pkg.toml`, run `update`, then `adopt` to hold
    /// what is actually installed instead of what `update` just resolved).
    /// `run` overwrites that pin unconditionally, so this is the only record
    /// of what was replaced -- without it, `render_adopt` has no way to say
    /// a committed file's prior content is gone, and Phase 3 treats that
    /// silence the same as `Change::RepinnedSameVersion` treats it on the
    /// `update` side: a lie by omission.
    pub adopted: Vec<(Name, Matched, Option<String>)>,
    pub refused: Vec<(Name, String)>,
    /// What `scan` could not read, carried out so the caller can print it.
    ///
    /// A package whose `manifest.json` cannot be read is absent from `scan`,
    /// so `adopt` refuses it with "<name> is not installed" -- which is false,
    /// and, without this, printed with no diagnostic at all. `status`, `apply`
    /// and `update` have each printed these warnings since Phase 2a; `adopt`
    /// was the one command that dropped them on the floor, and the Phase 3
    /// dogfood found it by adopting a package a junction made unreadable.
    pub warnings: Vec<String>,
    /// A write that failed part way through, and which of the three files it
    /// had already changed.
    ///
    /// This used to propagate with `?`, which skipped `render_adopt` entirely:
    /// the user was told `cannot create ...\state.json.tmp1234` and nothing
    /// anywhere said that `pkg.lock` and `pkg.toml` had already been rewritten.
    /// Fatal to the run -- the packages after it are not attempted -- but
    /// reported rather than swallowed.
    pub partial_write: Option<PartialWrite>,
}

/// What a write that stopped part way through left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialWrite {
    pub name: Name,
    /// The files that really were rewritten, in the order they were written.
    /// Never includes the one that failed.
    pub wrote: Vec<&'static str>,
    pub why: String,
}

/// Adopt every named package. Per package it is all or nothing.
///
/// Across packages, a **refusal** is reported and the rest proceed, the same
/// shape as `prepare`. A **write failure** is not a refusal and does not work
/// that way: the three files are re-read at the top of every iteration, so a
/// half-written set makes every later package's guards read a state dotpkg
/// itself does not understand. It stops the run, and it is recorded in
/// `Outcome::partial_write` -- naming which files really did change -- rather
/// than propagating out of a `?` that would skip the report entirely.
///
/// **Write order: `pkg.lock`, then `pkg.toml`, then `state.json`.** Every
/// prefix of that order is inert:
///
/// - lock only: an entry for an undeclared package. `plan()` never reads it
///   and the next whole-run `update` drops it.
/// - lock + `pkg.toml`: declared, locked, and installed at the locked version,
///   so `plan()` emits nothing at all.
/// - all three: adopted.
///
/// The dangerous order is `state.json` first, which makes the package
/// `installed ∧ ¬declared ∧ owned` -- a **prune candidate** (`src/plan.rs`).
/// This mirrors the executor's own reasoning about claiming ownership late.
///
/// **Dispatches on `backend`** (`SCOOP` or `WINGET`, `src/model.rs`) to
/// exactly one of `run_scoop`/`run_winget` below -- an `adopt` invocation
/// names one backend, via `--backend` on the CLI (`SCOOP` by default, for
/// every caller that predates Task 15), and adopts every one of `names` from
/// that backend only. `winget` is threaded through unconditionally, even for
/// a scoop-only call, the same way `apply::load_everything` always builds
/// both backends: constructing a `Winget<C>` spawns nothing by itself, and
/// `run_scoop` never calls any of its methods.
pub fn run<C: WingetCmd>(
    scoop_root: &Path,
    winget: &Winget<C>,
    backend: &str,
    names: &[Name],
    config_path: &Path,
    lock_path: &Path,
    state_path: &Path,
) -> Result<Outcome> {
    match backend {
        SCOOP => run_scoop(scoop_root, names, config_path, lock_path, state_path),
        WINGET => run_winget(winget, names, config_path, lock_path, state_path),
        other => Err(anyhow::anyhow!(
            "{other:?} is not a backend dotpkg knows -- pass \"scoop\" or \"winget\""
        )),
    }
}

fn run_scoop(
    scoop_root: &Path,
    names: &[Name],
    config_path: &Path,
    lock_path: &Path,
    state_path: &Path,
) -> Result<Outcome> {
    let scoop = Scoop::new(scoop_root.to_path_buf());
    let scan = Backend::scan(&scoop)?;
    let mut out = Outcome {
        warnings: scan.warnings.clone(),
        ..Outcome::default()
    };

    for name in names {
        // Re-read all three every iteration: each package's write must land
        // before the next one's guard reads it, or adopting two packages in
        // one command would lose the first.
        let declared = crate::config::load(config_path)?;
        let mut lock = crate::lock::load_or_empty(lock_path)?;
        // No special-casing here: a state.json this cannot read (a directory
        // sitting at that path, a permission denial, corrupt JSON) is a
        // condition dotpkg cannot understand, so the whole package refuses --
        // via this `?` -- before anything is written, rather than proceeding
        // on a guessed-empty ownership record. Defaulting to "nothing owned"
        // here would let `adopt` write pkg.lock and edit pkg.toml on a false
        // belief and discover the problem only at the final `state.save`.
        let mut state = State::load_or_empty(state_path)?;

        match adopt_one(
            scoop_root,
            &scoop,
            &scan,
            &declared,
            &lock,
            &state,
            name,
            config_path,
        ) {
            Err(why) => out.refused.push((name.clone(), why)),
            Ok((pin, matched, config_text, config_changed)) => {
                // Captured before the insert below overwrites it. `adopt_one`
                // refuses on `state.owns` and on "not installed", but not on
                // `pkg.lock` already carrying a pin for this name -- see
                // `Outcome::adopted`'s doc comment.
                let previous_version = lock.scoop.get(name).map(|p| p.version().to_string());
                // `pin` is already the exact `Pin::ScoopCommit` `Backend::
                // resolve_installed` built -- bucket spelled by `key()`,
                // matching `update`: `choose_bucket` opened `buckets/<key>`
                // and `Scoop::stage` opens what the lock says verbatim, so
                // the display spelling would name a directory nothing
                // verified.
                lock.scoop.insert(name.clone(), pin);
                state.set(SCOOP, name, Ownership::Adopted);
                if let Err(failure) = write_in_order(
                    WriteLock(|| crate::lock::save(&lock, lock_path)),
                    WritePkgToml(|| {
                        // Skip the write entirely when `pkg.toml`'s bytes
                        // would not change -- see `adopt_one`'s
                        // `already_declared` comment. `write_in_order` only
                        // counts this as "wrote pkg.toml" when the closure
                        // says it really did.
                        if config_changed {
                            crate::config_edit::save(config_path, &config_text).map(|()| true)
                        } else {
                            Ok(false)
                        }
                    }),
                    WriteState(|| state.save(state_path)),
                ) {
                    out.partial_write = Some(PartialWrite {
                        name: name.clone(),
                        wrote: failure.wrote,
                        why: format!("{:#}", failure.error),
                    });
                    return Ok(out);
                }
                out.adopted.push((name.clone(), matched, previous_version));
            }
        }
    }
    Ok(out)
}

/// `run_scoop`'s twin for winget. Same all-or-nothing-per-package shape, same
/// re-read-everything-per-iteration reasoning, same three-file write order
/// through the same `write_in_order` seam -- only `adopt_one_winget` and
/// which lock map/state backend/`pkg.toml` section get written differ.
fn run_winget<C: WingetCmd>(
    winget: &Winget<C>,
    names: &[Name],
    config_path: &Path,
    lock_path: &Path,
    state_path: &Path,
) -> Result<Outcome> {
    let scan = Backend::scan(winget)?;
    let mut out = Outcome {
        warnings: scan.warnings.clone(),
        ..Outcome::default()
    };

    for name in names {
        let declared = crate::config::load(config_path)?;
        let mut lock = crate::lock::load_or_empty(lock_path)?;
        let mut state = State::load_or_empty(state_path)?;

        match adopt_one_winget(winget, &scan, &declared, &state, name, config_path) {
            Err(why) => out.refused.push((name.clone(), why)),
            Ok((canonical, pin, config_text, config_changed)) => {
                // The canonical-id rule, `adopt`'s half: `update` warns when
                // the spelling it was asked to resolve differs from what
                // winget echoed back (`src/update.rs`); `adopt` must be just
                // as loud, or a user running `adopt --backend winget
                // git.git` sees only "+ winget Git.Git adopted" and is never
                // told the two differ at all. Compared by `Display`
                // (`.to_string()`), not `Name`'s own `Eq`, which folds case
                // and would never see a difference here.
                if canonical.to_string() != name.to_string() {
                    let typed = name.to_string();
                    let matched = canonical.to_string();
                    out.warnings.push(format!(
                        "{typed}: you typed this as {typed:?}, but winget's own listing spells \
                         it {matched:?} -- pkg.lock and state.json record the canonical \
                         spelling; pkg.toml keeps the spelling you typed."
                    ));
                }
                // Looked up by the CANONICAL name -- the key a previous
                // `adopt`/`update` would have written -- not the name this
                // call was given, for the same fold-case reason
                // `resolve_into_lock`'s `previous` lookup in `src/update.rs`
                // works regardless of which case a prior entry's key used.
                let previous_version = lock.winget.get(&canonical).map(|p| p.version().to_string());
                lock.winget.insert(canonical.clone(), pin);
                state.set(WINGET, &canonical, Ownership::Adopted);
                if let Err(failure) = write_in_order(
                    WriteLock(|| crate::lock::save(&lock, lock_path)),
                    WritePkgToml(|| {
                        if config_changed {
                            crate::config_edit::save(config_path, &config_text).map(|()| true)
                        } else {
                            Ok(false)
                        }
                    }),
                    WriteState(|| state.save(state_path)),
                ) {
                    out.partial_write = Some(PartialWrite {
                        name: name.clone(),
                        wrote: failure.wrote,
                        why: format!("{:#}", failure.error),
                    });
                    return Ok(out);
                }
                out.adopted
                    .push((canonical, Matched::WingetConfirmed, previous_version));
            }
        }
    }
    Ok(out)
}

/// One wrapper per write, so the three cannot be passed in the wrong order.
///
/// Without these, `write_in_order` takes three closures of indistinguishable
/// type, positionally, and swapping two of them at the call site compiles and
/// ships. That mistake is exactly the `state.json`-first ordering this
/// module's whole doc comment exists to forbid, and it was **measured** to be
/// invisible: with the arguments reversed, all 175 library tests passed --
/// including both seam tests below, which exercise `write_in_order` with their
/// own recorders and therefore cannot observe what `run` hands it. The only
/// test that caught it was `#[cfg(unix)]`, so on Windows -- this tool's only
/// real target -- the reversal was undetectable.
///
/// Same move `Name` makes in `crate::model`: the type exists so that the wrong
/// thing is not a bug to be caught but a program that cannot be written. It
/// needs no test, runs on every platform, and cannot rot.
struct WriteLock<F>(F);
struct WritePkgToml<F>(F);
struct WriteState<F>(F);

/// The write order itself, behind a seam: lock, then pkg.toml, then
/// state.json, stopping at the first failure. `run` always calls this with
/// closures over the real `lock::save` / `config_edit::save` / `State::save`
/// -- the only reason this exists separately is so the ORDER is directly
/// observable in a test, by injecting closures that record each call, rather
/// than only inferable from what a real interrupted write leaves behind.
///
/// Three properties, held by three different things, deliberately:
///
/// - **Which closure goes in which position** -- held by the wrapper types
///   above, at compile time, on every platform.
/// - **That this function calls them in order and short-circuits** -- held by
///   the two seam tests below, portably.
/// - **That the sequence survives a real interrupted write** -- held by
///   `tests/adopt.rs`'s `a_failed_last_write_leaves_a_prefix_that_plan_does_
///   nothing_about` (`#[cfg(unix)]`, a real filesystem failure).
///
/// The failure carries the prefix that really did land. The error alone names
/// only the file that failed, and "which files did this leave changed" is the
/// one question a user whose `adopt` died half way through actually has.
///
/// `write_pkg_toml` returns `Result<bool>`, not `Result<()>` like the other
/// two: `run` skips the actual `pkg.toml` write when the text would not
/// change (see `adopt_one`'s `already_declared`), and `wrote`'s job is to
/// name what really changed on disk -- listing "pkg.toml" there for a write
/// that never happened would itself be the kind of false line this module
/// exists to avoid, on the (narrow) path where `state.json`'s write fails
/// right after a skipped `pkg.toml` write.
fn write_in_order<L, P, S>(
    write_lock: WriteLock<L>,
    write_pkg_toml: WritePkgToml<P>,
    write_state: WriteState<S>,
) -> std::result::Result<(), WriteFailure>
where
    L: FnOnce() -> Result<()>,
    P: FnOnce() -> Result<bool>,
    S: FnOnce() -> Result<()>,
{
    let mut wrote: Vec<&'static str> = Vec::new();
    if let Err(error) = (write_lock.0)() {
        return Err(WriteFailure { wrote, error });
    }
    wrote.push("pkg.lock");
    match (write_pkg_toml.0)() {
        Err(error) => return Err(WriteFailure { wrote, error }),
        Ok(true) => wrote.push("pkg.toml"),
        Ok(false) => {}
    }
    if let Err(error) = (write_state.0)() {
        return Err(WriteFailure { wrote, error });
    }
    Ok(())
}

/// A write that stopped part way through, and the prefix it left behind.
#[derive(Debug)]
struct WriteFailure {
    wrote: Vec<&'static str>,
    error: anyhow::Error,
}

/// Everything that can refuse, before anything is written. Returns the pieces
/// the caller needs, so no partial state can exist between a check and a write.
///
/// The bucket-choice-then-history-search sequence this used to hold inline is
/// gone: it is exactly what `Scoop::resolve_installed` (`src/backend/
/// scoop.rs`) already does, moved onto the `Backend` trait unchanged by
/// Task 13 but never actually CALLED from here until this rewiring -- which
/// is what made it dead code despite being tested directly. A reviewer
/// line-compared the two before this change and found them equivalent except
/// for which spelling one error message names (`name`, the declared
/// spelling, here; `inst.name`, the scan-derived spelling, in the trait
/// method) -- a cosmetic divergence, reachable only when the user types a
/// different case than what `scan` found on disk, and not "fixed" by picking
/// a side: going through the trait method means that message now carries
/// its wording, which is the whole point of routing through one seam instead
/// of two copies.
#[allow(clippy::too_many_arguments)]
fn adopt_one(
    scoop_root: &Path,
    scoop: &Scoop,
    scan: &crate::backend::Scan,
    declared: &Config,
    lock: &Lock,
    state: &State,
    name: &Name,
    config_path: &Path,
) -> std::result::Result<(Pin, Matched, String, bool), String> {
    let Some(inst) = scan
        .installed
        .iter()
        .find(|i| i.backend == SCOOP && &i.name == name)
    else {
        return Err(format!(
            "{name} is not installed. `adopt` brings an existing package under \
             management; to install one, declare it and run `dotpkg update` then \
             `dotpkg apply`."
        ));
    };
    if state.owns(SCOOP, name) {
        return Err(format!("{name} is already managed by dotpkg"));
    }

    let warnings_sink: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let canonical_sink: RefCell<Option<Name>> = RefCell::new(None);
    let matched_sink: RefCell<Option<Matched>> = RefCell::new(None);
    let ctx = ResolveCtx {
        // `adopt` reaches no network at all -- this module's own top-of-file
        // doc comment -- so `offline: true` is simply a true fact here, not
        // a flag anything downstream reads: `Scoop::resolve_installed` never
        // consults it.
        offline: true,
        declared,
        scoop_root,
        old: lock,
        warnings: &warnings_sink,
        canonical: &canonical_sink,
        matched: &matched_sink,
    };
    let pin = match Backend::resolve_installed(scoop, inst, &ctx) {
        crate::update::Resolution::Resolved { pin } => pin,
        crate::update::Resolution::Failed { why } => return Err(why),
    };
    // `Scoop::resolve_installed` sets this on every `Resolved` it returns
    // (see its own doc comment) -- absent here would mean the trait method
    // itself has a bug, not a state this function should paper over with a
    // guessed default that would misreport the evidence's strength.
    let matched = matched_sink.into_inner().ok_or_else(|| {
        format!(
            "{name}: resolved without recording which rule matched -- an internal \
             inconsistency in Scoop::resolve_installed, not a fact about this package"
        )
    })?;

    // Prepared, not written: the caller writes all three in order only once
    // every refusal above has been passed.
    let text = std::fs::read_to_string(config_path).map_err(|e| format!("{e}"))?;
    let already_declared = declared.scoop.packages.contains(name);
    let config_text = if already_declared {
        text
    } else {
        crate::config_edit::add_scoop_package(&text, name).map_err(|e| format!("{e:#}"))?
    };

    // `already_declared` doubles as "config_text is byte-identical to what is
    // already on disk" -- true exactly when the `if` above took the
    // unedited-`text` branch. The caller uses this to skip a `config_edit::
    // save` that would rewrite `pkg.toml` with the same bytes and leave a
    // `pkg.toml.bak` carrying that same content behind it, for no reason.
    Ok((pin, matched, config_text, !already_declared))
}

/// Everything `adopt`'s winget path needs, before anything is written --
/// `adopt_one`'s twin, one backend over. No bucket, no history, no manifest
/// on disk to read: winget's own index answers the one question that
/// matters, through `Backend::resolve_installed`.
#[allow(clippy::too_many_arguments)]
fn adopt_one_winget<C: WingetCmd>(
    winget: &Winget<C>,
    scan: &crate::backend::Scan,
    declared: &Config,
    state: &State,
    name: &Name,
    config_path: &Path,
) -> std::result::Result<(Name, Pin, String, bool), String> {
    let Some(inst) = scan
        .installed
        .iter()
        .find(|i| i.backend == WINGET && &i.name == name)
    else {
        return Err(format!(
            "{name} is not installed. `adopt` brings an existing package under \
             management; to install one, declare it and run `dotpkg update` then \
             `dotpkg apply`."
        ));
    };
    if state.owns(WINGET, name) {
        return Err(format!("{name} is already managed by dotpkg"));
    }

    // `Winget::resolve_installed` reads none of `declared`/`scoop_root`/
    // `old`/`offline` -- winget has no bucket to choose and `adopt` reaches
    // no network in the first place (this module's own top-of-file doc
    // comment) -- so these three are throwaway values, built fresh per call
    // rather than via `ResolveCtx::offline()`'s LEAKED statics: that
    // constructor exists for callers with no natural lifetime to borrow from
    // (today, only tests), and this function has one -- its own stack frame
    // -- so there is no reason to leak memory on every adopted package.
    let empty_declared = Config::default();
    let empty_lock = Lock::default();
    let warnings_sink: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let canonical_sink: RefCell<Option<Name>> = RefCell::new(None);
    let matched_sink: RefCell<Option<Matched>> = RefCell::new(None);
    let ctx = ResolveCtx {
        offline: true,
        declared: &empty_declared,
        scoop_root: Path::new("."),
        old: &empty_lock,
        warnings: &warnings_sink,
        canonical: &canonical_sink,
        matched: &matched_sink,
    };

    let version = match Backend::resolve_installed(winget, inst, &ctx) {
        crate::update::Resolution::Resolved { pin } => pin.version().to_string(),
        crate::update::Resolution::Failed { why } => return Err(why),
    };
    // The canonical-id rule: `Winget::resolve_installed` reads `inst.name`
    // back to winget (`show --id <inst.name> -v <inst.version>`), and
    // `inst.name` is already whatever `winget list`'s `Id` column printed --
    // canonical by construction, not something this call could get wrong the
    // way a hand-typed `pkg.toml` spelling could. `canonical_sink` is read
    // anyway, for the same reason `update::run` reads it: recording what
    // actually resolved, not assuming it, is the rule this task exists to
    // apply consistently.
    let canonical = canonical_sink
        .into_inner()
        .unwrap_or_else(|| inst.name.clone());
    let pin = Pin::WingetVersion { version };

    let text = std::fs::read_to_string(config_path).map_err(|e| format!("{e}"))?;
    let already_declared = declared.winget.packages.contains(name);
    let config_text = if already_declared {
        text
    } else {
        // The user's own spelling, deliberately -- never `canonical`.
        // `pkg.toml` is the user's file; the canonical-id rule is reported,
        // not silently rewritten into it.
        crate::config_edit::add_winget_package(&text, name).map_err(|e| format!("{e:#}"))?
    };

    Ok((canonical, pin, config_text, !already_declared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// The seam itself, exercised directly and portably: no filesystem, no
    /// `#[cfg(unix)]`, so this runs -- and would catch a regression -- on
    /// Windows, the one platform `a_failed_last_write_leaves_a_prefix_that_
    /// plan_does_nothing_about` (tests/adopt.rs) cannot reach.
    #[test]
    fn write_in_order_calls_lock_then_pkg_toml_then_state_and_propagates_the_last_failure() {
        // Named for what it can actually discriminate. The third write is the
        // last, so this test alone cannot tell "stopped after the failure"
        // from "recorded it and had nothing left to do" -- its sibling below,
        // where the FIRST write fails, is the short-circuit proof.
        let log: RefCell<Vec<&str>> = RefCell::new(Vec::new());
        let result = write_in_order(
            WriteLock(|| {
                log.borrow_mut().push("lock");
                Ok(())
            }),
            WritePkgToml(|| {
                log.borrow_mut().push("pkg.toml");
                Ok(true)
            }),
            WriteState(|| {
                log.borrow_mut().push("state.json");
                anyhow::bail!("state.json write failed")
            }),
        );

        let failure = result.expect_err("the third write's failure must propagate");
        assert_eq!(
            failure.wrote,
            vec!["pkg.lock", "pkg.toml"],
            "the two writes that really landed must be named, and the one that \
             failed must not be: this list is what `render_adopt` tells the user \
             was changed"
        );
        assert_eq!(
            *log.borrow(),
            vec!["lock", "pkg.toml", "state.json"],
            "the recorded order must be exactly lock, then pkg.toml, then \
             state.json -- with the first two recorded (they ran) and the \
             third also recorded (it ran and failed), and nothing after it"
        );
    }

    /// A failure on the FIRST write must stop before the other two ever run
    /// -- the "all or nothing per package" promise, observed through the
    /// same seam rather than only through `Outcome`.
    #[test]
    fn write_in_order_stops_immediately_when_the_first_write_fails() {
        let log: RefCell<Vec<&str>> = RefCell::new(Vec::new());
        let result = write_in_order(
            WriteLock(|| {
                log.borrow_mut().push("lock");
                anyhow::bail!("lock write failed")
            }),
            WritePkgToml(|| {
                log.borrow_mut().push("pkg.toml");
                Ok(true)
            }),
            WriteState(|| {
                log.borrow_mut().push("state.json");
                Ok(())
            }),
        );

        let failure = result.expect_err("the first write's failure must propagate");
        assert!(
            failure.wrote.is_empty(),
            "the write that failed changed nothing, so nothing may be reported as \
             written: {:?}",
            failure.wrote
        );
        assert_eq!(
            *log.borrow(),
            vec!["lock"],
            "pkg.toml and state.json must never have been called"
        );
    }

    /// A skipped `pkg.toml` write (`Ok(false)` -- the package was already
    /// declared, so its text would not change) must still run `write_state`
    /// and must not be named in `wrote` on a later failure: it did not
    /// change anything on disk, so `render_adopt` must not say it did.
    #[test]
    fn a_skipped_pkg_toml_write_still_runs_state_and_is_not_named_as_written() {
        let log: RefCell<Vec<&str>> = RefCell::new(Vec::new());
        let result = write_in_order(
            WriteLock(|| {
                log.borrow_mut().push("lock");
                Ok(())
            }),
            WritePkgToml(|| {
                log.borrow_mut().push("pkg.toml (skipped)");
                Ok(false)
            }),
            WriteState(|| {
                log.borrow_mut().push("state.json");
                anyhow::bail!("state.json write failed")
            }),
        );

        let failure = result.expect_err("the third write's failure must propagate");
        assert_eq!(
            failure.wrote,
            vec!["pkg.lock"],
            "pkg.toml was skipped, not written, so it must not appear here even \
             though state.json failed right after it: {:?}",
            failure.wrote
        );
        assert_eq!(
            *log.borrow(),
            vec!["lock", "pkg.toml (skipped)", "state.json"],
            "the skipped write must still run in its slot -- it decides on its \
             own to do nothing, `write_in_order` does not skip calling it"
        );
    }
}
