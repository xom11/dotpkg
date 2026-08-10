use crate::apply::{Outcome, Preparation, Prepared};
use crate::execute::{Execution, ItemResult};
use crate::model::Name;
use crate::plan::{Action, Plan, SkipReason};
// Only this file's own tests name `Divergence` directly (`render`'s match
// arm binds `divergence` and calls `.describe()` without needing the type in
// scope), so the import is `cfg(test)`-gated -- otherwise it is unused in a
// normal build and the crate would no longer be warning-free.
#[cfg(test)]
use crate::plan::Divergence;

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
                arch,
            } => {
                format!(
                    "  + {backend:<6} {name:<14} {version:<24} (install{})",
                    arch_suffix(arch)
                )
            }
            Action::Upgrade {
                backend,
                name,
                from,
                to,
                arch,
            } => {
                format!(
                    "  ^ {backend:<6} {name:<14} {:<24} (upgrade{})",
                    format!("{from} -> {to}"),
                    arch_suffix(arch)
                )
            }
            Action::Downgrade {
                backend,
                name,
                from,
                to,
                arch,
            } => {
                format!(
                    "  v {backend:<6} {name:<14} {:<24} (downgrade, from lock{})",
                    format!("{from} -> {to}"),
                    arch_suffix(arch)
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
                    SkipReason::ReportedOnly(divergence) => divergence.describe(),
                    SkipReason::Opaque => {
                        "installed, but its state could not be read -- see the warnings above"
                            .to_string()
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

/// Renders what `prepare` found out.
///
/// It does **not** print "Nothing has been changed." — that sentence is
/// `--prepare`'s promise, and in a full `apply` run this same table is
/// printed before the mutations begin, which would make the run state the
/// promise and then break it. `main.rs` prints the promise itself, in the
/// `--prepare` branch, where it is true.
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
    out
}

/// What the run actually did, according to the disk.
///
/// It never says "N upgraded". Every number here comes from a `verdict`
/// against the filesystem, and the wording says so, because the tool this
/// orchestrates reports success unconditionally.
///
/// A failed recovery-script write is surfaced here too, not just as the
/// warning `execute` prints the moment it happens: that warning sits above
/// however many minutes of scoop output follow it, and this table is what a
/// user actually reads at the end of the run.
pub fn render_execution(ex: &Execution) -> String {
    let mut out = String::new();
    for (name, r) in &ex.results {
        let line = match r {
            // `{name:<14} ` -- a literal space after the padded field, the
            // same fix `prepared_line` needed and got at Task 16
            // (`src/render.rs:229`; see its own doc comment for the
            // mechanism). This function used the narrower `{name:<13}` with
            // no literal separator, the identical shape that turned into
            // a real, dogfood-found glued-together line at `prepared_line`
            // -- one column short, and reachable here the same way, with an
            // ordinary scoop package: `windows-terminal` is 16 characters,
            // long enough to exhaust the padding entirely and leave nothing
            // to separate it from what follows.
            ItemResult::Done => format!("  done    scoop  {name:<14} verified on disk"),
            ItemResult::Failed { why, .. } => format!("  FAILED  scoop  {name:<14} {why}"),
            ItemResult::Held(why) => format!("  held    scoop  {name:<14} {why}"),
        };
        out.push_str(&line);
        out.push('\n');
    }
    for name in &ex.dropped_ghosts {
        out.push_str(&format!(
            "  note    scoop  {name:<14} ownership record dropped: nothing by that name is installed\n"
        ));
    }
    out.push_str(&format!(
        "\n  {} verified on disk, {} failed, {} held.\n",
        ex.changed(),
        ex.failed(),
        ex.held()
    ));
    if ex.failed() > 0 {
        // `changed()` alone would understate this: a `Replace` whose
        // uninstall really removed the package before its install failed
        // has `changed() == 0` (no result is `Done`) but has still altered
        // the machine, and `touched()` is what says so. Without checking it
        // here too, this sentence would say "nothing was changed" about a
        // machine that just lost a package -- the same claim-more-than-
        // verified mistake this function exists to avoid, just moved from
        // "changed" to "unchanged".
        if ex.changed() > 0 || ex.touched() > 0 {
            out.push_str("  Some packages were changed and some were not. Look at the machine.\n");
        } else {
            out.push_str(
                "  Nothing was changed; the failure(s) above are everything that happened.\n",
            );
        }
    }
    if let Some(why) = &ex.recovery_write_failed {
        out.push_str(&format!(
            "  warning: the recovery script could not be written, so a crash from here on \
             leaves no automatic way back to what was installed before this run: {why}\n"
        ));
    }
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
    // `{name:<14} ` -- a literal space after the padded field, not just
    // padding relied on to provide one. `{name:<13}{rest}` (no literal
    // space, one column narrower) was sized against Phase 2b1's scoop-only
    // examples, every one of them under 13 characters; a real winget id
    // (`Microsoft.WindowsAppRuntime.1.7`, `JanDeDobbeleer.OhMyPosh`, ...)
    // routinely exceeds it, and `{:<N}` does not truncate or insert a
    // minimum gap when the value is already >= N wide -- it emits nothing
    // extra at all, so `rest` glued directly onto the end of `name` with no
    // separator. Found by the Phase 4 dogfood's own `apply --prepare`
    // output, not by review. `render(plan)`'s sibling lines already use this
    // same `{name:<14} ` shape (explicit space, one field over) for exactly
    // this reason; matched here rather than inventing a second convention.
    format!("  {marker:<8}{backend:<6} {name:<14} {rest}\n")
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

/// `", arm64"` when the plan resolved an architecture, empty when it did not
/// (`Arch::Keep`, or nothing installed to fall back to) -- so an `Install`,
/// `Upgrade` or `Downgrade` line says which architecture `scoop download`
/// will actually be told to fetch, which is half the reason Task 8 resolves
/// it in the plan at all: the other half is that it must appear in the plan
/// the user says yes to, not stay a fact only `apply` finds out later.
fn arch_suffix(arch: &Option<String>) -> String {
    match arch {
        Some(a) => format!(", {a}"),
        None => String::new(),
    }
}

/// The right-hand side of a `ready` line. `classify` only ever produces a
/// ready outcome for these four action shapes (`ReadyToFetch` for the three
/// `NeedsArtifact` kinds, `ReadyToRemove` for `Prune`), so the fallback below
/// is unreachable in practice; it stays total rather than panicking if that
/// ever changes.
fn ready_rest(action: &Action) -> String {
    match action {
        Action::Install { version, arch, .. } => {
            format!("{version:<18}(install{})", arch_suffix(arch))
        }
        Action::Upgrade { from, to, arch, .. } => {
            format!(
                "{:<18}(upgrade{})",
                format!("{from} -> {to}"),
                arch_suffix(arch)
            )
        }
        Action::Downgrade { from, to, arch, .. } => {
            format!(
                "{:<18}(downgrade{})",
                format!("{from} -> {to}"),
                arch_suffix(arch)
            )
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

use crate::update::{Change, Update};

/// The diff between the old lock and the new one — the only place both exist
/// at once, and therefore the only place a user can be told that a
/// same-version re-pin will produce no action at all.
pub fn render_update(u: &Update) -> String {
    let mut out = String::new();
    for c in &u.changes {
        let line = match c {
            Change::Added {
                backend,
                name,
                version,
            } => {
                format!("  + {backend:<6} {name:<14} {version:<26} (new pin)")
            }
            Change::VersionChanged {
                backend,
                name,
                from,
                to,
            } => format!(
                "  ^ {backend:<6} {name:<14} {:<26} (version changed)",
                format!("{from} -> {to}")
            ),
            Change::RepinnedSameVersion {
                backend,
                name,
                version,
            } => format!(
                "  = {backend:<6} {name:<14} {:<26} (apply will not act on this)",
                format!("{version}, commit re-pinned")
            ),
            Change::Dropped {
                backend,
                name,
                version,
            } => {
                format!("  - {backend:<6} {name:<14} {version:<26} (dropped, no longer declared)")
            }
            // Two different facts share this variant, and they must not read
            // the same: a package that already had a pin genuinely keeps it
            // (shown, so the reader can tell what is still installed), while
            // a brand-new package whose first resolution failed has no pin
            // to keep at all -- saying "kept the previous pin" about it would
            // be a false line the reader has no way to catch.
            Change::Kept {
                backend,
                name,
                version: Some(v),
                why,
            } => {
                format!("  ! {backend:<6} {name:<14} {v:<26} kept the previous pin: {why}")
            }
            Change::Kept {
                backend,
                name,
                version: None,
                why,
            } => {
                format!("  ! {backend:<6} {name:<14} could not be resolved, nothing to keep: {why}")
            }
            // An unchanged package is the ordinary case and would drown the
            // lines that matter. Counted in the summary instead.
            Change::Unchanged { .. } => continue,
        };
        out.push_str(&line);
        out.push('\n');
    }

    let unchanged = u
        .changes
        .iter()
        .filter(|c| matches!(c, Change::Unchanged { .. }))
        .count();
    out.push_str(&format!(
        "\n  {} changed, {} unchanged, {} could not be resolved.\n",
        u.changes.len() - unchanged - u.failed_count(),
        unchanged,
        u.failed_count(),
    ));
    if !u.wrote_anything() {
        if u.failed_count() == 0 {
            // Every change is `Unchanged`: the lock really does match what a
            // fresh resolve would produce, so saying so is true.
            out.push_str("  pkg.lock is already current -- not rewritten.\n");
        } else {
            // `wrote_anything()` is false for the same reason above, but not
            // because everything converged -- at least one package could not
            // be re-resolved (`Change::Kept`) and nothing was written for it
            // either. Saying "already current" here would tell the reader
            // the opposite of their situation, so name the failures instead.
            out.push_str(&format!(
                "  pkg.lock was not rewritten -- {} package(s) could not be \
                 resolved.\n",
                u.failed_count()
            ));
        }
    }
    out
}

use crate::adopt::{Matched, Outcome as AdoptOutcome};

/// `backend` is the one `--backend` the whole `adopt` invocation ran under
/// (`SCOOP` or `WINGET`, `src/model.rs`): every package `o` names was adopted
/// from that single backend -- `adopt::run` dispatches to exactly one of
/// `run_scoop`/`run_winget` per call -- so it is a parameter here rather than
/// a field threaded through every tuple in `Outcome`.
pub fn render_adopt(backend: &str, o: &AdoptOutcome) -> String {
    let mut out = String::new();
    for (name, matched, previous_version) in &o.adopted {
        let how = match matched {
            Matched::Content => "the installed manifest matches the bucket exactly",
            Matched::Version => "matched by version only -- the installed manifest differs",
            Matched::WingetConfirmed => "winget confirms this version is still in its index",
        };
        // `adopt_one` does not refuse when `pkg.lock` already pins this name
        // (see `Outcome::adopted`'s doc comment), so this write can silently
        // replace a committed pin. Naming what was replaced is the same
        // promise `Change::RepinnedSameVersion` makes on the `update` side --
        // a pin changing is reported, not folded into a bare "adopted".
        match previous_version {
            Some(prev) => out.push_str(&format!(
                "  + {backend:<6} {name:<14} adopted ({how}); replaced the existing pin {prev}\n"
            )),
            None => out.push_str(&format!("  + {backend:<6} {name:<14} adopted ({how})\n")),
        }
    }
    for (name, why) in &o.refused {
        out.push_str(&format!("  ! {backend:<6} {name:<14} {why}\n"));
    }
    // A write that stopped part way through is not a refusal: files really did
    // change. Naming them is the whole point -- the error alone names only the
    // file that failed, and until this the user saw that error and no line at
    // all saying what had already been rewritten.
    if let Some(p) = &o.partial_write {
        out.push_str(&format!(
            "  ! {backend:<6} {:<14} a write failed part way through: {}\n",
            p.name, p.why
        ));
        // The complement, computed rather than phrased: "and the rest were
        // not" is the kind of line that goes stale the moment a fourth file
        // appears.
        let untouched: Vec<&str> = ["pkg.lock", "pkg.toml", "state.json"]
            .into_iter()
            .filter(|f| !p.wrote.contains(f))
            .collect();
        if p.wrote.is_empty() {
            out.push_str("      nothing was changed on disk.\n");
        } else {
            out.push_str(&format!(
                "      changed on disk: {}. Not changed: {}.\n",
                p.wrote.join(", "),
                untouched.join(", ")
            ));
        }
        out.push_str("      The packages after it were not attempted.\n");
    }
    out.push_str(&format!(
        "\n  {} adopted, {} refused. Nothing installed and nothing removed.\n",
        o.adopted.len(),
        o.refused.len()
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::{Outcome, Preparation, Prepared};
    use crate::lock::Lock;
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
    fn the_preparation_table_does_not_promise_anything_about_mutations() {
        // `apply` prints this same table before it starts changing things, so
        // the promise cannot live here. main.rs prints it in the --prepare
        // branch, and tests/cli.rs asserts it appears there.
        let out = render_preparation(&Preparation::default());
        assert!(
            !out.contains("Nothing has been changed."),
            "the promise belongs to --prepare's caller: {out}"
        );
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

        // Byte-for-byte: the strongest check available that the column
        // widths were reverse-engineered correctly rather than eyeballed.
        // One column wider than Phase 2b1's original design-doc mockup
        // (`{name:<13}{rest}` -> `{name:<14} {rest}`) -- see `prepared_line`'s
        // own doc comment for why: a name this wide plus a literal space is
        // what keeps a long winget id from running into its version with no
        // separator, and every scoop name here is short enough that the
        // extra column is the only visible difference from before.
        let expected = "  ready   scoop  ripgrep        14.1.0            (install)
  ready   scoop  bat            0.25.0 -> 0.26.1  (upgrade)
  FAILED  scoop  fzf            commit a28d0c56 is not in bucket main
  FAILED  scoop  neovim         download failed: hash mismatch
  !       scoop  kanata         running -- stop it first
  !       scoop  zellij         no lock entry -- run `dotpkg update`

  2 of 4 changes ready, 2 failed, 1 skipped, 1 not locked.
";
        assert_eq!(render_preparation(&p), expected);
    }

    #[test]
    fn a_name_at_least_14_characters_long_still_gets_a_separator_before_its_rest() {
        // The dogfood-found shape, reproduced directly (the same
        // `Divergence::Change` a real `Brave.Brave` skip renders elsewhere in
        // this file, with a name long enough to exhaust `{name:<14}`'s
        // padding entirely -- nothing left to pad with, so the literal space
        // after it is the only thing that still separates it from what
        // follows). Without that literal space, this renders as
        // "...JanDeDobbeleer.OhMyPosh29.36.0.0 ->..." -- exactly what a14's
        // real `apply --prepare` output showed.
        let p = Preparation {
            prepared: vec![Prepared {
                action: Action::Skip {
                    backend: WINGET.into(),
                    name: Name::new("JanDeDobbeleer.OhMyPosh"),
                    reason: SkipReason::ReportedOnly(Divergence::Change {
                        from: "29.36.0.0".into(),
                        to: "30.6.3".into(),
                    }),
                },
                outcome: Outcome::Skipped {
                    why: Divergence::Change {
                        from: "29.36.0.0".into(),
                        to: "30.6.3".into(),
                    }
                    .describe(),
                },
            }],
        };
        let out = render_preparation(&p);
        assert!(
            out.contains("JanDeDobbeleer.OhMyPosh 29.36.0.0 -> 30.6.3"),
            "a literal space must separate the name from what follows even \
             when the name itself is >= 14 characters: {out}"
        );
    }

    #[test]
    fn a_short_name_is_still_padded_not_just_given_one_bare_space() {
        // The other side of the same fix: this must not degrade into
        // "always emit exactly one space and stop aligning columns" --
        // short names still line up in a fixed-width column, the same as
        // every other line in this table.
        let p = Preparation {
            prepared: vec![Prepared {
                action: Action::Install {
                    backend: SCOOP.into(),
                    name: "fzf".into(),
                    version: "14.1.0".into(),
                    arch: None,
                },
                outcome: ready_to_fetch("fzf", "14.1.0"),
            }],
        };
        let out = render_preparation(&p);
        assert!(
            out.contains("  ready   scoop  fzf            14.1.0            (install)\n"),
            "a short name must still be padded out to the column width, not \
             just followed by one bare space: {out}"
        );
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
    fn a_ready_line_names_the_architecture_the_plan_resolved() {
        // Half the reason Task 8 resolves arch in the plan is that it must
        // show up here, before the user says yes -- not stay a fact `apply`
        // discovers on its own later.
        let p = Preparation {
            prepared: vec![Prepared {
                action: Action::Install {
                    backend: SCOOP.into(),
                    name: "python".into(),
                    version: "3.14.6".into(),
                    arch: Some("arm64".into()),
                },
                outcome: ready_to_fetch("python", "3.14.6"),
            }],
        };
        let out = render_preparation(&p);
        assert!(
            out.contains("(install, arm64)"),
            "the resolved architecture must be visible: {out}"
        );
    }

    #[test]
    fn a_ready_line_adds_nothing_when_no_architecture_was_resolved() {
        // `Arch::Keep`, or a machine with no install.json to fall back to:
        // the parenthetical must read exactly as it did before this field
        // existed -- not "(install, )" or any other trace of an empty value.
        let p = Preparation {
            prepared: vec![Prepared {
                action: Action::Install {
                    backend: SCOOP.into(),
                    name: "python".into(),
                    version: "3.14.6".into(),
                    arch: None,
                },
                outcome: ready_to_fetch("python", "3.14.6"),
            }],
        };
        let out = render_preparation(&p);
        assert!(out.contains("(install)"), "got: {out}");
        assert!(
            !out.contains("arm64") && !out.contains("(install, )"),
            "no architecture was resolved, so none should appear: {out}"
        );
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
                    reason: SkipReason::ReportedOnly(Divergence::Install {
                        version: "2.55.0".into(),
                    }),
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
    fn the_summary_omits_architecture_drift_entirely_when_there_is_none() {
        // The counterweight to the assertion above. Without it the drift
        // clause could be printed unconditionally -- ", 0 architecture
        // drift" on every ordinary run -- and the suite stayed green.
        let plan = Plan {
            actions: vec![Action::Install {
                backend: SCOOP.into(),
                name: "ripgrep".into(),
                version: "14.1.1".into(),
                arch: None,
            }],
        };
        let out = render(&plan);
        assert!(out.contains("1 change(s), 0 skipped"), "{out}");
        assert!(
            !out.contains("architecture drift"),
            "a run with no drift must not mention drift at all: {out}"
        );
    }

    #[test]
    fn a_run_with_no_failures_says_nothing_about_failures() {
        // `render_execution`'s failure sentences are gated on `failed() > 0`.
        // Printed unconditionally, a clean run would end with "the failure(s)
        // above are everything that happened" -- a false line in a tool whose
        // spine is that every printed line is true.
        let out = render_execution(&Execution::default());
        assert!(
            !out.contains("failure(s) above"),
            "a clean run has no failures to explain: {out}"
        );
        assert!(
            !out.contains("Look at the machine"),
            "and nothing to send the user looking for: {out}"
        );
        assert!(
            out.contains("0 failed"),
            "the counts are still printed: {out}"
        );
    }

    #[test]
    fn render_shows_the_architecture_the_plan_resolved() {
        let plan = Plan {
            actions: vec![
                Action::Install {
                    backend: SCOOP.into(),
                    name: "python".into(),
                    version: "3.14.6".into(),
                    arch: Some("arm64".into()),
                },
                Action::Upgrade {
                    backend: SCOOP.into(),
                    name: "bat".into(),
                    from: "0.26.0".into(),
                    to: "0.26.1".into(),
                    arch: Some("arm64".into()),
                },
                Action::Downgrade {
                    backend: SCOOP.into(),
                    name: "fzf".into(),
                    from: "0.74.2".into(),
                    to: "0.74.1".into(),
                    arch: Some("arm64".into()),
                },
            ],
        };
        let out = render(&plan);
        assert!(
            out.contains("(install, arm64)"),
            "an install must say which architecture it will fetch: {out}"
        );
        assert!(
            out.contains("(upgrade, arm64)"),
            "an upgrade must say which architecture it will fetch: {out}"
        );
        assert!(
            out.contains("(downgrade, from lock, arm64)"),
            "a downgrade must say which architecture it will fetch: {out}"
        );
    }

    #[test]
    fn render_adds_nothing_when_no_architecture_was_resolved() {
        // `Arch::Keep`, or nothing installed to fall back to: an undecorated
        // plan line must read exactly as it did before this field existed.
        let plan = Plan {
            actions: vec![
                Action::Install {
                    backend: SCOOP.into(),
                    name: "python".into(),
                    version: "3.14.6".into(),
                    arch: None,
                },
                Action::Upgrade {
                    backend: SCOOP.into(),
                    name: "bat".into(),
                    from: "0.26.0".into(),
                    to: "0.26.1".into(),
                    arch: None,
                },
                Action::Downgrade {
                    backend: SCOOP.into(),
                    name: "fzf".into(),
                    from: "0.74.2".into(),
                    to: "0.74.1".into(),
                    arch: None,
                },
            ],
        };
        let out = render(&plan);
        assert!(out.contains("(install)"), "got: {out}");
        assert!(out.contains("(upgrade)"), "got: {out}");
        assert!(out.contains("(downgrade, from lock)"), "got: {out}");
        assert!(
            !out.contains("arm64"),
            "no architecture was resolved, so none should appear: {out}"
        );
    }

    #[test]
    fn a_winget_package_that_differs_from_the_lock_prints_both_versions_and_says_reported_only() {
        // The user must be able to tell "dotpkg saw this and cannot act yet"
        // apart from "dotpkg never saw it" -- and must be able to see the
        // diff itself (both versions), not just the fact that one exists:
        // hiding that Brave is `151.1 -> 151.2` throws away the whole product
        // of `status`.
        let plan = Plan {
            actions: vec![Action::Skip {
                backend: WINGET.into(),
                name: "Brave.Brave".into(),
                reason: SkipReason::ReportedOnly(Divergence::Change {
                    from: "151.1.93.132".into(),
                    to: "151.1.93.134".into(),
                }),
            }],
        };
        let out = render(&plan);
        assert!(out.contains("Brave.Brave"), "got: {out}");
        assert!(
            out.contains("151.1.93.132") && out.contains("151.1.93.134"),
            "both versions must be visible, not just \"differs\": {out}"
        );
        assert!(out.contains("reported only"), "got: {out}");
    }

    #[test]
    fn a_plan_with_no_winget_divergence_never_prints_reported_only() {
        // The counterweight to the assertion above: without it, an
        // unconditional "reported only" line would satisfy it on every plan,
        // winget or not.
        let plan = Plan {
            actions: vec![Action::Install {
                backend: SCOOP.into(),
                name: "ripgrep".into(),
                version: "14.1.1".into(),
                arch: None,
            }],
        };
        let out = render(&plan);
        assert!(
            !out.contains("reported only"),
            "no winget divergence in this plan: {out}"
        );
    }

    #[test]
    fn a_declared_unlocked_winget_package_is_now_told_to_run_dotpkg_update() {
        // Task 15 made `update::run` resolve winget packages too
        // (`src/update.rs:403-459`; `fold_backend` writes the result into
        // `pkg.lock`'s `winget` table at `:477-487`) -- the exact reverse of
        // what this test used to assert
        // (`a_declared_unlocked_winget_package_is_not_told_to_run_update`,
        // which checked `!out.contains("dotpkg update")`). Staying silent
        // about the one command that now fixes this state would itself be
        // the false line, so the old test is replaced rather than kept
        // alongside a contradicting one.
        let plan = Plan {
            actions: vec![Action::Skip {
                backend: WINGET.into(),
                name: "Git.Git".into(),
                reason: SkipReason::ReportedOnly(Divergence::NotLocked),
            }],
        };
        let out = render(&plan);
        assert!(out.contains("Git.Git"), "got: {out}");
        assert!(out.contains("not in pkg.lock"), "got: {out}");
        assert!(
            out.contains("dotpkg update"),
            "update can resolve a winget package's version now, so the \
             fix must be named: {out}"
        );
        // Paired with the assertion above rather than left standing alone:
        // resolving is no longer impossible, only installing still is. A
        // regression that kept "run `dotpkg update`" but also brought back
        // a "cannot resolve" claim would still print a false line, and
        // would slip past the assertion above by itself.
        assert!(
            !out.contains("cannot resolve"),
            "installing is still impossible, but resolving is not -- the \
             text must not claim otherwise: {out}"
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

    // -- render_execution --------------------------------------------------

    #[test]
    fn the_summary_never_claims_more_than_the_run_verified() {
        let ex = Execution {
            results: vec![
                (Name::new("bat"), ItemResult::Done),
                (
                    Name::new("fzf"),
                    ItemResult::Failed {
                        why: "install did not happen".into(),
                        touched: false,
                    },
                ),
                (
                    Name::new("kanata"),
                    ItemResult::Held("started running".into()),
                ),
            ],
            dropped_ghosts: vec![Name::new("stale")],
            ..Default::default()
        };
        let out = render_execution(&ex);
        assert!(out.contains("1 verified on disk"), "{out}");
        assert!(out.contains("1 failed"), "{out}");
        assert!(out.contains("1 held"), "{out}");
        assert!(out.contains("fzf"), "name what failed: {out}");
        assert!(
            out.contains("stale"),
            "say which ownership records were dropped: {out}"
        );
        assert!(
            !out.contains("Nothing has been changed."),
            "that promise belongs to --prepare and must not appear after a mutation: {out}"
        );
    }

    #[test]
    fn a_failed_install_with_nothing_touched_gets_its_own_honest_sentence() {
        // Important 2's exact shape: 0 done, 1 failed, 0 touched. Before this
        // fix, `render_execution` printed "Some packages were changed and
        // some were not" here regardless -- a claim this scenario does not
        // support, since nothing on the machine changed at all.
        let ex = Execution {
            results: vec![(
                Name::new("fzf"),
                ItemResult::Failed {
                    why: "install did not happen".into(),
                    touched: false,
                },
            )],
            ..Default::default()
        };
        let out = render_execution(&ex);
        assert!(
            !out.contains("Some packages were changed and some were not"),
            "nothing changed, so this must not claim otherwise: {out}"
        );
        assert!(
            out.contains("Nothing was changed"),
            "say plainly that nothing changed: {out}"
        );
    }

    #[test]
    fn a_done_package_alongside_an_untouched_failure_still_says_some_changed() {
        // render.rs:181's `changed() > 0 || touched() > 0` looked like it
        // might be an equivalent mutant under `>` -> `<`, reasoning that
        // `touched()` is a superset of `changed()` (the comment above the
        // line argues `touched()` catches cases `changed()` misses). But the
        // two sets are disjoint, not nested: `changed()` counts `Done`;
        // `touched()` counts `Failed { touched: true }` (`execute.rs`'s
        // `Execution::changed`/`touched`). `touched() >= changed()` only
        // holds when `changed() == 0`.
        //
        // This is the case where it does not: one `Done` and one
        // `Failed { touched: false }` (reachable -- `Step::Remove`'s
        // uninstall-command-failed arm, `execute.rs:221`, never touches the
        // machine). `changed() == 1`, `touched() == 0`. The real code's
        // `changed() > 0` disjunct is true and prints the "some changed"
        // sentence. The `>` -> `<` mutant compares a `usize` against 0 with
        // `<`, which is always false, so both disjuncts are permanently
        // false and it prints "Nothing was changed" instead -- a false
        // claim about a machine that just gained a package.
        let ex = Execution {
            results: vec![
                (Name::new("bat"), ItemResult::Done),
                (
                    Name::new("fzf"),
                    ItemResult::Failed {
                        why: "install did not happen".into(),
                        touched: false,
                    },
                ),
            ],
            ..Default::default()
        };
        let out = render_execution(&ex);
        assert!(
            out.contains("Some packages were changed and some were not"),
            "a Done alongside an untouched failure must say so, not claim \
             nothing changed: {out}"
        );
        assert!(
            !out.contains("Nothing was changed"),
            "changed() == 1, so this must not be reached: {out}"
        );
    }

    #[test]
    fn a_failed_recovery_write_is_surfaced_not_silently_dropped() {
        // `execute` already prints a warning the moment `write_recovery`
        // fails, above however many minutes of scoop output follow -- but
        // nothing read `Execution.recovery_write_failed` back afterwards,
        // which made that warning as good as gone by the time the run ends.
        let ex = Execution {
            recovery_write_failed: Some("permission denied".into()),
            ..Default::default()
        };
        let out = render_execution(&ex);
        assert!(out.contains("permission denied"), "{out}");
    }

    #[test]
    fn a_name_at_least_14_characters_long_still_gets_a_separator_before_verified_on_disk() {
        // The same dogfood-found shape `prepared_line` had (see the paired
        // tests above `render_preparation`'s own section, and that
        // function's own doc comment): `render_execution`'s `Done` line used
        // `{name:<13}` with no literal separator -- exhausted by any name
        // >= 13 characters, gluing the name straight onto "verified on
        // disk". `windows-terminal` (16 characters) is an ordinary scoop
        // package, not a constructed edge case; it is 16, not 13, only
        // because the fixed column is now one wider (`<14`), the same
        // widening `prepared_line` got.
        let ex = Execution {
            results: vec![(Name::new("windows-terminal"), ItemResult::Done)],
            ..Default::default()
        };
        let out = render_execution(&ex);
        assert!(
            out.contains("windows-terminal verified on disk"),
            "a literal space must separate the name from what follows even \
             when the name itself is >= 14 characters: {out}"
        );
    }

    #[test]
    fn a_short_name_in_render_execution_is_still_padded_not_just_given_one_bare_space() {
        // The other side of the same fix: this must not degrade into
        // "always emit exactly one space and stop aligning columns" -- short
        // names still line up in a fixed-width column. Built with the same
        // `{:<14}` rule the real code uses, rather than a hand-counted
        // literal, so a padding-width typo in the test itself cannot make
        // this pass for the wrong reason.
        let ex = Execution {
            results: vec![(Name::new("fzf"), ItemResult::Done)],
            ..Default::default()
        };
        let out = render_execution(&ex);
        let expected = format!("  done    scoop  {:<14} verified on disk\n", "fzf");
        assert!(
            out.contains(&expected),
            "a short name must still be padded out to the column width, not \
             just followed by one bare space: {out}"
        );
    }

    // -- render_update -----------------------------------------------------
    //
    // This function produces 100% of what a user of `dotpkg update` reads,
    // and it shipped with no test of its own -- which is exactly how the
    // `Kept` variant's false "kept the previous pin" line (printed even when
    // there was no previous pin at all) reached this file undetected.

    #[test]
    fn each_change_variant_renders_a_distinguishable_line() {
        let u = Update {
            lock: Lock::default(),
            changes: vec![
                Change::Added {
                    backend: SCOOP,
                    name: Name::new("fzf"),
                    version: "0.74.2".into(),
                },
                Change::VersionChanged {
                    backend: SCOOP,
                    name: Name::new("bat"),
                    from: "0.25.0".into(),
                    to: "0.26.1".into(),
                },
                Change::RepinnedSameVersion {
                    backend: SCOOP,
                    name: Name::new("ripgrep"),
                    version: "14.1.0".into(),
                },
                Change::Dropped {
                    backend: SCOOP,
                    name: Name::new("aichat"),
                    version: "0.30.0".into(),
                },
                Change::Kept {
                    backend: SCOOP,
                    name: Name::new("zellij"),
                    version: Some("0.44.3".into()),
                    why: "bucket \"extras\" has no zellij.json".into(),
                },
                Change::Unchanged {
                    backend: SCOOP,
                    name: Name::new("neovim"),
                },
            ],
        };
        let out = render_update(&u);
        assert!(out.contains("+ scoop  fzf"), "Added: {out}");
        assert!(out.contains("^ scoop  bat"), "VersionChanged: {out}");
        assert!(
            out.contains("= scoop  ripgrep"),
            "RepinnedSameVersion: {out}"
        );
        assert!(out.contains("- scoop  aichat"), "Dropped: {out}");
        assert!(out.contains("! scoop  zellij"), "Kept: {out}");
        // Unchanged is the ordinary case and would drown the lines that
        // matter -- it must not get a line of its own, only a count.
        assert!(
            !out.contains("neovim"),
            "an Unchanged package must not get a line of its own: {out}"
        );
    }

    #[test]
    fn a_winget_change_is_labelled_winget_not_hardcoded_scoop() {
        // Before Task 15, `Change` carried no backend at all and every line
        // this function printed said "scoop" unconditionally -- true by
        // construction while `update` only ever resolved scoop, and false
        // the moment it also resolves winget. Every line format is exercised
        // here, not just one, since each used to have its own hardcoded
        // literal.
        let u = Update {
            lock: Lock::default(),
            changes: vec![
                Change::Added {
                    backend: WINGET,
                    name: Name::new("Git.Git"),
                    version: "2.55.0".into(),
                },
                Change::VersionChanged {
                    backend: WINGET,
                    name: Name::new("Brave.Brave"),
                    from: "151.1.93.132".into(),
                    to: "151.1.93.134".into(),
                },
                Change::Dropped {
                    backend: WINGET,
                    name: Name::new("OpenAI.Codex"),
                    version: "0.145.0".into(),
                },
                Change::Kept {
                    backend: WINGET,
                    name: Name::new("BurntSushi.ripgrep.MSVC"),
                    version: Some("15.1.0".into()),
                    why: "version 15.1.0 is no longer in the winget index".into(),
                },
            ],
        };
        let out = render_update(&u);
        assert!(out.contains("+ winget Git.Git"), "Added: {out}");
        assert!(
            out.contains("^ winget Brave.Brave"),
            "VersionChanged: {out}"
        );
        assert!(out.contains("- winget OpenAI.Codex"), "Dropped: {out}");
        assert!(
            out.contains("! winget BurntSushi.ripgrep.MSVC"),
            "Kept: {out}"
        );
        // The counterweight the four `contains` checks above cannot carry on
        // their own: a version that hardcoded "scoop" INSTEAD of reading
        // `backend` would still contain "Git.Git" et al., just on the wrong
        // line -- this is what actually proves the literal word is gone.
        assert!(
            !out.contains("scoop"),
            "nothing here is a scoop package: {out}"
        );
    }

    #[test]
    fn the_repin_line_says_outright_that_apply_will_not_act_on_it() {
        // The whole answer to "does update converge by version or by commit"
        // that a user can actually see. Asserted on the substring that
        // carries the meaning, not merely that the `=` line exists.
        let u = Update {
            lock: Lock::default(),
            changes: vec![Change::RepinnedSameVersion {
                backend: SCOOP,
                name: Name::new("ripgrep"),
                version: "14.1.0".into(),
            }],
        };
        let out = render_update(&u);
        assert!(
            out.contains("apply will not act on this"),
            "the = line must say outright that apply will not act on it: {out}"
        );
    }

    #[test]
    fn kept_with_a_previous_pin_says_so_and_shows_what_is_still_pinned() {
        // The `Kept` line is the only one that could otherwise show no
        // version at all -- and when a package that already had a pin fails
        // to re-resolve, the reader needs to be able to tell what is still
        // installed.
        let u = Update {
            lock: Lock::default(),
            changes: vec![Change::Kept {
                backend: SCOOP,
                name: Name::new("zellij"),
                version: Some("0.44.3".into()),
                why: "bucket \"extras\" has no zellij.json".into(),
            }],
        };
        let out = render_update(&u);
        assert!(
            out.contains("0.44.3"),
            "the version that is still pinned must be visible: {out}"
        );
        assert!(out.contains("kept the previous pin"), "{out}");
    }

    #[test]
    fn kept_with_no_previous_pin_does_not_claim_anything_was_kept() {
        // Regression test for the false line this used to print
        // unconditionally: a brand-new declared package whose FIRST
        // resolution fails (ambiguous bucket, bucket not found, resolve
        // error) has no previous pin at all -- `resolve_into_lock` records
        // this as `version: None`. `tests/update.rs`'s
        // `an_ambiguous_bucket_is_refused_rather_than_guessed_and_names_both_
        // candidates` constructs exactly this state, with `Lock::default()`.
        let u = Update {
            lock: Lock::default(),
            changes: vec![Change::Kept {
                backend: SCOOP,
                name: Name::new("tool"),
                version: None,
                why: "2 declared buckets carry it (main, extras)".into(),
            }],
        };
        let out = render_update(&u);
        assert!(
            !out.contains("kept the previous pin"),
            "nothing was kept -- there was no previous pin to keep: {out}"
        );
        assert!(out.contains("tool"), "{out}");
    }

    #[test]
    fn the_summary_counts_changed_unchanged_and_unresolved_correctly() {
        let u = Update {
            lock: Lock::default(),
            changes: vec![
                Change::Added {
                    backend: SCOOP,
                    name: Name::new("a"),
                    version: "1.0".into(),
                },
                Change::VersionChanged {
                    backend: SCOOP,
                    name: Name::new("b"),
                    from: "1.0".into(),
                    to: "2.0".into(),
                },
                Change::Unchanged {
                    backend: SCOOP,
                    name: Name::new("c"),
                },
                Change::Unchanged {
                    backend: SCOOP,
                    name: Name::new("d"),
                },
                Change::Kept {
                    backend: SCOOP,
                    name: Name::new("e"),
                    version: Some("1.0".into()),
                    why: "no bucket has it".into(),
                },
            ],
        };
        let out = render_update(&u);
        assert!(
            out.contains("2 changed, 2 unchanged, 1 could not be resolved."),
            "{out}"
        );
        // Unchanged is counted above but must not print a line of its own:
        // only the two changed entries and the one Kept entry get a marker
        // line at all.
        let marker_lines = out
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with("+ ")
                    || t.starts_with("^ ")
                    || t.starts_with("= ")
                    || t.starts_with("- ")
                    || t.starts_with("! ")
            })
            .count();
        assert_eq!(
            marker_lines, 3,
            "Unchanged must be counted but not printed: {out}"
        );
    }

    #[test]
    fn the_not_rewritten_line_appears_exactly_when_nothing_was_written() {
        let converged = Update {
            lock: Lock::default(),
            changes: vec![Change::Unchanged {
                backend: SCOOP,
                name: Name::new("fzf"),
            }],
        };
        assert!(
            render_update(&converged).contains("pkg.lock is already current -- not rewritten."),
            "{}",
            render_update(&converged)
        );

        let changed = Update {
            lock: Lock::default(),
            changes: vec![Change::Added {
                backend: SCOOP,
                name: Name::new("fzf"),
                version: "1.0".into(),
            }],
        };
        assert!(
            !render_update(&changed).contains("not rewritten"),
            "{}",
            render_update(&changed)
        );
    }

    #[test]
    fn the_not_rewritten_line_does_not_claim_convergence_when_resolution_failed() {
        // docs/phase3-notes.md item 9, measured on the dogfood machine: no
        // pkg.lock at all, one declared package, and its only resolution
        // failed. `wrote_anything()` is false for the same reason as the
        // genuinely-converged case above -- `Change::Kept` does not count as
        // a write -- but nothing is pinned here at all, so "already current"
        // would tell the reader the opposite of their situation.
        let old = Lock::default();
        let u = Update {
            lock: Lock::default(),
            changes: vec![Change::Kept {
                backend: SCOOP,
                name: Name::new("tool"),
                version: None,
                why: "no declared bucket has it (searched: main)".into(),
            }],
        };
        assert_eq!(old, Lock::default(), "sanity: there was no old pkg.lock");
        assert!(
            u.lock.scoop.is_empty() && u.lock.winget.is_empty(),
            "sanity: nothing is pinned in the resulting lock either"
        );
        assert!(!u.wrote_anything(), "sanity: nothing was written");

        let out = render_update(&u);
        assert!(
            !out.contains("already current"),
            "nothing is pinned -- this is not convergence: {out}"
        );
        assert!(
            out.contains("could not be resolved"),
            "the true reason nothing was written must be named: {out}"
        );
    }

    #[test]
    fn render_adopt_names_the_package_and_which_match_rule_answered() {
        // The two Matched variants read as different strengths of evidence --
        // a user deciding whether to trust an adopted pin needs to see which
        // one fired, not just that adoption "succeeded".
        let out = AdoptOutcome {
            adopted: vec![
                (Name::new("aichat"), Matched::Content, None),
                (Name::new("legacy-tool"), Matched::Version, None),
            ],
            refused: vec![],
            warnings: vec![],
            partial_write: None,
        };
        let text = render_adopt(SCOOP, &out);
        assert!(
            text.contains("aichat")
                && text.contains("the installed manifest matches the bucket exactly"),
            "a Content match must say it matched exactly: {text}"
        );
        assert!(
            text.contains("legacy-tool")
                && text.contains("matched by version only -- the installed manifest differs"),
            "a Version match must say it is the weaker rule: {text}"
        );
    }

    #[test]
    fn render_adopt_labels_a_winget_adoption_winget_and_names_its_own_confirmation_rule() {
        // winget has no commit history to search, so `Matched::WingetConfirmed`
        // is neither `Content` nor `Version` -- it must read as its own
        // sentence, not as either scoop rule, and the line must say "winget",
        // not the hardcoded "scoop" every line used to print.
        let out = AdoptOutcome {
            adopted: vec![(Name::new("Git.Git"), Matched::WingetConfirmed, None)],
            refused: vec![],
            warnings: vec![],
            partial_write: None,
        };
        let text = render_adopt(WINGET, &out);
        assert!(
            text.contains("+ winget Git.Git"),
            "the backend passed in must label the line: {text}"
        );
        assert!(
            text.contains("winget confirms this version is still in its index"),
            "its own confirmation rule, not one of scoop's two: {text}"
        );
        assert!(
            !text.contains("the installed manifest"),
            "winget has no manifest to compare -- neither scoop sentence may \
             appear here: {text}"
        );
    }

    #[test]
    fn render_adopt_names_a_replaced_pin_and_what_it_was() {
        // `adopt_one` does not refuse when `pkg.lock` already pins the name
        // being adopted (the reachable sequence: hand-write `pkg.toml`, run
        // `update`, then `adopt` to hold what is actually installed) -- `run`
        // overwrites that pin unconditionally. Before this fix,
        // `render_adopt` printed the same "+ scoop fzf adopted (...)" line
        // whether or not a committed pin had just been replaced.
        let out = AdoptOutcome {
            adopted: vec![(Name::new("fzf"), Matched::Version, Some("0.74.2".into()))],
            refused: vec![],
            warnings: vec![],
            partial_write: None,
        };
        let text = render_adopt(SCOOP, &out);
        assert!(
            text.contains("replaced the existing pin 0.74.2"),
            "a replaced pin must be named, and what it was: {text}"
        );

        // The counterweight: a fresh adoption with no previous pin must not
        // claim one was replaced. Without this, an unconditional line would
        // satisfy the assertion above and falsely announce a replacement on
        // every ordinary adopt.
        let fresh = AdoptOutcome {
            adopted: vec![(Name::new("aichat"), Matched::Content, None)],
            refused: vec![],
            warnings: vec![],
            partial_write: None,
        };
        let fresh_text = render_adopt(SCOOP, &fresh);
        assert!(
            !fresh_text.contains("replaced"),
            "a first-time adopt has no pin to replace: {fresh_text}"
        );
    }

    #[test]
    fn render_adopt_prints_every_refusal_reason_verbatim() {
        let out = AdoptOutcome {
            adopted: vec![],
            refused: vec![
                (Name::new("nothere"), "nothere is not installed".to_string()),
                (
                    Name::new("aichat"),
                    "no commit in bucket main carries aichat 9.9.9".to_string(),
                ),
            ],
            warnings: vec![],
            partial_write: None,
        };
        let text = render_adopt(SCOOP, &out);
        assert!(text.contains("nothere is not installed"), "{text}");
        assert!(
            text.contains("no commit in bucket main carries aichat 9.9.9"),
            "{text}"
        );
    }

    #[test]
    fn render_adopt_summary_counts_match_the_outcome_and_promises_nothing_else_moved() {
        // The one line a user reads if they read nothing else. It must both
        // total correctly and repeat the promise that adopt never installs or
        // removes anything -- the property the whole command exists to keep.
        let out = AdoptOutcome {
            adopted: vec![(Name::new("aichat"), Matched::Content, None)],
            refused: vec![(Name::new("nothere"), "nothere is not installed".to_string())],
            warnings: vec![],
            partial_write: None,
        };
        let text = render_adopt(SCOOP, &out);
        assert!(
            text.contains("1 adopted, 1 refused. Nothing installed and nothing removed."),
            "{text}"
        );
    }

    #[test]
    fn a_partial_write_names_the_files_that_changed_and_the_files_that_did_not() {
        // The whole point of carrying `partial_write` out of `adopt::run`
        // rather than letting a `?` skip this function: the error names only
        // the file that FAILED, and "which files did this leave changed" is
        // the question the user actually has.
        use crate::adopt::PartialWrite;
        let out = AdoptOutcome {
            partial_write: Some(PartialWrite {
                name: Name::new("aichat"),
                wrote: vec!["pkg.lock", "pkg.toml"],
                why: "cannot create /x/state.json.tmp1234: Permission denied".into(),
            }),
            ..AdoptOutcome::default()
        };
        let text = render_adopt(SCOOP, &out);
        assert!(
            text.contains("changed on disk: pkg.lock, pkg.toml"),
            "name what really changed: {text}"
        );
        assert!(
            text.contains("Not changed: state.json"),
            "and name what did not, so the two lists cannot be confused: {text}"
        );
        assert!(
            text.contains("Permission denied"),
            "the failure itself is still reported: {text}"
        );
        assert!(
            text.contains("The packages after it were not attempted."),
            "a write failure stops the run, unlike a refusal: {text}"
        );

        // The counterweight: an outcome with no partial write must say none of
        // this. Without it, an unconditional block satisfies every assertion
        // above -- and would tell every ordinary `adopt` that a write failed.
        let clean = AdoptOutcome {
            adopted: vec![(Name::new("aichat"), Matched::Content, None)],
            ..AdoptOutcome::default()
        };
        let clean_text = render_adopt(SCOOP, &clean);
        assert!(
            !clean_text.contains("changed on disk") && !clean_text.contains("were not attempted"),
            "a run that wrote everything has no partial write to report: {clean_text}"
        );
    }

    #[test]
    fn render_adopt_of_an_empty_outcome_still_prints_the_zero_summary() {
        let text = render_adopt(SCOOP, &AdoptOutcome::default());
        assert!(
            text.contains("0 adopted, 0 refused. Nothing installed and nothing removed."),
            "{text}"
        );
    }
}
