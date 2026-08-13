//! `dotpkg update` — the only command that resolves "latest".
//!
//! This module is the decision, not the plumbing: no git, no filesystem, no
//! network. The driver hands it what the buckets said and it produces the new
//! lock plus the diff a user reads.

use crate::lock::{Lock, Pin};
use crate::model::Name;
use std::collections::BTreeMap;

/// What a bucket said about one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The pin this backend resolved. `Pin` is deliberately asymmetric, so a
    /// winget resolution carrying a commit is a compile error rather than a
    /// bug a test has to catch.
    Resolved { pin: Pin },
    /// Per package, never fatal to the run.
    Failed { why: String },
}

/// One line of the diff `update` prints.
///
/// `RepinnedSameVersion` is the variant that exists because the answer to
/// "version or commit" is *both, in different places*: `update` records the
/// new commit, and `apply` -- whose decision is `cur.version == want` -- will
/// do nothing about it. This is the only place a user can see that gap, so it
/// is a named variant rather than folded into `Unchanged`.
///
/// Every variant carries `backend` (`SCOOP` or `WINGET`) since Task 15, which
/// resolves both through one `Vec<Change>`. Before that, every `Change` was a
/// scoop fact by construction and `render_update` hardcoded the word
/// "scoop" into every line it printed; a mixed run needs to say which backend
/// each line is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added {
        backend: &'static str,
        name: Name,
        version: String,
    },
    VersionChanged {
        backend: &'static str,
        name: Name,
        from: String,
        to: String,
    },
    RepinnedSameVersion {
        backend: &'static str,
        name: Name,
        version: String,
    },
    Unchanged {
        backend: &'static str,
        name: Name,
    },
    Dropped {
        backend: &'static str,
        name: Name,
        version: String,
    },
    /// Re-resolution failed. If there was a previous pin, dropping it would
    /// turn a working package into `Skip{NotLocked}`, which makes the next
    /// `apply` refuse the whole run, so it is kept instead.
    ///
    /// `version` is `None` for a brand-new declared package whose FIRST
    /// resolution fails: an ambiguous bucket, a bucket that does not carry
    /// it, or a resolve error. There is no previous pin in that case, so
    /// nothing was "kept" -- `render_update` must not say otherwise. `Option`
    /// rather than an empty string on purpose: an empty string that means
    /// "there was nothing to keep" is exactly the kind of implicit encoding
    /// this codebase avoids everywhere else, and it very nearly let
    /// `render_update` print a false line here.
    Kept {
        backend: &'static str,
        name: Name,
        version: Option<String>,
        why: String,
    },
}

/// Whether this is `dotpkg update` or `dotpkg update <pkg>...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    WholeRun,
    Named(Vec<Name>),
}

