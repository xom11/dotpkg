use crate::config::{Config, PkgOpts};
use crate::lock::{Lock, Pin};
use crate::model::{Installed, Name, Running, SCOOP, WINGET};
use crate::state::State;
use std::collections::{BTreeMap, BTreeSet};

/// Scoop installs these itself to unpack other packages and does NOT record
/// that it did: install.json for `dark` is shape-identical to a user-requested
/// package's. No installed manifest declares `depends` either, so there is
/// nothing to infer from and this list has to be explicit.
///
/// Update it if scoop gains a new extraction helper.
pub const SCOOP_HELPERS: &[&str] = &["dark", "innounp", "7zip", "lessmsi"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The package's process is alive. Changing it now risks the running app.
    Running,
    /// Declared in pkg.toml with no pkg.lock entry. `apply` must refuse rather
    /// than resolve a version itself.
    NotLocked,
    /// What a `Capability::ReportsOnly` backend's package would have become,
    /// had dotpkg been able to act on it -- winget scans and plans in Phase
    /// 4, but nothing installs or uninstalls a winget package yet. Reported
    /// rather than dropped, and reported with the diff, not just the fact of
    /// one: the spec's rule is "never degrade silently", `Plan::change_count`
    /// is the one line a user reads before saying yes, and hiding that Brave
    /// is `151.1 -> 151.2` throws away the whole product of `status` for the
    /// sake of a number that would otherwise count a change that will never
    /// happen.
    ReportedOnly(Divergence),
    /// Installed, but the scan could not establish its state (see
    /// `Scan::opaque`). Must not be read as "not installed" -- that reading is
    /// what turned two working, pinned-at-version packages into an
    /// uninstall-then-reinstall under `--yes` on a14.
    Opaque,
}

/// What a `Capability::ReportsOnly` package's declared/installed state would
/// have turned into, had its backend been one Phase 4 can act on. Carried by
/// `SkipReason::ReportedOnly` so the diff a user needs -- both versions, not
/// merely "differs" -- survives all the way to `render` and to
/// `apply::classify`'s `Outcome::Skipped` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Divergence {
    /// Declared, but pkg.lock has no entry at all -- there is not even a
    /// target to diff against yet.
    ///
    /// Deliberately its own `Divergence` variant rather than routed through
    /// `SkipReason::NotLocked`, which is what this used to be before review
    /// caught the regression: `NotLocked` fails the whole run
    /// (`apply::classify` -> `Intent::NotLocked` -> `Outcome::NotLocked` ->
    /// `not_locked_count() > 0` -> `is_ok() == false`) because for a backend
    /// that *Acts*, resolving a version is `update`'s job, not `apply`'s, and
    /// the user must go run it. That reasoning does not transfer to a
    /// `ReportsOnly` backend: `apply` could not act on this package even
    /// *with* a lock entry, so refusing the entire run -- including every
    /// unrelated scoop action in the same plan -- over one that does not
    /// exist yet helps nobody. It is also advice `dotpkg update` cannot
    /// follow today: `update::run` explicitly declines to resolve winget
    /// packages and carries existing pins through untouched, so "run
    /// `dotpkg update`" would be false for exactly the case that reaches
    /// this variant.
    NotLocked,
    /// Declared and locked, but nothing is installed: the version dotpkg
    /// would have installed.
    Install { version: String },
    /// Installed at a version other than the one the lock pins, in either
    /// direction. One shape for both: `plan_backend`'s Upgrade/Downgrade
    /// split is cosmetic display order only (see `is_older`'s own doc
    /// comment), and a backend that cannot act on either has no reason to
    /// carry a distinction that exists purely to pick an arrow.
    Change { from: String, to: String },
    /// Installed, undeclared, and owned by dotpkg: the version that would
    /// have been pruned.
    Prune { version: String },
}

