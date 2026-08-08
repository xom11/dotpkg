use crate::apply::{Outcome, Preparation, Prepared};
use crate::model::Name;
use crate::plan::{Action, Plan, SkipReason};

/// The plan is the product here: `status` is this and nothing else, and in
/// Phase 2 `apply` prints exactly this before asking for confirmation.
pub fn render(plan: &Plan) -> String {
    let mut out = String::new();
    for a in &plan.actions {
        let line = match a {
            Action::Install {
                backend,
                name,
                version,
                ..
            } => {
                format!("  + {backend:<6} {name:<14} {version:<24} (install)")
            }
            Action::Upgrade {
                backend,
                name,
                from,
                to,
                ..
            } => {
                format!(
                    "  ^ {backend:<6} {name:<14} {:<24} (upgrade)",
                    format!("{from} -> {to}")
                )
            }
            Action::Downgrade {
                backend,
                name,
                from,
                to,
                ..
            } => {
                format!(
                    "  v {backend:<6} {name:<14} {:<24} (downgrade, from lock)",
                    format!("{from} -> {to}")
                )
            }
            Action::Prune {
                backend,
                name,
                version,
            } => {
                format!("  - {backend:<6} {name:<14} {version:<24} (prune, owned)")
            }
            Action::Skip {
                backend,
                name,
                reason,
            } => {
                let why = match reason {
                    SkipReason::Running => "running -- stop it first".to_string(),
                    SkipReason::NotLocked => "no lock entry -- run `dotpkg update`".to_string(),
                    SkipReason::BackendNotImplemented => {
                        format!("{backend} backend not implemented until phase 4")
                    }
                };
                format!("  ! {backend:<6} {name:<14} {why}")
            }
            Action::Unmanaged {
                backend,
                name,
                version,
            } => {
                format!("  ? {backend:<6} {name:<14} {version:<24} (unmanaged -- no action)")
            }
            Action::ArchDrift {
                backend,
                name,
                have,
                want,
            } => {
                format!(
                    "  ~ {backend:<6} {name:<14} {:<24} (architecture drift -- reported, not fixed)",
                    format!("{have}, declared {want}")
                )
            }
        };
        out.push_str(&line);
        out.push('\n');
    }

    if plan.actions.is_empty() {
        out.push_str("  nothing to do\n");
    } else {
        let mut summary = format!(
            "\n  {} change(s), {} skipped",
            plan.change_count(),
            plan.skip_count()
        );
        if plan.drift_count() > 0 {
            summary.push_str(&format!(", {} architecture drift", plan.drift_count()));
        }
        summary.push('\n');
        out.push_str(&summary);
    }
    out
}

/// Renders what `prepare` found out, in the shape the design specifies.
///
/// `Nothing has been changed.` is printed unconditionally, even when
/// `p.prepared` is empty: it is the promise of the whole phase, true whether
/// the preparation found nothing to do, everything ready, or everything
/// failed.
pub fn render_preparation(p: &Preparation) -> String {
    let mut out = String::new();
    if p.prepared.is_empty() {
        out.push_str("  nothing to prepare\n");
    } else {
        for item in &p.prepared {
            out.push_str(&prepared_line(item));
        }
    }
    out.push('\n');
    if !p.prepared.is_empty() {
        out.push_str(&format!(
            "  {} of {} changes ready, {} failed, {} skipped, {} not locked.\n",
            p.ready_count(),
            p.ready_count() + p.failed_count(),
            p.failed_count(),
            p.skipped_count(),
            p.not_locked_count(),
        ));
    }
    out.push_str("  Nothing has been changed.\n");
    out
}

fn prepared_line(item: &Prepared) -> String {
    let (backend, name) = action_backend_name(&item.action);
    let (marker, rest) = match &item.outcome {
        Outcome::ReadyToFetch { .. } | Outcome::ReadyToRemove => {
            ("ready", ready_rest(&item.action))
        }
        Outcome::Failed { why } => ("FAILED", why.clone()),
        Outcome::Skipped { why } => ("!", why.clone()),
        Outcome::NotLocked => ("!", "no lock entry -- run `dotpkg update`".to_string()),
        Outcome::Report => report_marker_and_rest(&item.action),
    };
    format!("  {marker:<8}{backend:<6} {name:<13}{rest}\n")
}