impl Scope {
    fn covers(&self, name: &Name) -> bool {
        match self {
            Scope::WholeRun => true,
            Scope::Named(names) => names.contains(name),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Update {
    pub lock: Lock,
    pub changes: Vec<Change>,
}

impl Update {
    pub fn failed_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| matches!(c, Change::Kept { .. }))
            .count()
    }

    /// Whether the new lock differs from the old one at all. `main` uses this
    /// to avoid rewriting a file -- and displacing its `.bak` -- for nothing.
    pub fn wrote_anything(&self) -> bool {
        self.changes
            .iter()
            .any(|c| !matches!(c, Change::Unchanged { .. } | Change::Kept { .. }))
    }
}

/// Fold one backend's resolutions into its own map of a `Lock` under
/// construction, appending to `changes`. Shared by scoop and winget so the
/// "added / version-changed / unchanged / re-pinned / kept / dropped"
/// judgement -- the actual substance of `update` -- is written once and
/// applies identically to both, rather than drifting between two near-copies.
///
/// `canonical` is the one place the two backends genuinely differ: scoop's
/// own resolvers never touch `ResolveCtx::canonical` (nothing renames a scoop
/// package), so the map handed in for scoop is always empty and every entry
/// is stored under the name it was declared with -- unchanged from Phase 3.
/// Winget's is populated per resolved name from what
/// `Winget::resolve_latest` read back out of `Found <name> [<Id>]`, and
/// **that** is the key an entry is stored under, not the spelling `pkg.toml`
/// declared -- the mirror of Phase 3's scoop-bucket fix, pointing the other
/// way: record the thing that actually resolved. `previous` is always looked
/// up by the DECLARED name regardless: `Name`'s `Eq`/`Ord` fold case, so a
/// prior entry stored under a different-case canonical key is still found.
#[allow(clippy::too_many_arguments)]
fn fold_backend(
    backend: &'static str,
    lock_map: &mut BTreeMap<Name, Pin>,
    old_map: &BTreeMap<Name, Pin>,
    declared: &[Name],
    resolutions: &BTreeMap<Name, Resolution>,
    canonical: &BTreeMap<Name, Name>,
    scope: &Scope,
    changes: &mut Vec<Change>,
) {
    for name in declared {
        // `get_key_value`, not `get`: a carried-forward pin must be
        // reinserted under the key the OLD map actually used, which for a
        // winget entry a prior `update`/`adopt` may have already written
        // under its canonical spelling -- not under `name`, the spelling
        // THIS run's `declared` list happens to carry. Task 15 review,
        // Important 1: every branch below except `Resolution::Resolved`
        // used to reinsert under `name.clone()` regardless, so a named
        // `update <unrelated-scoop-package>` run -- touching no winget
        // package at all -- silently rewrote a committed canonical winget
        // key back to whatever pkg.toml happens to spell it as, with no
        // `Change` line and no warning. Pinned three ways: the three
        // `fold_backend_keeps_the_canonical_key_*` unit tests below (one per
        // branch) and `tests/update.rs`'s
        // `a_named_scoop_only_update_does_not_revert_an_existing_winget_
        // lock_entrys_canonical_case`, which reproduces the review's own
        // `dotpkg update fzf` example end to end.
        let previous_entry = old_map.get_key_value(name);
        let previous = previous_entry.map(|(_, p)| p);
        if !scope.covers(name) {
            if let Some((old_key, p)) = previous_entry {
                lock_map.insert(old_key.clone(), p.clone());
            }
            continue;
        }
        match resolutions.get(name) {
            Some(Resolution::Resolved { pin }) => {
                let fresh = pin.clone();
                let version = fresh.version().to_string();
                let key = canonical.get(name).cloned().unwrap_or_else(|| name.clone());
                changes.push(match previous {
                    None => Change::Added {
                        backend,
                        name: name.clone(),
                        version: version.clone(),
                    },
                    Some(p) if *p == fresh => Change::Unchanged {
                        backend,
                        name: name.clone(),
                    },
                    Some(p) if p.version() != version => Change::VersionChanged {
                        backend,
                        name: name.clone(),
                        from: p.version().to_string(),
                        to: version.clone(),
                    },
                    Some(_) => Change::RepinnedSameVersion {
                        backend,
                        name: name.clone(),
                        version: version.clone(),
                    },
                });
                lock_map.insert(key, fresh);
            }
            Some(Resolution::Failed { why }) => {
                changes.push(Change::Kept {
                    backend,
                    name: name.clone(),
                    version: previous.map(|p| p.version().to_string()),
                    why: why.clone(),
                });
                if let Some((old_key, p)) = previous_entry {
                    lock_map.insert(old_key.clone(), p.clone());
                }
            }
            // Not resolved and not failed: the driver never asked about it,
            // which happens for a named run's untouched neighbours. Keep it.
            None => {
                if let Some((old_key, p)) = previous_entry {
                    lock_map.insert(old_key.clone(), p.clone());
                }
            }
        }
    }

    // Entries for packages pkg.toml no longer declares. Only a whole run drops
    // them: `update fzf` must not quietly delete a stale aichat pin the user
    // did not mention.
    for (name, pin) in old_map {
        if declared.contains(name) {
            continue;
        }
        match scope {
            Scope::WholeRun => changes.push(Change::Dropped {
                backend,
                name: name.clone(),
                version: pin.version().to_string(),
            }),
            Scope::Named(_) => {
                lock_map.insert(name.clone(), pin.clone());
            }
        }
    }
}

/// Fold what the buckets said into a new lock, and say what changed.
///
/// Pure. Every git result arrives as a `Resolution`, which is what lets the
/// whole of `update`'s judgement be tested with no repository at all.
///
/// Scoop only: `run` below is the caller that also resolves winget, and it
/// calls `fold_backend` a second time itself (with winget's own declared
/// list, resolutions and canonical-id map) rather than through this function
/// -- kept this way, rather than widened to take both backends' inputs at
/// once, so every one of this function's own tests (all of them scoop-only,
/// several predating Task 15) keeps its exact existing signature and meaning.
pub fn resolve_into_lock(
    old: &Lock,
    declared: &[Name],
    resolutions: &BTreeMap<Name, Resolution>,
    scope: &Scope,
) -> Update {
    // Carrying the winget map through untouched here is deliberate: this
    // function's own callers -- its unit tests, all scoop-only -- must not
    // see winget pins vanish just because this particular fold never touches
    // them. `run` below overwrites `lock.winget` with the real result of its
    // own winget fold immediately after calling this.
    let mut lock = Lock {
        scoop: BTreeMap::new(),
        winget: old.winget.clone(),
    };
    let mut changes = Vec::new();
    fold_backend(
        crate::model::SCOOP,
        &mut lock.scoop,
        &old.scoop,
        declared,
        resolutions,
        &BTreeMap::new(),
        scope,
        &mut changes,
    );
    Update { lock, changes }
}

use crate::backend::scoop::Scoop;
use crate::backend::winget::{Winget, WingetCmd};
use crate::backend::{Backend, ResolveCtx};
use crate::bucket;
use crate::config::Config;
use std::cell::RefCell;
use std::path::Path;

/// Resolve every declared package, scoop and winget alike, against what is
/// actually on disk (scoop) or in winget's own index.
///
/// Returns the decision plus the warnings that belong on stderr. Warnings are
/// returned rather than printed so that this whole function is testable.
///
/// `offline` skips both fetches -- scoop's per-bucket `git fetch` and
/// winget's own `source update`. Everything else about the run is identical,
/// and the caller is told, because "latest" out of an index nobody refreshed
/// is "latest as of whenever something else last pulled it".
pub fn run<C: WingetCmd>(
    scoop_root: &Path,
    winget: &Winget<C>,
    declared: &Config,
    old: &Lock,
    scope: &Scope,
    offline: bool,
) -> (Update, Vec<String>) {
    let mut warnings = Vec::new();

    if offline {
        warnings.push(
            "offline: buckets were not fetched, so `latest` means whatever this \
             machine last pulled."
                .to_string(),
        );
    }
    // The presence check runs whether or not this is an offline run: a
    // declared bucket that is not on this machine is a fact about the machine,
    // not about the network, and it changes what every later resolution can
    // possibly mean. The fetch itself is what `offline` skips.
    for b in &declared.scoop.buckets {
        let dir = scoop_root.join("buckets").join(b.name.key());
        if !dir.join(".git").exists() {
            // Until this, the loop did a bare `continue`. On a fresh machine,
            // or on a pkg.toml that just grew a bucket line, that made EVERY
            // declared package fail with "no declared bucket has it" and the
            // run exit 1, with nothing anywhere saying the real problem was an
            // uncloned bucket.
            warnings.push(format!(
                "bucket {}: declared in pkg.toml but not present at {}. Nothing was \
                 fetched from it and nothing was searched in it -- `dotpkg apply \
                 --clone-missing-buckets` clones it.",
                b.name,
                dir.display()
            ));
            continue;
        }
        if offline {
            continue;
        }
        if bucket::tip(&dir).stale.is_some() {
            warnings.push(format!(
                "bucket {}: no upstream to fetch from, so `latest` is only as \
                 current as this clone.",
                b.name
            ));
            continue;
        }
        if let Err(e) = bucket::fetch(&dir) {
            warnings.push(format!(
                "bucket {}: could not fetch ({e:#}); resolving against what is \
                 already on disk.",
                b.name
            ));
        }
    }

    // The seam Phase 4 exists to prove: this module used to name two
    // `crate::bucket` free functions directly (choosing a bucket, then
    // resolving latest against it), which made "a new backend slots in
    // without touching the planner" a promise the code did not keep.
    // `Scoop`'s own `resolve_latest` (`src/backend/scoop.rs`) holds the exact
    // same logic now -- same precedence, same `--full-history`-free git call,
    // same fallback-to-tip warning, just reached through `Backend` instead of
    // named here. `scoop_root` is passed through `ctx` rather than read off
    // `scoop` itself, so this call resolves against precisely the path this
    // function was given, unaffected by `Scoop::new`'s own
    // root-canonicalisation.
    let scoop = Scoop::new(scoop_root.to_path_buf());
    let fallback_warnings: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let canonical_sink: RefCell<Option<Name>> = RefCell::new(None);
    let matched_sink: RefCell<Option<crate::adopt::Matched>> = RefCell::new(None);
    let ctx = ResolveCtx {
        offline,
        declared,
        scoop_root,
        old,
        warnings: &fallback_warnings,
        canonical: &canonical_sink,
        matched: &matched_sink,
    };

    let mut resolutions = BTreeMap::new();
    for name in &declared.scoop.packages {
        if !scope.covers(name) {
            continue;
        }
        resolutions.insert(name.clone(), scoop.resolve_latest(name, &ctx));
    }

    // Winget's analogue of the per-bucket fetch above: `winget source update
    // --name winget`. Measured inert when scoped this way (
    // `docs/measurements-2026-08-09-winget.md` §9, "Repeated, scoped" -- 141
    // rows before and after, on a machine already through one such update,
    // with the `(Name, Id, Version, Source)` multiset identical and zero
    // `Available`-column moves), unlike the BARE form, which installs
    // winget's own `winget-font` source MSIX and is never used here. Gated on
    // a non-empty `[winget] packages`, the same way the bucket loop above is
    // implicitly gated on `[scoop] buckets`: nothing declared means nothing
    // needs a fresher index this run.
    if !declared.winget.packages.is_empty() {
        if offline {
            warnings.push(
                "offline: winget's index was not refreshed, so `latest` means \
                 whatever this machine last pulled."
                    .to_string(),
            );
        } else {
            match winget.update_source() {
                // The ordinary path says nothing, which is the point: the
                // trigger below was measured at 0 of 10 with no other winget
                // process alive, so a user who is not racing winget sees no
                // new line at all.
                Ok(crate::backend::winget::SourceRefresh::FirstTry) => {}
                // The only thing that has ever been able to observe the retry
                // firing. A successful retry produces no other output by
                // design, so without this line "it never happened" and "it
                // happened and was absorbed" are the same run to a reader --
                // which is exactly why six dogfood rounds settled nothing.
                Ok(crate::backend::winget::SourceRefresh::AfterRetry) => warnings.push(
                    "winget: its index refresh exited 0x8A150001 once (measured to \
                     mean another winget process held the index) and succeeded on \
                     one retry; `latest` was resolved against a refreshed index."
                        .to_string(),
                ),
                Err(e) => warnings.push(format!(
                    "winget: could not refresh its index ({e:#}); resolving against \
                     whatever it already has."
                )),
            }
        }
    }

    let mut winget_resolutions: BTreeMap<Name, Resolution> = BTreeMap::new();
    let mut winget_canonical: BTreeMap<Name, Name> = BTreeMap::new();
    for name in &declared.winget.packages {
        if !scope.covers(name) {
            continue;
        }
        let resolution = winget.resolve_latest(name, &ctx);
        // Read and cleared immediately: `canonical` is a per-call sink (see
        // `ResolveCtx`'s own doc comment), not an accumulator like
        // `warnings`, so it must be drained right after the one call it
        // belongs to and before the next iteration's call can overwrite it.
        if let Some(c) = canonical_sink.borrow_mut().take() {
            let declared_spelling = name.to_string();
            let matched_spelling = c.to_string();
            // **A different id, not a different spelling of the same one.**
            // `Name`'s `Eq` folds case, so this is false for the `git.git` ->
            // `Git.Git` case the warning below exists for, and true only when
            // winget matched something else entirely. It can: `resolve_latest`
            // deliberately omits `--exact` (see its own doc comment -- that is
            // what folds case on the way in), which leaves `--id` a substring
            // filter, so a declared `OhMyPosh` matches
            // `JanDeDobbeleer.OhMyPosh`.
            //
            // Recording it was worse than refusing it. `fold_backend` keys
            // `pkg.lock` by the canonical id while `plan` looks the pin up by
            // the *declared* name, and those two never meet -- so `apply` got
            // `Skip { NotLocked }` and refused the whole run at exit 2, while
            // `update` rewrote the identical unusable lock every time it was
            // run to fix it. Failing the one package says the same thing in a
            // form the user can act on, and leaves the rest of the run alone.
            if c != *name {
                winget_resolutions.insert(
                    name.clone(),
                    Resolution::Failed {
                        why: format!(
                            "winget matched {matched_spelling:?}, not the id pkg.toml declares \
                             ({declared_spelling:?}) -- declare it as {matched_spelling:?}"
                        ),
                    },
                );
                continue;
            }
            if matched_spelling != declared_spelling {
                // The canonical-id rule, the mirror of Phase 3's scoop-bucket
                // fix: `pkg.lock` records what winget actually matched, and a
                // `pkg.toml` whose spelling differs in case is reported, not
                // silently rewritten -- `pkg.toml` is the user's file.
                //
                // Both interpolations below are plain `String`s obtained via
                // `Name`'s `Display` (`to_string()`, already computed above
                // for the comparison), never `Name`'s own `Debug`: that
                // derive dumps its private `display`/`key` fields verbatim
                // (`Name { display: "Git.Git", key: "git.git" }`), which is
                // not a sentence a user should see. Measured by this task's
                // own negative control: a `{name:?}`-based version of this
                // message still happened to contain both spellings as
                // substrings of that struct dump, so a `.contains()`-only
                // test could not have caught the wrong format specifier by
                // itself -- it surfaced only by rerunning the control and
                // reading the panic message it printed.
                warnings.push(format!(
                    "{declared_spelling}: pkg.toml declares this as {declared_spelling:?}, but \
                     winget matches it as {matched_spelling:?} -- pkg.lock records the \
                     canonical spelling; pkg.toml is left as you wrote it."
                ));
            }
            winget_canonical.insert(name.clone(), c);
        }
        winget_resolutions.insert(name.clone(), resolution);
    }
    // Extended only now, after every use of `ctx` (which borrows
    // `fallback_warnings`) in both loops above -- moving this earlier is a
    // borrow-checker error, not merely a style choice, since `ctx` is shared
    // by the scoop loop and the winget loop rather than rebuilt per backend.
    // Per-package fallback warnings from BOTH backends land here, together,
    // ahead of the winget-specific warnings pushed directly above.
    warnings.extend(fallback_warnings.into_inner());

    let mut update = resolve_into_lock(old, &declared.scoop.packages, &resolutions, scope);
    // `resolve_into_lock` seeded `update.lock.winget` with `old.winget`
    // unchanged (see its own doc comment: that is correct for ITS callers,
    // all scoop-only). Cleared here so `fold_backend` rebuilds it from
    // scratch, the same starts-empty contract `lock.scoop` gets inside
    // `resolve_into_lock` itself -- otherwise a genuinely dropped winget
    // entry (declared once, no longer declared, `Scope::WholeRun`) would
    // survive in the carried-through copy even though `fold_backend` never
    // re-inserts it.
    update.lock.winget = BTreeMap::new();
    fold_backend(
        crate::model::WINGET,
        &mut update.lock.winget,
        &old.winget,
        &declared.winget.packages,
        &winget_resolutions,
        &winget_canonical,
        scope,
        &mut update.changes,
    );

    (update, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::Pin;

    fn sha(c: char) -> String {
        std::iter::repeat_n(c, 40).collect()
    }
    fn locked(bucket: &str, commit: char, version: &str) -> Pin {
        Pin::ScoopCommit {
            bucket: bucket.into(),
            commit: sha(commit),
            version: version.into(),
        }
    }
    fn resolved(bucket: &str, commit: char, version: &str) -> Resolution {
        Resolution::Resolved {
            pin: locked(bucket, commit, version),
        }
    }
    fn lock_of(entries: &[(&str, Pin)]) -> Lock {
        let mut l = Lock::default();
        for (n, p) in entries {
            l.scoop.insert(Name::new(*n), p.clone());
        }
        l
    }
    fn res(entries: &[(&str, Resolution)]) -> BTreeMap<Name, Resolution> {
        entries
            .iter()
            .map(|(n, r)| (Name::new(*n), r.clone()))
            .collect()
    }

    #[test]
    fn a_package_with_no_previous_entry_is_added() {
        let u = resolve_into_lock(
            &Lock::default(),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'a', "0.74.2"))]),
            &Scope::WholeRun,
        );
        assert_eq!(
            u.changes,
            vec![Change::Added {
                backend: crate::model::SCOOP,
                name: Name::new("fzf"),
                version: "0.74.2".into()
            }]
        );
        assert_eq!(u.lock.scoop.len(), 1);
    }

    #[test]
    fn a_new_version_is_reported_as_a_version_change() {
        let u = resolve_into_lock(
            &lock_of(&[("fzf", locked("main", 'a', "0.74.1"))]),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'b', "0.74.2"))]),
            &Scope::WholeRun,
        );
        assert_eq!(
            u.changes,
            vec![Change::VersionChanged {
                backend: crate::model::SCOOP,
                name: Name::new("fzf"),
                from: "0.74.1".into(),
                to: "0.74.2".into()
            }]
        );
    }

    #[test]
    fn the_same_version_at_a_new_commit_is_a_repin_and_says_so() {
        // The answer to "does update converge by version or by commit", in one
        // test. It converges by COMMIT when it writes -- the new commit really
        // is recorded -- and `apply` converges by VERSION when it acts, so
        // this line is the only place a user can see the gap.
        let u = resolve_into_lock(
            &lock_of(&[("fzf", locked("main", 'a', "0.74.1"))]),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'b', "0.74.1"))]),
            &Scope::WholeRun,
        );
        assert_eq!(
            u.changes,
            vec![Change::RepinnedSameVersion {
                backend: crate::model::SCOOP,
                name: Name::new("fzf"),
                version: "0.74.1".into()
            }]
        );
        // And the commit really moved. A "report it and keep the old pin"
        // implementation would pass the assertion above and silently make the
        // lock a lie.
        match &u.lock.scoop[&Name::new("fzf")] {
            Pin::ScoopCommit { commit, .. } => assert_eq!(*commit, sha('b')),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_identical_resolution_is_unchanged_and_not_a_repin() {
        let u = resolve_into_lock(
            &lock_of(&[("fzf", locked("main", 'a', "0.74.1"))]),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'a', "0.74.1"))]),
            &Scope::WholeRun,
        );
        assert_eq!(
            u.changes,
            vec![Change::Unchanged {
                backend: crate::model::SCOOP,
                name: Name::new("fzf")
            }]
        );
    }

    #[test]
    fn a_package_no_longer_declared_is_dropped_on_a_whole_run() {
        let u = resolve_into_lock(
            &lock_of(&[
                ("fzf", locked("main", 'a', "0.74.1")),
                ("aichat", locked("main", 'c', "0.30.0")),
            ]),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'a', "0.74.1"))]),
            &Scope::WholeRun,
        );
        assert!(u.changes.contains(&Change::Dropped {
            backend: crate::model::SCOOP,
            name: Name::new("aichat"),
            version: "0.30.0".into()
        }));
        assert!(!u.lock.scoop.contains_key(&Name::new("aichat")));
    }

    #[test]
    fn a_named_run_touches_only_what_it_was_asked_about_and_drops_nothing() {
        // `update fzf` must not rewrite bat's pin, and must not drop a stale
        // aichat entry the user did not mention.
        let old = lock_of(&[
            ("fzf", locked("main", 'a', "0.74.1")),
            ("bat", locked("main", 'c', "0.26.0")),
            ("aichat", locked("main", 'd', "0.30.0")),
        ]);
        let u = resolve_into_lock(
            &old,
            &[Name::new("fzf"), Name::new("bat")],
            // `bat` is given a resolution that differs from its existing pin,
            // even though the scope does not name it. A `Scope::covers` that
            // wrongly returned `true` for `bat` would let this resolution
            // through and rewrite it -- without this second resolution, the
            // "not covered, keep" and "covered but unresolved, keep" branches
            // produce byte-identical output and the mutation is invisible.
            &res(&[
                ("fzf", resolved("main", 'b', "0.74.2")),
                ("bat", resolved("main", 'e', "0.27.0")),
            ]),
            &Scope::Named(vec![Name::new("fzf")]),
        );
        assert_eq!(
            u.lock.scoop[&Name::new("bat")],
            old.scoop[&Name::new("bat")]
        );
        assert!(
            u.lock.scoop.contains_key(&Name::new("aichat")),
            "a named run drops nothing"
        );
        assert_eq!(u.changes.len(), 1, "only fzf is reported: {:?}", u.changes);
    }

    #[test]
    fn a_failed_reresolve_keeps_the_previous_entry_rather_than_dropping_it() {
        // Dropping it would turn a package that works today into
        // Skip{NotLocked}, which makes the NEXT apply refuse the whole run.
        // The failure is per package; the pin that already worked survives.
        let old = lock_of(&[("zellij", locked("extras", 'a', "0.44.3"))]);
        let u = resolve_into_lock(
            &old,
            &[Name::new("zellij")],
            &res(&[(
                "zellij",
                Resolution::Failed {
                    why: "bucket \"extras\" has no zellij.json".into(),
                },
            )]),
            &Scope::WholeRun,
        );
        assert_eq!(
            u.lock.scoop[&Name::new("zellij")],
            old.scoop[&Name::new("zellij")],
            "the previous pin must survive a failed re-resolve"
        );
        assert_eq!(
            u.changes,
            vec![Change::Kept {
                backend: crate::model::SCOOP,
                name: Name::new("zellij"),
                version: Some("0.44.3".into()),
                why: "bucket \"extras\" has no zellij.json".into()
            }]
        );
        assert_eq!(u.failed_count(), 1);
    }

    #[test]
    fn a_failed_reresolve_for_a_package_that_had_no_entry_adds_nothing() {
        let u = resolve_into_lock(
            &Lock::default(),
            &[Name::new("new")],
            &res(&[(
                "new",
                Resolution::Failed {
                    why: "no declared bucket has it".into(),
                },
            )]),
            &Scope::WholeRun,
        );
        assert!(
            u.lock.scoop.is_empty(),
            "nothing to keep, so nothing is written"
        );
        assert_eq!(u.failed_count(), 1);
        match &u.changes[0] {
            Change::Kept { why, version, .. } => {
                assert!(why.contains("no declared bucket"));
                // There was no previous entry, so there is nothing to keep --
                // `render_update` reads exactly this field to decide whether
                // it may say "kept the previous pin".
                assert_eq!(
                    *version, None,
                    "nothing was kept: there was no previous pin"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    // -- fold_backend: the routing item 1 of Task 15's brief names ------
    //
    // `resolve_into_lock`'s own tests above never pass a non-empty
    // `canonical` map, so none of them can tell "stored under the declared
    // name" apart from "stored under a canonical override" -- that
    // distinction is `fold_backend`'s alone, and these pin it directly,
    // below the level of any particular backend.

    #[test]
    fn fold_backend_stores_a_resolved_entry_under_its_canonical_key_when_one_is_given() {
        let mut lock_map = BTreeMap::new();
        let mut changes = Vec::new();
        let mut canonical = BTreeMap::new();
        canonical.insert(Name::new("git.git"), Name::new("Git.Git"));

        fold_backend(
            crate::model::WINGET,
            &mut lock_map,
            &BTreeMap::new(),
            &[Name::new("git.git")],
            &res(&[(
                "git.git",
                Resolution::Resolved {
                    pin: Pin::WingetVersion {
                        version: "2.55.0".into(),
                    },
                },
            )]),
            &canonical,
            &Scope::WholeRun,
            &mut changes,
        );

        assert_eq!(lock_map.len(), 1, "not stored under both spellings");
        let (stored_key, _) = lock_map.get_key_value(&Name::new("git.git")).unwrap();
        assert_eq!(
            stored_key.to_string(),
            "Git.Git",
            "the map records the canonical spelling, not the declared one: {:?}",
            lock_map.keys().collect::<Vec<_>>()
        );
        match &changes[0] {
            Change::Added { backend, name, .. } => {
                assert_eq!(*backend, crate::model::WINGET);
                // The reported NAME is still the declared spelling -- only
                // the map key changes. `render_update` reads the change list,
                // and "git.git added" is what the user typed and expects to
                // see, not a spelling correction sprung on them mid-report.
                assert_eq!(name.to_string(), "git.git");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fold_backend_stores_under_the_declared_name_when_no_canonical_override_is_given() {
        // The positive control: an empty `canonical` map -- what every scoop
        // call passes -- must leave the ordinary declared-name key alone.
        let mut lock_map = BTreeMap::new();
        let mut changes = Vec::new();

        fold_backend(
            crate::model::SCOOP,
            &mut lock_map,
            &BTreeMap::new(),
            &[Name::new("fzf")],
            &res(&[("fzf", resolved("main", 'a', "0.74.2"))]),
            &BTreeMap::new(),
            &Scope::WholeRun,
            &mut changes,
        );

        assert!(lock_map.contains_key(&Name::new("fzf")));
        match &changes[0] {
            Change::Added { backend, .. } => assert_eq!(*backend, crate::model::SCOOP),
            other => panic!("{other:?}"),
        }
    }

    // -- fold_backend: the canonical key must survive every "carry
    // forward" branch, not just Resolution::Resolved (review, Important 1)
    //
    // `fold_backend`'s Resolved branch correctly stores under `canonical`,
    // but every OTHER branch used to reinsert a carried-forward pin under
    // `name.clone()` -- the DECLARED spelling -- discarding whatever
    // canonical key the OLD map actually used. A same-case fixture cannot
    // discriminate this at all: `Name`'s `Eq`/`Ord` fold case, so
    // `old_map.get(name)` finds the entry either way, and if `old_map`'s own
    // key and `name` stringify identically there is nothing for `stored_key
    // .to_string()` to catch. Every test below deliberately uses "Git.Git"
    // in the old map and "git.git" as the declared spelling -- the two
    // must produce different `to_string()`s for the assertion to mean
    // anything.

    fn winget_canonical_fixture() -> (Name, Name, BTreeMap<Name, Pin>) {
        let canonical = Name::new("Git.Git");
        let declared_spelling = Name::new("git.git");
        let mut old_map = BTreeMap::new();
        old_map.insert(
            canonical.clone(),
            Pin::WingetVersion {
                version: "2.55.0".into(),
            },
        );
        (canonical, declared_spelling, old_map)
    }

    #[test]
    fn fold_backend_keeps_the_canonical_key_for_an_entry_outside_the_named_scope() {
        // The exact shape the review's concrete example reproduces: an
        // unrelated named run (`update fzf`) that does not cover this
        // winget package at all must not touch its key.
        let (canonical, declared_spelling, old_map) = winget_canonical_fixture();
        let mut lock_map = BTreeMap::new();
        let mut changes = Vec::new();

        fold_backend(
            crate::model::WINGET,
            &mut lock_map,
            &old_map,
            std::slice::from_ref(&declared_spelling),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &Scope::Named(vec![]), // covers nothing
            &mut changes,
        );

        assert_eq!(lock_map.len(), 1, "not stored under both spellings");
        let (stored_key, _) = lock_map.get_key_value(&declared_spelling).unwrap();
        assert_eq!(
            stored_key.to_string(),
            canonical.to_string(),
            "an out-of-scope carry-forward must keep the canonical key: {:?}",
            lock_map.keys().collect::<Vec<_>>()
        );
        assert!(
            changes.is_empty(),
            "nothing outside the scope is reported as changed: {changes:?}"
        );
    }

    #[test]
    fn fold_backend_keeps_the_canonical_key_when_re_resolution_fails() {
        let (canonical, declared_spelling, old_map) = winget_canonical_fixture();
        let mut lock_map = BTreeMap::new();
        let mut changes = Vec::new();
        let resolutions = res(&[(
            "git.git",
            Resolution::Failed {
                why: "no longer in the winget index".into(),
            },
        )]);

        fold_backend(
            crate::model::WINGET,
            &mut lock_map,
            &old_map,
            std::slice::from_ref(&declared_spelling),
            &resolutions,
            &BTreeMap::new(),
            &Scope::WholeRun,
            &mut changes,
        );

        let (stored_key, _) = lock_map.get_key_value(&declared_spelling).unwrap();
        assert_eq!(
            stored_key.to_string(),
            canonical.to_string(),
            "a failed re-resolve's carried-forward pin must keep the canonical \
             key: {:?}",
            lock_map.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn fold_backend_keeps_the_canonical_key_when_the_driver_never_asked() {
        // Covered by scope but absent from `resolutions` entirely --
        // unreachable through `update::run` today (its own winget loop
        // inserts a resolution for every in-scope name), but `fold_backend`
        // is a shared, general fold and its contract must hold regardless
        // of which caller reaches this branch.
        let (canonical, declared_spelling, old_map) = winget_canonical_fixture();
        let mut lock_map = BTreeMap::new();
        let mut changes = Vec::new();

        fold_backend(
            crate::model::WINGET,
            &mut lock_map,
            &old_map,
            std::slice::from_ref(&declared_spelling),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &Scope::WholeRun,
            &mut changes,
        );

        let (stored_key, _) = lock_map.get_key_value(&declared_spelling).unwrap();
        assert_eq!(
            stored_key.to_string(),
            canonical.to_string(),
            "an unresolved carry-forward must keep the canonical key: {:?}",
            lock_map.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn winget_entries_survive_a_scoop_only_resolve_into_lock_call_untouched() {
        // `resolve_into_lock` itself is scoop-only (see its own doc comment);
        // `run` is what folds winget in, by calling `fold_backend` a second
        // time for it. Calling `resolve_into_lock` directly, as every other
        // test in this module does, must still carry an existing winget map
        // through unchanged rather than dropping it.
        let mut old = Lock::default();
        old.winget.insert(
            Name::new("Git.Git"),
            Pin::WingetVersion {
                version: "2.55.0".into(),
            },
        );
        let u = resolve_into_lock(&old, &[], &BTreeMap::new(), &Scope::WholeRun);
        assert_eq!(u.lock.winget, old.winget);
    }

    // -- wrote_anything --------------------------------------------------
    //
    // What this protects names its own failure consequence: get it wrong and
    // `update` rewrites pkg.lock, and displaces its `.bak`, on every run of
    // an already-converged machine. The three below call it directly rather
    // than through `resolve_into_lock`, so a future change to that fold
    // cannot make them pass for the wrong reason.

    #[test]
    fn wrote_anything_is_false_when_every_change_is_unchanged() {
        let u = Update {
            lock: Lock::default(),
            changes: vec![
                Change::Unchanged {
                    backend: crate::model::SCOOP,
                    name: Name::new("fzf"),
                },
                Change::Unchanged {
                    backend: crate::model::SCOOP,
                    name: Name::new("bat"),
                },
            ],
        };
        assert!(
            !u.wrote_anything(),
            "an already-converged run must not ask for a rewrite"
        );
    }

    #[test]
    fn wrote_anything_is_true_when_a_change_is_added() {
        let u = Update {
            lock: Lock::default(),
            changes: vec![
                Change::Unchanged {
                    backend: crate::model::SCOOP,
                    name: Name::new("fzf"),
                },
                Change::Added {
                    backend: crate::model::SCOOP,
                    name: Name::new("bat"),
                    version: "0.26.1".into(),
                },
            ],
        };
        assert!(
            u.wrote_anything(),
            "a genuinely new pin must ask for a rewrite"
        );
    }

    #[test]
    fn wrote_anything_is_false_when_the_only_change_is_kept() {
        // Kept means re-resolution failed and the previous pin was carried
        // forward byte-for-byte. Nothing about the lock actually changed, so
        // rewriting it -- and displacing its .bak -- for this alone would be
        // exactly the failure this function exists to prevent.
        let u = Update {
            lock: Lock::default(),
            changes: vec![Change::Kept {
                backend: crate::model::SCOOP,
                name: Name::new("zellij"),
                version: Some("0.44.3".into()),
                why: "bucket \"extras\" has no zellij.json".into(),
            }],
        };
        assert!(
            !u.wrote_anything(),
            "a failed re-resolve that changed nothing must not ask for a rewrite"
        );
    }
}