impl Divergence {
    /// The one line a user reads for why dotpkg is not acting on this
    /// package, with the diff embedded. Shared by `render::render` (the
    /// plan) and `apply::classify` (`Outcome::Skipped`'s `why`), so the
    /// explanation reads identically whether it came from `status` or from
    /// `apply`'s preparation table.
    ///
    /// `NotLocked`'s text deliberately does not say "run `dotpkg update`" --
    /// see that variant's own doc comment for why that advice is false here.
    pub fn describe(&self) -> String {
        match self {
            Divergence::NotLocked => "declared, but not in pkg.lock -- reported only, \
                 dotpkg cannot resolve or install a winget package yet"
                .to_string(),
            Divergence::Install { version } => format!(
                "would install {version} -- reported only, dotpkg cannot install or \
                 remove winget packages yet"
            ),
            Divergence::Change { from, to } => format!(
                "{from} -> {to} -- reported only, dotpkg cannot install or remove \
                 winget packages yet"
            ),
            Divergence::Prune { version } => format!(
                "would remove {version} -- reported only, dotpkg cannot install or \
                 remove winget packages yet"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Install {
        backend: String,
        name: Name,
        version: String,
        arch: Option<String>,
    },
    Upgrade {
        backend: String,
        name: Name,
        from: String,
        to: String,
        arch: Option<String>,
    },
    Downgrade {
        backend: String,
        name: Name,
        from: String,
        to: String,
        arch: Option<String>,
    },
    Prune {
        backend: String,
        name: Name,
        version: String,
    },
    Skip {
        backend: String,
        name: Name,
        reason: SkipReason,
    },
    /// Installed, undeclared, and not owned by dotpkg. Reported, never touched.
    Unmanaged {
        backend: String,
        name: Name,
        version: String,
    },
    /// Installed for an architecture other than the one declared. Reported in
    /// Phase 2a and not acted on: fixing it means a reinstall, and that
    /// decision waits for the measured picture from a real machine.
    ArchDrift {
        backend: String,
        name: Name,
        have: String,
        want: String,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<Action>,
}

impl Plan {
    /// How many actions this run will actually perform. Printed in the one
    /// line a user reads before saying yes to `apply` -- so a
    /// `SkipReason::ReportedOnly` (a winget package that differs from the
    /// lock, but that dotpkg cannot act on) is deliberately excluded by
    /// construction: it is `Action::Skip`, never `Install`/`Upgrade`/
    /// `Downgrade`/`Prune`, and counting a change that will never happen
    /// would put a false number in that line -- the defect class Phase 3
    /// fixed twice in `render.rs`. See `skip_count`, which does count it.
    pub fn change_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    Action::Install { .. }
                        | Action::Upgrade { .. }
                        | Action::Downgrade { .. }
                        | Action::Prune { .. }
                )
            })
            .count()
    }

    pub fn skip_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, Action::Skip { .. }))
            .count()
    }

    pub fn drift_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, Action::ArchDrift { .. }))
            .count()
    }
}

/// What a backend's declared-package pass may turn a version difference
/// into. Added by Task 14, when winget got a real view: scoop's is `Acts`
/// (an `Install`/`Upgrade`/`Downgrade`/`Prune` really happens), winget's is
/// `ReportsOnly` (Phase 4 stops at scan/plan/lock/report -- nothing installs
/// or uninstalls a winget package), and `plan_backend` reads this to decide
/// which of the two a difference becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Acts,
    ReportsOnly,
}

/// One backend's slice of the inputs. `plan()` runs the same pass over each
/// backend it has a view for -- both scoop and winget, since Task 14: winget's
/// `capability` is `ReportsOnly`, so the same declared/undeclared logic that
/// would install, upgrade, downgrade or prune for scoop instead reports the
/// diff via `SkipReason::ReportedOnly` for winget, and never touches the
/// machine.
struct BackendView<'a> {
    backend: &'static str,
    declared: &'a [Name],
    lock: &'a BTreeMap<Name, Pin>,
    /// `[scoop.opts]`. Empty for backends that have no per-package options --
    /// winget's `WingetSection` declares none.
    opts: &'a BTreeMap<Name, PkgOpts>,
    /// Names this backend installs for itself and does not record. Empty for
    /// winget, whose equivalent is not a fixed list but the sourceless rows,
    /// which never reach `installed` at all.
    helpers: &'static [&'static str],
    /// Whether a version difference for this backend becomes a real change
    /// or a `SkipReason::ReportedOnly`. See `Capability`'s own doc comment.
    capability: Capability,
}