/// Every `Action` variant names a backend and a package; this is the one
/// place that destructures all seven just to reach those two fields.
fn action_backend_name(action: &Action) -> (&str, &Name) {
    match action {
        Action::Install { backend, name, .. }
        | Action::Upgrade { backend, name, .. }
        | Action::Downgrade { backend, name, .. }
        | Action::Prune { backend, name, .. }
        | Action::Skip { backend, name, .. }
        | Action::Unmanaged { backend, name, .. }
        | Action::ArchDrift { backend, name, .. } => (backend.as_str(), name),
    }
}

/// The right-hand side of a `ready` line. `classify` only ever produces a
/// ready outcome for these four action shapes (`ReadyToFetch` for the three
/// `NeedsArtifact` kinds, `ReadyToRemove` for `Prune`), so the fallback below
/// is unreachable in practice; it stays total rather than panicking if that
/// ever changes.
fn ready_rest(action: &Action) -> String {
    match action {
        Action::Install { version, .. } => format!("{version:<18}(install)"),
        Action::Upgrade { from, to, .. } => {
            format!("{:<18}(upgrade)", format!("{from} -> {to}"))
        }
        Action::Downgrade { from, to, .. } => {
            format!("{:<18}(downgrade)", format!("{from} -> {to}"))
        }
        Action::Prune { version, .. } => format!("{version:<18}(prune)"),
        _ => String::new(),
    }
}

