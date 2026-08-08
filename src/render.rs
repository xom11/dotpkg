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
                    SkipReason::Running => "running -- stop it first",
                    SkipReason::NotLocked => "no lock entry -- run `dotpkg update`",
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
        };
        out.push_str(&line);
        out.push('\n');
    }

    if plan.actions.is_empty() {
        out.push_str("  nothing to do\n");
    } else {
        out.push_str(&format!(
            "\n  {} change(s), {} skipped\n",
            plan.change_count(),
            plan.skip_count()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SCOOP;

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
                Action::Unmanaged {
                    backend: SCOOP.into(),
                    name: "antigravity".into(),
                    version: "2.0.6".into(),
                },
            ],
        };
        let out = render(&plan);
        assert!(out.contains("+ scoop  ripgrep"));
        assert!(out.contains("v scoop  fzf"));
        assert!(out.contains("- scoop  aichat"));
        assert!(out.contains("! scoop  kanata"));
        assert!(out.contains("? scoop  antigravity"));
        assert!(out.contains("3 change(s), 1 skipped"));
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