/// Keeps at most one `Installed` per name for `backend`, first one wins.
/// Keyed by `Name`'s case-folding `Ord`, not by raw strings, so two spellings
/// of one package still collapse to one entry -- the same rule every other
/// name comparison in this crate follows.
///
/// Pulled out of the undeclared loop as its own pure function so this
/// property is unit-testable directly, in every build profile. Going through
/// `plan_backend` instead would also go through its `debug_assert!` just
/// above, which panics on exactly the duplicate-name input this function
/// exists to guard against -- so a test that only called `plan_backend`
/// could observe this property in a release build alone, where the assert is
/// compiled out. See `dedupe_installed_for_backend_keeps_only_the_first_entry_for_a_duplicated_name`.
fn dedupe_installed_for_backend<'a>(
    installed: &'a [Installed],
    backend: &str,
) -> Vec<&'a Installed> {
    let mut acted: BTreeSet<&Name> = BTreeSet::new();
    installed
        .iter()
        .filter(|i| i.backend == backend)
        .filter(|i| acted.insert(&i.name))
        .collect()
}

/// One backend's full pass: declared packages first (install / upgrade /
/// downgrade / skip, plus arch drift), then installed-but-undeclared packages
/// (prune if owned, report if not, ignore helpers). Appends into the shared
/// `actions` / `prunes` / `reports` that `plan()` orders and concatenates
/// once every backend view has run.
#[allow(clippy::too_many_arguments)]
fn plan_backend(
    view: &BackendView,
    installed: &[Installed],
    opaque: &[Name],
    state: &State,
    running: &Running,
    actions: &mut Vec<Action>,
    prunes: &mut Vec<Action>,
    reports: &mut Vec<Action>,
) {
    // Measured: winget's `list` returns `7zip.7zip` twice, with two different
    // versions (`26.01.00.0` and `26.02`), and eight ids in total duplicated
    // on the author's machine. Nothing below can safely pick one of several:
    // the declared loop's `.find()` would silently take the first, and the
    // undeclared loop would emit one Prune per duplicate for what is really
    // one package. A backend's scan is responsible for collapsing duplicates
    // or marking the name opaque before `installed` ever reaches `plan()`;
    // this only catches a violation in development builds. The `acted` guard
    // in the undeclared loop below is the release-build backstop that keeps
    // a violation that slips past this from double-pruning.
    debug_assert!(
        {
            let mut seen = BTreeSet::new();
            installed
                .iter()
                .filter(|i| i.backend == view.backend)
                .all(|i| seen.insert(&i.name))
        },
        "a backend returned two Installed entries for one name; \
         Scan must collapse them or mark them opaque"
    );

    let declared_set: BTreeSet<&Name> = view.declared.iter().collect();

    // Declared packages: install / upgrade / downgrade / skip.
    for name in view.declared {
        let current = installed
            .iter()
            .find(|i| i.backend == view.backend && &i.name == name);

        // Emitted independently of the version verdict, and before the lock
        // check: architecture is a fact about the machine, true whether or not
        // dotpkg knows which version it wants. A package can be both an
        // Upgrade and an ArchDrift; those are two true facts.
        if let (Some(cur), Some(want)) = (
            current,
            view.opts
                .get(name)
                .and_then(|o| o.arch)
                .and_then(|a| a.as_scoop()),
        ) {
            // A missing install.json means "unknown", not "wrong". Older scoop
            // versions did not write one, and reinstalling those on every run
            // would be a bug, not a fix.
            if let Some(have) = cur.arch.as_deref() {
                // Scoop writes lowercase today, but the decision layer must
                // not depend on that staying true: a case-different match
                // here is exactly the kind of comparison this branch exists
                // to remove, and in Phase 2b drift may drive a reinstall.
                if !have.eq_ignore_ascii_case(want) {
                    reports.push(Action::ArchDrift {
                        backend: view.backend.into(),
                        name: name.clone(),
                        have: have.to_string(),
                        want: want.to_string(),
                    });
                }
            }
        }

        if opaque.iter().any(|o| o == name) {
            actions.push(Action::Skip {
                backend: view.backend.into(),
                name: name.clone(),
                reason: SkipReason::Opaque,
            });
            continue;
        }

        let Some(pin) = view.lock.get(name) else {
            // Gated on capability: `NotLocked` fails the whole run (see
            // `Divergence::NotLocked`'s own doc comment for the reasoning),
            // which is correct for a backend that *Acts* -- `update` really
            // can fix it -- but not for one that `ReportsOnly`, where
            // `apply` could not act on this package even with a pin.
            match view.capability {
                Capability::Acts => actions.push(Action::Skip {
                    backend: view.backend.into(),
                    name: name.clone(),
                    reason: SkipReason::NotLocked,
                }),
                Capability::ReportsOnly => actions.push(Action::Skip {
                    backend: view.backend.into(),
                    name: name.clone(),
                    reason: SkipReason::ReportedOnly(Divergence::NotLocked),
                }),
            }
            continue;
        };
        let want = pin.version();

        // Resolved here, not in the executor, so the architecture an install
        // will actually use is visible in the plan the user confirms.
        //
        // Declared wins; otherwise keep what is installed, because
        // reinstalling an undeclared package under a different architecture is
        // a change nobody asked for. `Arch::Keep` yields None, which means
        // "pass no -a".
        let arch: Option<String> = match view.opts.get(name).and_then(|o| o.arch) {
            Some(a) => a.as_scoop().map(str::to_string),
            None => current.and_then(|c| c.arch.clone()),
        };

        match current {
            None => match view.capability {
                Capability::Acts => actions.push(Action::Install {
                    backend: view.backend.into(),
                    name: name.clone(),
                    version: want.to_string(),
                    arch: arch.clone(),
                }),
                // `arch` is dropped here: `Divergence` carries no
                // architecture, matching winget having none to carry --
                // `Installed::arch` is always `None` for it (see
                // `backend::winget::rows_to_scan`'s own doc comment) and
                // `ReportsOnly` never reaches an executor that would need it.
                Capability::ReportsOnly => actions.push(Action::Skip {
                    backend: view.backend.into(),
                    name: name.clone(),
                    reason: SkipReason::ReportedOnly(Divergence::Install {
                        version: want.to_string(),
                    }),
                }),
            },
            Some(cur) if cur.version == want => {}
            Some(cur) => {
                // Checked only once a change is actually called for, so a
                // healthy running package produces no line at all. Applies
                // regardless of `capability`: whether or not dotpkg could
                // act on it, a running process is still the more urgent fact
                // -- `Running`'s own doc comment is about the live app, not
                // about who may change it.
                if running.covers(cur) {
                    actions.push(Action::Skip {
                        backend: view.backend.into(),
                        name: name.clone(),
                        reason: SkipReason::Running,
                    });
                } else {
                    match view.capability {
                        Capability::Acts => {
                            if is_older(&cur.version, want) {
                                actions.push(Action::Upgrade {
                                    backend: view.backend.into(),
                                    name: name.clone(),
                                    from: cur.version.clone(),
                                    to: want.to_string(),
                                    arch: arch.clone(),
                                });
                            } else {
                                actions.push(Action::Downgrade {
                                    backend: view.backend.into(),
                                    name: name.clone(),
                                    from: cur.version.clone(),
                                    to: want.to_string(),
                                    arch: arch.clone(),
                                });
                            }
                        }
                        Capability::ReportsOnly => actions.push(Action::Skip {
                            backend: view.backend.into(),
                            name: name.clone(),
                            reason: SkipReason::ReportedOnly(Divergence::Change {
                                from: cur.version.clone(),
                                to: want.to_string(),
                            }),
                        }),
                    }
                }
            }
        }
    }

    // Installed but undeclared: prune if owned, report if not, ignore helpers.
    //
    // `dedupe_installed_for_backend` is the release-build twin of the
    // `debug_assert!` above: if a backend does hand back two `Installed`
    // entries for one name, this still emits at most one
    // Prune/Skip/Unmanaged for it rather than one per duplicate.
    for inst in dedupe_installed_for_backend(installed, view.backend) {
        if declared_set.contains(&inst.name) {
            continue;
        }
        if state.owns(view.backend, &inst.name) {
            // Ownership outranks the helper list. The list exists to stop a
            // helper scoop installed for itself being reported as a stray; a
            // helper *dotpkg* installed is dotpkg's to release, and skipping
            // it here left it unreleasable and unmentioned forever.
            if running.covers(inst) {
                actions.push(Action::Skip {
                    backend: view.backend.into(),
                    name: inst.name.clone(),
                    reason: SkipReason::Running,
                });
            } else {
                match view.capability {
                    Capability::Acts => prunes.push(Action::Prune {
                        backend: view.backend.into(),
                        name: inst.name.clone(),
                        version: inst.version.clone(),
                    }),
                    // Pushed straight into `actions`, not `prunes`: this is a
                    // report, not a removal dotpkg is about to perform, so it
                    // does not belong in the bucket that install-before-
                    // uninstall ordering exists for. Every other `Skip` this
                    // function emits is pushed the same direct way.
                    Capability::ReportsOnly => actions.push(Action::Skip {
                        backend: view.backend.into(),
                        name: inst.name.clone(),
                        reason: SkipReason::ReportedOnly(Divergence::Prune {
                            version: inst.version.clone(),
                        }),
                    }),
                }
            }
        } else if !view.helpers.contains(&inst.name.key()) {
            reports.push(Action::Unmanaged {
                backend: view.backend.into(),
                name: inst.name.clone(),
                version: inst.version.clone(),
            });
        }
    }
}

