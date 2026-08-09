use crate::config::Config;
use crate::lock::Lock;
use crate::model::{Installed, Name, Running, SCOOP, WINGET};
use crate::state::State;
use std::collections::BTreeSet;

/// Scoop installs these itself to unpack other packages and does NOT record
/// that it did: install.json for `dark` is shape-identical to a user-requested
/// package's. No installed manifest declares `depends` either, so there is
/// nothing to infer from and this list has to be explicit.
///
/// Update it if scoop gains a new extraction helper.
pub const SCOOP_HELPERS: &[&str] = &["dark", "innounp", "7zip", "lessmsi"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The package's process is alive. Changing it now risks the running app.
    Running,
    /// Declared in pkg.toml with no pkg.lock entry. `apply` must refuse rather
    /// than resolve a version itself.
    NotLocked,
    /// Declared for a backend this build cannot act on yet. Reported rather
    /// than dropped: the spec's rule is "never degrade silently", and the whole
    /// product of `status` is the printed plan. A user who follows the spec's
    /// own example pkg.toml and gets `nothing to do` has been lied to.
    BackendNotImplemented,
    /// Installed, but the scan could not establish its state (see
    /// `Scan::opaque`). Must not be read as "not installed" -- that reading is
    /// what turned two working, pinned-at-version packages into an
    /// uninstall-then-reinstall under `--yes` on a14.
    Opaque,
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

    let declared_scoop: BTreeSet<&Name> = declared.scoop.packages.iter().collect();

    // Declared packages: install / upgrade / downgrade / skip.
    for name in &declared.scoop.packages {
        let current = installed
            .iter()
            .find(|i| i.backend == SCOOP && &i.name == name);

        // Emitted independently of the version verdict, and before the lock
        // check: architecture is a fact about the machine, true whether or not
        // dotpkg knows which version it wants. A package can be both an
        // Upgrade and an ArchDrift; those are two true facts.
        if let (Some(cur), Some(want)) = (
            current,
            declared
                .scoop
                .opts
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
                        backend: SCOOP.into(),
                        name: name.clone(),
                        have: have.to_string(),
                        want: want.to_string(),
                    });
                }
            }
        }

        if opaque.iter().any(|o| o == name) {
            actions.push(Action::Skip {
                backend: SCOOP.into(),
                name: name.clone(),
                reason: SkipReason::Opaque,
            });
            continue;
        }

        let Some(pin) = lock.scoop.get(name) else {
            actions.push(Action::Skip {
                backend: SCOOP.into(),
                name: name.clone(),
                reason: SkipReason::NotLocked,
            });
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
        let arch: Option<String> = match declared.scoop.opts.get(name).and_then(|o| o.arch) {
            Some(a) => a.as_scoop().map(str::to_string),
            None => current.and_then(|c| c.arch.clone()),
        };

        match current {
            None => actions.push(Action::Install {
                backend: SCOOP.into(),
                name: name.clone(),
                version: want.to_string(),
                arch: arch.clone(),
            }),
            Some(cur) if cur.version == want => {}
            Some(cur) => {
                // Checked only once a change is actually called for, so a
                // healthy running package produces no line at all.
                if running.covers(cur) {
                    actions.push(Action::Skip {
                        backend: SCOOP.into(),
                        name: name.clone(),
                        reason: SkipReason::Running,
                    });
                } else if is_older(&cur.version, want) {
                    actions.push(Action::Upgrade {
                        backend: SCOOP.into(),
                        name: name.clone(),
                        from: cur.version.clone(),
                        to: want.to_string(),
                        arch: arch.clone(),
                    });
                } else {
                    actions.push(Action::Downgrade {
                        backend: SCOOP.into(),
                        name: name.clone(),
                        from: cur.version.clone(),
                        to: want.to_string(),
                        arch: arch.clone(),
                    });
                }
            }
        }
    }

    // Declared winget packages. There is no winget scan in Phase 1 and none is
    // wanted here — the backend lands in Phase 4. What must not happen in the
    // meantime is silence: dropping these would print `nothing to do` to a user
    // whose pkg.toml declares seventeen of them.
    for name in &declared.winget.packages {
        actions.push(Action::Skip {
            backend: WINGET.into(),
            name: name.clone(),
            reason: SkipReason::BackendNotImplemented,
        });
    }

    // Installed but undeclared: prune if owned, report if not, ignore helpers.
    for inst in installed.iter().filter(|i| i.backend == SCOOP) {
        if declared_scoop.contains(&inst.name) {
            continue;
        }
        if state.owns(SCOOP, &inst.name) {
            // Ownership outranks the helper list. The list exists to stop a
            // helper scoop installed for itself being reported as a stray; a
            // helper *dotpkg* installed is dotpkg's to release, and skipping
            // it here left it unreleasable and unmentioned forever.
            if running.covers(inst) {
                actions.push(Action::Skip {
                    backend: SCOOP.into(),
                    name: inst.name.clone(),
                    reason: SkipReason::Running,
                });
            } else {
                prunes.push(Action::Prune {
                    backend: SCOOP.into(),
                    name: inst.name.clone(),
                    version: inst.version.clone(),
                });
            }
        } else if !SCOOP_HELPERS.contains(&inst.name.key()) {
            reports.push(Action::Unmanaged {
                backend: SCOOP.into(),
                name: inst.name.clone(),
                version: inst.version.clone(),
            });
        }
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
}