/// The marker and right-hand side for a passed-through `Outcome::Report`.
/// Mirrors `render`'s own `Unmanaged`/`ArchDrift` lines so `status` and
/// `apply --prepare` describe the same fact the same way.
fn report_marker_and_rest(action: &Action) -> (&'static str, String) {
    match action {
        Action::Unmanaged { version, .. } => {
            ("?", format!("{version:<18}(unmanaged -- no action)"))
        }
        Action::ArchDrift { have, want, .. } => (
            "~",
            format!(
                "{:<18}(architecture drift -- reported, not fixed)",
                format!("{have}, declared {want}")
            ),
        ),
        _ => ("?", String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::{Outcome, Preparation, Prepared};
    use crate::model::{SCOOP, WINGET};

    /// A fetched-and-verified outcome, with the staged path `prepare` would
    /// really have produced. `ReadyToFetch` carries a `PathBuf` rather than an
    /// `Option`, so a test can no longer describe an install as having no
    /// manifest at all.
    fn ready_to_fetch(app: &str, version: &str) -> Outcome {
        Outcome::ReadyToFetch {
            manifest: std::path::PathBuf::from(format!("/stage/{app}/{version}/{app}.json")),
        }
    }

    #[test]
    fn an_empty_plan_says_so_rather_than_printing_nothing() {
        assert!(render(&Plan::default()).contains("nothing to do"));
    }

    // -- render_preparation ---------------------------------------------

    #[test]
    fn nothing_has_been_changed_appears_even_for_an_empty_preparation() {
        // The promise of the whole phase. True whether the run found nothing
        // to do, everything ready, or everything failed -- so it must not be
        // conditioned on the preparation having any content at all.
        assert!(render_preparation(&Preparation::default()).contains("Nothing has been changed."));
    }

    #[test]
    fn render_preparation_matches_the_designed_shape() {
        let p = Preparation {
            prepared: vec![
                Prepared {
                    action: Action::Install {
                        backend: SCOOP.into(),
                        name: "ripgrep".into(),
                        version: "14.1.0".into(),
                        arch: None,
                    },
                    outcome: ready_to_fetch("ripgrep", "14.1.0"),
                },
                Prepared {
                    action: Action::Upgrade {
                        backend: SCOOP.into(),
                        name: "bat".into(),
                        from: "0.25.0".into(),
                        to: "0.26.1".into(),
                        arch: None,
                    },
                    outcome: ready_to_fetch("bat", "0.26.1"),
                },
                Prepared {
                    action: Action::Install {
                        backend: SCOOP.into(),
                        name: "fzf".into(),
                        version: "0.75.0".into(),
                        arch: None,
                    },
                    outcome: Outcome::Failed {
                        why: "commit a28d0c56 is not in bucket main".into(),
                    },
                },
                Prepared {
                    action: Action::Upgrade {
                        backend: SCOOP.into(),
                        name: "neovim".into(),
                        from: "0.10.0".into(),
                        to: "0.11.0".into(),
                        arch: None,
                    },
                    outcome: Outcome::Failed {
                        why: "download failed: hash mismatch".into(),
                    },
                },
                Prepared {
                    action: Action::Skip {
                        backend: SCOOP.into(),
                        name: "kanata".into(),
                        reason: SkipReason::Running,
                    },
                    outcome: Outcome::Skipped {
                        why: "running -- stop it first".into(),
                    },
                },
                Prepared {
                    action: Action::Skip {
                        backend: SCOOP.into(),
                        name: "zellij".into(),
                        reason: SkipReason::NotLocked,
                    },
                    outcome: Outcome::NotLocked,
                },
            ],
        };

        // Byte-for-byte against the design doc's own example: the strongest
        // check available that the column widths were reverse-engineered
        // correctly rather than eyeballed.
        let expected = "  ready   scoop  ripgrep      14.1.0            (install)
  ready   scoop  bat          0.25.0 -> 0.26.1  (upgrade)
  FAILED  scoop  fzf          commit a28d0c56 is not in bucket main
  FAILED  scoop  neovim       download failed: hash mismatch
  !       scoop  kanata       running -- stop it first
  !       scoop  zellij       no lock entry -- run `dotpkg update`

  2 of 4 changes ready, 2 failed, 1 skipped, 1 not locked.
  Nothing has been changed.
";
        assert_eq!(render_preparation(&p), expected);
    }

    #[test]
    fn a_ready_prune_shows_the_prune_suffix() {
        let p = Preparation {
            prepared: vec![Prepared {
                action: Action::Prune {
                    backend: SCOOP.into(),
                    name: "aichat".into(),
                    version: "0.30.0".into(),
                },
                outcome: Outcome::ReadyToRemove,
            }],
        };
        let out = render_preparation(&p);
        assert!(out.contains("ready   scoop  aichat"), "got: {out}");
        assert!(out.contains("(prune)"), "got: {out}");
        assert!(out.contains("1 of 1 changes ready"), "got: {out}");
    }

    #[test]
    fn a_ready_downgrade_shows_the_reverse_arrow() {
        let p = Preparation {
            prepared: vec![Prepared {
                action: Action::Downgrade {
                    backend: SCOOP.into(),
                    name: "fzf".into(),
                    from: "0.74.2".into(),
                    to: "0.74.1".into(),
                    arch: None,
                },
                outcome: ready_to_fetch("fzf", "0.74.1"),
            }],
        };
        let out = render_preparation(&p);
        assert!(out.contains("0.74.2 -> 0.74.1"), "got: {out}");
        assert!(out.contains("(downgrade)"), "got: {out}");
    }

    #[test]
    fn report_lines_render_with_their_own_markers_and_do_not_affect_the_summary() {
        let p = Preparation {
            prepared: vec![
                Prepared {
                    action: Action::Install {
                        backend: SCOOP.into(),
                        name: "ripgrep".into(),
                        version: "14.1.0".into(),
                        arch: None,
                    },
                    outcome: ready_to_fetch("ripgrep", "14.1.0"),
                },
                Prepared {
                    action: Action::Unmanaged {
                        backend: SCOOP.into(),
                        name: "antigravity".into(),
                        version: "2.0.6".into(),
                    },
                    outcome: Outcome::Report,
                },
                Prepared {
                    action: Action::ArchDrift {
                        backend: SCOOP.into(),
                        name: "python".into(),
                        have: "64bit".into(),
                        want: "arm64".into(),
                    },
                    outcome: Outcome::Report,
                },
            ],
        };
        let out = render_preparation(&p);
        assert!(out.contains("?       scoop  antigravity"), "got: {out}");
        assert!(out.contains("(unmanaged -- no action)"), "got: {out}");
        assert!(out.contains("~       scoop  python"), "got: {out}");
        assert!(
            out.contains("(architecture drift -- reported, not fixed)"),
            "got: {out}"
        );
        // Two Reports plus one Ready must not inflate "changes": only the
        // Ready/Failed actions are changes at all.
        assert!(
            out.contains("1 of 1 changes ready"),
            "reports must not count as changes: {out}"
        );
    }

    #[test]
    fn every_action_kind_gets_a_distinct_marker() {
        let plan = Plan {
            actions: vec![
                Action::Install {
                    backend: SCOOP.into(),
                    name: "ripgrep".into(),
                    version: "14.1.0".into(),
                    arch: None,
                },
                Action::Upgrade {
                    backend: WINGET.into(),
                    name: "Brave.Brave".into(),
                    from: "1.85".into(),
                    to: "1.86".into(),
                    arch: None,
                },
                Action::Downgrade {
                    backend: SCOOP.into(),
                    name: "fzf".into(),
                    from: "0.74.2".into(),
                    to: "0.74.1".into(),
                    arch: None,
                },
                Action::Prune {
                    backend: SCOOP.into(),
                    name: "aichat".into(),
                    version: "0.30.0".into(),
                },
                Action::Skip {
                    backend: SCOOP.into(),
                    name: "kanata".into(),
                    reason: SkipReason::Running,
                },
                Action::Skip {
                    backend: WINGET.into(),
                    name: "Git.Git".into(),
                    reason: SkipReason::BackendNotImplemented,
                },
                Action::Unmanaged {
                    backend: SCOOP.into(),
                    name: "antigravity".into(),
                    version: "2.0.6".into(),
                },
                Action::ArchDrift {
                    backend: SCOOP.into(),
                    name: "python".into(),
                    have: "64bit".into(),
                    want: "arm64".into(),
                },
            ],
        };
        let out = render(&plan);
        assert!(out.contains("+ scoop  ripgrep"));
        assert!(out.contains("^ winget Brave.Brave"));
        assert!(out.contains("v scoop  fzf"));
        assert!(out.contains("- scoop  aichat"));
        assert!(out.contains("! scoop  kanata"));
        assert!(out.contains("! winget Git.Git"));
        assert!(out.contains("? scoop  antigravity"));
        assert!(out.contains("~ scoop  python"));
        assert!(out.contains("64bit, declared arm64"));
        assert!(out.contains("4 change(s), 2 skipped, 1 architecture drift"));
    }

    #[test]
    fn a_declared_winget_package_says_why_it_is_not_acted_on() {
        // The user must be able to tell "dotpkg saw this and cannot act yet"
        // apart from "dotpkg never saw it". A blank line does neither.
        let plan = Plan {
            actions: vec![Action::Skip {
                backend: WINGET.into(),
                name: "Brave.Brave".into(),
                reason: SkipReason::BackendNotImplemented,
            }],
        };
        let out = render(&plan);
        assert!(out.contains("Brave.Brave"), "got: {out}");
        assert!(
            out.contains("winget backend not implemented until phase 4"),
            "got: {out}"
        );
    }

    #[test]
    fn a_skip_says_what_to_do_about_it() {
        let plan = Plan {
            actions: vec![Action::Skip {
                backend: SCOOP.into(),
                name: "bat".into(),
                reason: SkipReason::NotLocked,
            }],
        };
        assert!(render(&plan).contains("dotpkg update"));
    }
}