/// Pure. No I/O, no network, no subprocess — every input is passed in, which is
/// what lets the whole decision layer be tested on any OS.
pub fn plan(
    declared: &Config,
    lock: &Lock,
    installed: &[Installed],
    opaque: &[Name],
    state: &State,
    running: &Running,
) -> Plan {
    let mut actions = Vec::new();
    let mut prunes = Vec::new();
    let mut reports = Vec::new();

    // One pass per backend, both real since Task 14: scoop `Acts`, winget
    // `ReportsOnly` -- Phase 4 stops at scan/plan/lock/report, so nothing
    // here installs or uninstalls a winget package, but a difference from the
    // lock is still reported, with its diff, via `SkipReason::ReportedOnly`.
    // `[winget.opts]` does not exist, so `empty_opts` stands in for it --
    // `BackendView::opts` still needs *a* `BTreeMap` to borrow, not `None`.
    let empty_opts: BTreeMap<Name, PkgOpts> = BTreeMap::new();
    let backends = [
        BackendView {
            backend: SCOOP,
            declared: declared.scoop.packages.as_slice(),
            lock: &lock.scoop,
            opts: &declared.scoop.opts,
            helpers: SCOOP_HELPERS,
            capability: Capability::Acts,
        },
        BackendView {
            backend: WINGET,
            declared: declared.winget.packages.as_slice(),
            lock: &lock.winget,
            opts: &empty_opts,
            helpers: &[],
            capability: Capability::ReportsOnly,
        },
    ];
    for view in &backends {
        plan_backend(
            view,
            installed,
            opaque,
            state,
            running,
            &mut actions,
            &mut prunes,
            &mut reports,
        );
    }

    // Install before uninstall: a run that dies partway should leave an extra
    // package rather than a missing one.
    actions.extend(prunes);
    actions.extend(reports);
    Plan { actions }
}

