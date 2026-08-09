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
    Resolved {
        bucket: String,
        commit: String,
        version: String,
    },
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added {
        name: Name,
        version: String,
    },
    VersionChanged {
        name: Name,
        from: String,
        to: String,
    },
    RepinnedSameVersion {
        name: Name,
        version: String,
    },
    Unchanged {
        name: Name,
    },
    Dropped {
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

/// Fold what the buckets said into a new lock, and say what changed.
///
/// Pure. Every git result arrives as a `Resolution`, which is what lets the
/// whole of `update`'s judgement be tested with no repository at all.
pub fn resolve_into_lock(
    old: &Lock,
    declared: &[Name],
    resolutions: &BTreeMap<Name, Resolution>,
    scope: &Scope,
) -> Update {
    // Phase 3 resolves scoop only. Carrying the winget map through untouched
    // is deliberate: dropping pins this command cannot resolve would delete
    // what Phase 4 needs.
    let mut lock = Lock {
        scoop: BTreeMap::new(),
        winget: old.winget.clone(),
    };
    let mut changes = Vec::new();

    for name in declared {
        let previous = old.scoop.get(name);
        if !scope.covers(name) {
            if let Some(p) = previous {
                lock.scoop.insert(name.clone(), p.clone());
            }
            continue;
        }
        match resolutions.get(name) {
            Some(Resolution::Resolved {
                bucket,
                commit,
                version,
            }) => {
                let fresh = Pin::ScoopCommit {
                    bucket: bucket.clone(),
                    commit: commit.clone(),
                    version: version.clone(),
                };
                changes.push(match previous {
                    None => Change::Added {
                        name: name.clone(),
                        version: version.clone(),
                    },
                    Some(p) if *p == fresh => Change::Unchanged { name: name.clone() },
                    Some(p) if p.version() != version => Change::VersionChanged {
                        name: name.clone(),
                        from: p.version().to_string(),
                        to: version.clone(),
                    },
                    Some(_) => Change::RepinnedSameVersion {
                        name: name.clone(),
                        version: version.clone(),
                    },
                });
                lock.scoop.insert(name.clone(), fresh);
            }
            Some(Resolution::Failed { why }) => {
                changes.push(Change::Kept {
                    name: name.clone(),
                    version: previous.map(|p| p.version().to_string()),
                    why: why.clone(),
                });
                if let Some(p) = previous {
                    lock.scoop.insert(name.clone(), p.clone());
                }
            }
            // Not resolved and not failed: the driver never asked about it,
            // which happens for a named run's untouched neighbours. Keep it.
            None => {
                if let Some(p) = previous {
                    lock.scoop.insert(name.clone(), p.clone());
                }
            }
        }
    }

    // Entries for packages pkg.toml no longer declares. Only a whole run drops
    // them: `update fzf` must not quietly delete a stale aichat pin the user
    // did not mention.
    for (name, pin) in &old.scoop {
        if declared.contains(name) {
            continue;
        }
        match scope {
            Scope::WholeRun => changes.push(Change::Dropped {
                name: name.clone(),
                version: pin.version().to_string(),
            }),
            Scope::Named(_) => {
                lock.scoop.insert(name.clone(), pin.clone());
            }
        }
    }

    Update { lock, changes }
}

use crate::bucket::{self, BucketChoice};
use crate::config::Config;
use std::path::Path;

/// Resolve every declared scoop package against the buckets on disk.
///
/// Returns the decision plus the warnings that belong on stderr. Warnings are
/// returned rather than printed so that this whole function is testable.
///
/// `offline` skips the fetch. Everything else about the run is identical, and
/// the caller is told, because "latest" out of a bucket nobody fetched is
/// "latest as of whenever something else last pulled it".
pub fn run(
    scoop_root: &Path,
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
    } else {
        for b in &declared.scoop.buckets {
            let dir = scoop_root.join("buckets").join(b.name.key());
            if !dir.join(".git").exists() {
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
    }

    let mut resolutions = BTreeMap::new();
    for name in &declared.scoop.packages {
        if !scope.covers(name) {
            continue;
        }
        let already = old.scoop.get(name).and_then(|p| match p {
            Pin::ScoopCommit { bucket, .. } => Some(bucket.as_str()),
            Pin::WingetVersion { .. } => None,
        });
        let resolution = match bucket::choose_bucket(scoop_root, declared, name, already) {
            BucketChoice::Ambiguous { candidates } => {
                let names: Vec<String> = candidates.iter().map(|c| c.to_string()).collect();
                Resolution::Failed {
                    why: format!(
                        "{} declared buckets carry it ({}). Say which with \
                         `[scoop.opts] {name} = {{ bucket = \"...\" }}`.",
                        candidates.len(),
                        names.join(", ")
                    ),
                }
            }
            BucketChoice::NotFound { searched } => {
                let names: Vec<String> = searched.iter().map(|s| s.to_string()).collect();
                Resolution::Failed {
                    why: format!("no declared bucket has it (searched: {})", names.join(", ")),
                }
            }
            BucketChoice::Chosen {
                name: bucket_name,
                dir,
                tip,
            } => match bucket::resolve_latest(&dir, name, &tip.rev) {
                Ok(Some(latest)) => {
                    if latest.fell_back_to_tip {
                        warnings.push(format!(
                            "{name}: no single commit carries this manifest's current \
                             content, so the bucket tip was pinned instead."
                        ));
                    }
                    Resolution::Resolved {
                        bucket: bucket_name.to_string(),
                        commit: latest.commit,
                        version: latest.version,
                    }
                }
                Ok(None) => Resolution::Failed {
                    why: format!("bucket {bucket_name} has no manifest for it"),
                },
                Err(e) => Resolution::Failed {
                    why: format!("{e:#}"),
                },
            },
        };
        resolutions.insert(name.clone(), resolution);
    }

    if !declared.winget.packages.is_empty() {
        warnings.push(format!(
            "{} winget package(s) were not resolved: the winget backend lands in \
             phase 4. Their existing pins are untouched.",
            declared.winget.packages.len()
        ));
    }

    (
        resolve_into_lock(old, &declared.scoop.packages, &resolutions, scope),
        warnings,
    )
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
            bucket: bucket.into(),
            commit: sha(commit),
            version: version.into(),
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

    #[test]
    fn winget_entries_survive_a_scoop_update_untouched() {
        // Phase 3 resolves scoop only. Dropping the winget map because this
        // command cannot resolve it would delete pins Phase 4 is going to need.
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
                    name: Name::new("fzf"),
                },
                Change::Unchanged {
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
                    name: Name::new("fzf"),
                },
                Change::Added {
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
