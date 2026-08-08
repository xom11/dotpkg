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
            } => {
                format!("  + {backend:<6} {name:<14} {version:<24} (install)")
            }
            Action::Upgrade {
                backend,
                name,
                from,
                to,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SCOOP, WINGET};

    #[test]
    fn an_empty_plan_says_so_rather_than_printing_nothing() {
        assert!(render(&Plan::default()).contains("nothing to do"));
    }

    #[test]
    fn every_action_kind_gets_a_distinct_marker() {
        let plan = Plan {
            actions: vec![
                Action::Install {
                    backend: SCOOP.into(),
                    name: "ripgrep".into(),
                    version: "14.1.0".into(),
                },
                Action::Upgrade {
                    backend: WINGET.into(),
                    name: "Brave.Brave".into(),
                    from: "1.85".into(),
                    to: "1.86".into(),
                },
                Action::Downgrade {
                    backend: SCOOP.into(),
                    name: "fzf".into(),
                    from: "0.74.2".into(),
                    to: "0.74.1".into(),
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