/// Dotted numeric comparison, falling back to string order for anything that
/// is not purely numeric. Deliberately not semver: scoop versions include
/// shapes like `26.01` and `2026.07.15.08.55` that semver rejects.
///
/// **Its result is load-bearing only for the arrow `status` prints.** The
/// decision to change a package is made by `cur.version == want` above; this
/// function only picks `Upgrade` vs `Downgrade` for the display. So its edge
/// cases are cosmetic today — but they are not the edge cases this comment
/// used to claim. `parts` keeps **every** numeric run, so `1.0.0-rc1` reduces
/// to `[1,0,0,1]`, not `[1,0,0]`: a prerelease sorts *after* its own release,
/// and the displayed arrow is therefore inverted for suffixed versions.
/// `tests/planner.rs` pins this as a fact rather than leaving it as a claim.
///
/// That stops being true the moment anything *gates* on the distinction: an
/// `apply` that refuses downgrades without `--allow-downgrade`, a policy that
/// skips them, a report that counts them separately. Whoever writes that is
/// promoting this function from cosmetic to load-bearing and owes it a real
/// version comparison — pre-release ordering, non-numeric suffixes, and the
/// `pa.is_empty()` string fallback all become answers a user can be hurt by.
fn is_older(a: &str, b: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    if pa.is_empty() || pb.is_empty() {
        return a < b;
    }
    pa < pb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering_handles_the_shapes_scoop_actually_uses() {
        assert!(is_older("0.74.1", "0.74.2"));
        assert!(!is_older("0.74.2", "0.74.1"));
        // Numeric, not lexical: "0.74.10" is newer than "0.74.9".
        assert!(is_older("0.74.9", "0.74.10"));
        assert!(is_older("26.01", "26.02"));
        assert!(is_older("2026.07.15", "2026.07.29"));
    }

    #[test]
    fn a_version_with_no_digits_at_all_falls_back_to_a_string_comparison() {
        // The branch the function's own doc comment calls out and that
        // nothing exercised until the Task 14 mutation run: scoop manifests
        // do carry versions like `nightly` and `latest`, and every mutant of
        // this fallback survived the whole suite.
        assert!(is_older("nightly", "stable"), "no digits either side");
        assert!(!is_older("stable", "nightly"));

        // Equal is not older -- on both paths. `<=` here would make `apply`
        // reinstall a package that is already at the pinned version, every
        // single run.
        assert!(!is_older("nightly", "nightly"), "the string path");
        assert!(!is_older("1.0.0", "1.0.0"), "the numeric path");

        // Exactly one side has digits, so the numeric comparison has nothing
        // to compare against. Falling through to `[] < [1, 0, 0]` would call
        // every digitless version older than every numbered one -- which for
        // an installed `nightly` against a pinned `1.0.0` is a silent
        // downgrade-shaped reinstall.
        assert!(!is_older("nightly", "1.0.0"), "one side has no digits");
    }

    fn installed(backend: &str, name: &str, version: &str) -> Installed {
        Installed {
            backend: backend.to_string(),
            name: Name::new(name),
            version: version.to_string(),
            arch: None,
            bucket: None,
            bins: Vec::new(),
        }
    }

    #[test]
    fn dedupe_installed_for_backend_keeps_only_the_first_entry_for_a_duplicated_name() {
        // Measured: winget's `list` returns 7zip.7zip twice, with two
        // different versions (26.01.00.0 and 26.02). This is the property
        // that keeps that from becoming two Prune actions for one package,
        // tested directly rather than only through `plan_backend` -- which
        // would also trip its `debug_assert!` on this exact input and be
        // observable only in a release build.
        let all = vec![
            installed(SCOOP, "7zip", "26.01.00.0"),
            installed(SCOOP, "7zip", "26.02"),
        ];
        let kept = dedupe_installed_for_backend(&all, SCOOP);
        assert_eq!(kept.len(), 1, "one package is one entry, got {kept:?}");
        assert_eq!(
            kept[0].version, "26.01.00.0",
            "first entry wins, matching the `.find()` the declared loop uses"
        );
    }

    #[test]
    fn dedupe_installed_for_backend_folds_case_like_every_other_name_comparison() {
        // `Name`'s `Ord` folds case; a dedup keyed on raw strings would keep
        // "FZF" and "fzf" as two separate entries for what is one package.
        let all = vec![installed(SCOOP, "FZF", "1"), installed(SCOOP, "fzf", "2")];
        let kept = dedupe_installed_for_backend(&all, SCOOP);
        assert_eq!(
            kept.len(),
            1,
            "two spellings of one package are one entry, got {kept:?}"
        );
    }

    #[test]
    fn dedupe_installed_for_backend_leaves_other_backends_alone() {
        // A duplicate id on winget must not swallow an unrelated scoop entry
        // of the same name, and an unrequested backend's own duplicates must
        // not be touched by a call scoped to a different one.
        let all = vec![
            installed(SCOOP, "git", "1"),
            installed(WINGET, "git", "2"),
            installed(WINGET, "git", "3"),
        ];
        let kept = dedupe_installed_for_backend(&all, SCOOP);
        assert_eq!(kept.len(), 1, "got {kept:?}");
        assert_eq!(kept[0].backend, SCOOP);
    }

    #[test]
    fn dedupe_installed_for_backend_does_not_touch_distinct_names() {
        // The guard must not turn into an accidental "one entry per backend"
        // truncation -- it only collapses entries that share a name.
        let all = vec![installed(SCOOP, "fzf", "1"), installed(SCOOP, "bat", "1")];
        let kept = dedupe_installed_for_backend(&all, SCOOP);
        assert_eq!(kept.len(), 2, "distinct names must both survive: {kept:?}");
    }
}
