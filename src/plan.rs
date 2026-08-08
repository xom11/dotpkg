use crate::config::Config;
use crate::lock::Lock;
use crate::model::{Installed, SCOOP, WINGET};
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Install {
        backend: String,
        name: String,
        version: String,
    },
    Upgrade {
        backend: String,
        name: String,
        from: String,
        to: String,
    },
    Downgrade {
        backend: String,
        name: String,
        from: String,
        to: String,
    },
    Prune {
        backend: String,
        name: String,
        version: String,
    },
    Skip {
        backend: String,
        name: String,
        reason: SkipReason,
    },
    /// Installed, undeclared, and not owned by dotpkg. Reported, never touched.
    Unmanaged {
        backend: String,
        name: String,
        version: String,
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
}

/// Pure. No I/O, no network, no subprocess — every input is passed in, which is
/// what lets the whole decision layer be tested on any OS.
pub fn plan(
    declared: &Config,
    lock: &Lock,
    installed: &[Installed],
    state: &State,
    running: &[String],
) -> Plan {
    let mut actions = Vec::new();
    let mut prunes = Vec::new();
    let mut reports = Vec::new();

    let declared_scoop: BTreeSet<&str> =
        declared.scoop.packages.iter().map(String::as_str).collect();
    let running: BTreeSet<&str> = running.iter().map(String::as_str).collect();

    // Declared packages: install / upgrade / downgrade / skip.
    for name in &declared.scoop.packages {
        let current = installed
            .iter()
            .find(|i| i.backend == SCOOP && &i.name == name);

        let Some(pin) = lock.scoop.get(name) else {
            actions.push(Action::Skip {
                backend: SCOOP.into(),
                name: name.clone(),
                reason: SkipReason::NotLocked,
            });
            continue;
        };
        let want = pin.version();

        match current {
            None => actions.push(Action::Install {
                backend: SCOOP.into(),
                name: name.clone(),
                version: want.to_string(),
            }),
            Some(cur) if cur.version == want => {}
            Some(cur) => {
                // Checked only once a change is actually called for, so a
                // healthy running package produces no line at all.
                if running.contains(name.as_str()) {
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
                    });
                } else {
                    actions.push(Action::Downgrade {
                        backend: SCOOP.into(),
                        name: name.clone(),
                        from: cur.version.clone(),
                        to: want.to_string(),
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
        if declared_scoop.contains(inst.name.as_str()) {
            continue;
        }
        if SCOOP_HELPERS.contains(&inst.name.as_str()) {
            continue;
        }
        if state.owns(SCOOP, &inst.name) {
            prunes.push(Action::Prune {
                backend: SCOOP.into(),
                name: inst.name.clone(),
                version: inst.version.clone(),
            });
        } else {
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
/// shapes like `26.01` and `2026.07.15.08.55` that semver rejects, and getting
/// the direction wrong only changes the arrow shown in the plan, never whether
/// a change happens.
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
}
