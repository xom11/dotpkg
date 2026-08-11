use crate::apply::{Outcome, Preparation, Prepared};
use crate::execute::{Execution, ItemResult};
use crate::model::{Name, WINGET};
use crate::plan::{Action, Plan, SkipReason};

/// The plan is the product here: `status` is this and nothing else, and in
/// Phase 2 `apply` prints exactly this before asking for confirmation.
///
/// `show_unmanaged` controls only how `Action::Unmanaged` is rendered, not
/// which actions exist: false (the default a caller should wire up) collapses
/// every backend's `Unmanaged` actions into one line per backend plus a hint
/// to pass this flag; true prints today's one-line-per-package form and no
/// hint. Measured on a14 (`docs/measurements-2026-08-11-phase5-guard-
/// unmanaged-retry.md` §4): a real machine with 0 declared winget packages
/// prints 36 `? winget` lines every run, and that volume -- not a missing
/// dependency vocabulary -- is what this collapses.
pub fn render(plan: &Plan, show_unmanaged: bool) -> String {
    let mut out = String::new();
    // Backend name paired with how many `Action::Unmanaged` entries carried
    // it, built in the same pass as the main loop below rather than by a
    // second scan over `plan.actions` -- so "the order backends first
    // appeared" is the literal iteration order, not one this function
    // re-derives. Only populated, and only consulted, when `show_unmanaged`
    // is false.
    let mut unmanaged_counts: Vec<(String, usize)> = Vec::new();
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
            // **A winget downgrade is announced as the refusal it will be, not
            // as a downgrade.** dotpkg does not downgrade a winget package --
            // decided, not deferred (the design's Non-goals) -- so this run
            // reaches `execute`, fires `install --version <pin>`, and comes back
            // `<id>: installed <x>, pinned <y> -- dotpkg will not downgrade a
            // winget package`, every run, forever. Rendering it as
            // `v ... (downgrade, from lock)` and counting it in `change_count()`
            // put it in the "N change(s)" a user says yes to, and left exit 1 as
            // the only thing that ever told them otherwise. Announcing a
            // downgrade in the plan the user consents to is a different decision
            // from refusing it forever, and only the second one was ever made.
            //
            // The design reasoned no render change was needed because
            // `Divergence::Change { from, to }` carried one shape for both
            // directions and printed no arrow. Phase 4b Task 13 deleted
            // `Divergence`, so that premise expired inside this branch.
            //
            // **Decided here, at the render, and NOT at the planner.** The
            // planner keeps emitting whichever of `Upgrade`/`Downgrade`
            // `plan::is_older` picks, and the step is still built and still
            // fired: winget's own measured refusal is the gate, which is what
            // keeps `is_older` cosmetic. Its own doc comment warns that whoever
            // gates on it "owes it a real version comparison", and winget
            // versions include `v0.2026.07.15.08.55.stable_01`.
            //
            // **The residual, stated rather than hidden:** because `is_older`
            // is cosmetic, it can pick `Downgrade` for a suffixed version pair
            // where the machine is really *behind* its pin -- and then this line
            // predicts a refusal that does not happen, and the package is
            // upgraded instead. That errs toward "we will not act" where the tool
            // then acts, which is the safe direction of the two: the opposite
            // error announces a change and refuses it. It is the same cosmetic
            // answer as before this fix, shown in a place a user can now notice
            // it being wrong.
            Action::Downgrade {
                backend,
                name,
                from,
                to,
                arch,
            } if backend == WINGET => {
                format!(
                    "  ! {backend:<6} {name:<14} {:<24} (dotpkg will not downgrade a winget \
                     package -- run `dotpkg update`{})",
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
                    SkipReason::Opaque => {
                        "installed, but its state could not be read -- see the warnings above"
                            .to_string()
                    }
                    SkipReason::Unscannable => "this backend could not be scanned -- see the \
                         warnings above; nothing was attempted for it"
                        .to_string(),
                };
                format!("  ! {backend:<6} {name:<14} {why}")
            }
            // Per backend, not merged into one running total: `{backend:<6}`
            // is what tells a reader which tool to go look at, and folding
            // scoop and winget into one count would repeat `docs/phase4-
            // notes.md`'s still-open minor about the merged `opaque` list
            // losing its own backend attribution.
            Action::Unmanaged {
                backend,
                name,
                version,
            } => {
                if show_unmanaged {
                    format!("  ? {backend:<6} {name:<14} {version:<24} (unmanaged -- no action)")
                } else {
                    match unmanaged_counts.iter_mut().find(|(b, _)| b == backend) {
                        Some((_, n)) => *n += 1,
                        None => unmanaged_counts.push((backend.clone(), 1)),
                    }
                    // Nothing pushed for this action here: the collapsed line
                    // is emitted once per backend, after the loop, from the
                    // counts just built.
                    continue;
                }
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

    // `unmanaged_counts` is empty whenever `show_unmanaged` is true (the arm
    // above never populates it on that path), so the collapsed lines and the
    // hint never appear alongside the per-line form.
    out.push_str(&unmanaged_collapse_lines(&unmanaged_counts));

    // A plan whose only actions are `Unmanaged` is NOT empty -- `is_empty()`
    // reads `plan.actions` itself, which still holds them regardless of
    // `show_unmanaged` -- so this stays the "nothing happened at all" guard
    // it always was, never mistaking "every action was a report" for "there
    // were no actions".
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
        // Its own clause, for `drift_count`'s reason: `change_count` excludes a
        // winget downgrade because dotpkg has decided never to perform one, and
        // a printed `!` line counted in no number at all would read as
        // "0 change(s), 0 skipped" above a line the user can see.
        if plan.refused_downgrade_count() > 0 {
            summary.push_str(&format!(
                ", {} winget downgrade(s) that will be refused",
                plan.refused_downgrade_count()
            ));
        }
        // Its own clause, for the same reason `drift_count` and
        // `refused_downgrade_count` have one: an `Unmanaged` action is
        // counted by neither `change_count` nor `skip_count`
        // (`Plan::unmanaged_count`'s own doc comment), and this function
        // just collapsed every one of them -- printed lines that used to
        // carry the fact directly -- into the one-line-per-backend form
        // above (or, with `show_unmanaged`, kept them all on screen). Either
        // way, a printed fact accounted for in no number at all would read
        // as "0 change(s), 0 skipped" above a line the user can see.
        if plan.unmanaged_count() > 0 {
            summary.push_str(&format!(", {} unmanaged", plan.unmanaged_count()));
        }
        summary.push('\n');
        out.push_str(&summary);
    }
    out
}

/// One collapsed line per backend, in the order given, plus one hint line --
/// never one hint per backend, however many backends `counts` names --
/// printed only when `counts` is non-empty. Written as one function and
/// called from both `render(plan)` and `render_preparation` rather than
/// copied twice: those two tables were caught by review disagreeing about
/// this exact collapse (`render(plan)` collapsed from the start of this
/// task, `render_preparation` did not, and printed a false hint on a
/// machine's second table while the first had already collapsed correctly)
/// -- sharing this one implementation is what makes that specific
/// disagreement unable to recur between them. `render(plan)` and
/// `render_preparation` build `counts` themselves, each from its own
/// backend-name field on the action shape it walks, and each empties it
/// on the `show_unmanaged == true` path before ever calling this.
fn unmanaged_collapse_lines(counts: &[(String, usize)]) -> String {
    let mut out = String::new();
    for (backend, n) in counts {
        out.push_str(&format!(
            "  ? {backend:<6}   {n} installed outside dotpkg -- no action\n"
        ));
    }
    if !counts.is_empty() {
        out.push_str("      pass --show-unmanaged to list them\n");
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
///
/// `show_unmanaged` means exactly what it means for `render(plan)`, and
/// collapses only `Action::Unmanaged` reports the same way -- **not**
/// `Action::ArchDrift` ones, which print a different marker (`~`, not `?`)
/// and are not the flood this task measured or scoped. Added because a full
/// `apply` run (not just `--prepare`) prints this table right after
/// `render(plan)`'s own, and until this fix `render(plan)` collapsed while
/// this function still printed one line per `Unmanaged` package regardless
/// of the flag -- so a real run showed a collapsed line and its hint from
/// the first table, then every individual line the hint had just said the
/// flag would restore, from the second. See this task's review for the
/// measured transcript.
pub fn render_preparation(p: &Preparation, show_unmanaged: bool) -> String {
    let mut out = String::new();
    // Same shape as `render(plan)`'s own `unmanaged_counts`, and for the same
    // reason: built in the same pass as the loop below, so backend order is
    // the literal iteration order over `p.prepared`, not one re-derived
    // after the fact.
    let mut unmanaged_counts: Vec<(String, usize)> = Vec::new();
    if p.prepared.is_empty() {
        out.push_str("  nothing to prepare\n");
    } else {
        for item in &p.prepared {
            // Only `Action::Unmanaged` under `Outcome::Report` collapses --
            // matched on both together, not on the action alone, so a
            // hypothetical future `Unmanaged` outcome that is not a bare
            // report (there is none today) would fall through to
            // `prepared_line` rather than being silently swallowed here.
            if !show_unmanaged {
                if let (Action::Unmanaged { backend, .. }, Outcome::Report) =
                    (&item.action, &item.outcome)
                {
                    match unmanaged_counts.iter_mut().find(|(b, _)| b == backend) {
                        Some((_, n)) => *n += 1,
                        None => unmanaged_counts.push((backend.clone(), 1)),
                    }
                    continue;
                }
            }
            out.push_str(&prepared_line(item));
        }
        out.push_str(&unmanaged_collapse_lines(&unmanaged_counts));
    }
    out.push('\n');
    if !p.prepared.is_empty() {
        // A winget `Downgrade`'s `ReadyToSet` is subtracted out of the
        // printed "ready" number here even though `Preparation::ready_count`
        // itself still counts it -- that method's own doc comment says why
        // it must. This is the number a user actually reads, and it must
        // agree with the `!` line `prepared_line` just printed for the same
        // package, not with an internal bookkeeping count meant for a
        // different question (`main.rs`'s routing-bug check).
        let refused = p.refused_winget_downgrade_count();
        let ready = p.ready_count().saturating_sub(refused);
        let mut summary = format!(
            "  {} of {} changes ready, {} failed, {} skipped, {} not locked",
            ready,
            ready + p.failed_count(),
            p.failed_count(),
            p.skipped_count(),
            p.not_locked_count(),
        );
        // Its own clause, matching `render(plan)`'s summary for the same
        // reason that one has it: `ready` above excludes it, and a printed
        // `!` line accounted for in no number at all would read as "N of N
        // ready" with a refusal sitting silently inside N.
        if refused > 0 {
            summary.push_str(&format!(
                ", {refused} winget downgrade(s) that will be refused"
            ));
        }
        summary.push_str(".\n");
        out.push_str(&summary);
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
    // `{backend:<6}` read from the item, never the literal `scoop` this
    // function printed on every line until Phase 4b Task 13. That literal was
    // true while no `Step::Winget` could reach `execute`, and became a
    // `FAILED  scoop  Brave.Brave` line the day winget got an executor. See
    // `execute::ItemOutcome`'s own doc comment.
    for item in &ex.results {
        let (backend, name) = (&item.backend, &item.name);
        let line = match &item.result {
            // `{name:<14} ` -- a literal space after the padded field, the
            // same fix `prepared_line` needed and got at Task 16
            // (`src/render.rs:303`; see its own doc comment for the
            // mechanism). This function used the narrower `{name:<13}` with
            // no literal separator, the identical shape that turned into
            // a real, dogfood-found glued-together line at `prepared_line`
            // -- one column short, and reachable here the same way, with an
            // ordinary scoop package: `windows-terminal` is 16 characters,
            // long enough to exhaust the padding entirely and leave nothing
            // to separate it from what follows.
            ItemResult::Done => {
                format!("  done    {backend:<6} {name:<14} verified on disk")
            }
            ItemResult::Failed { why, .. } => {
                format!("  FAILED  {backend:<6} {name:<14} {why}")
            }
            ItemResult::Held(why) => format!("  held    {backend:<6} {name:<14} {why}"),
        };
        out.push_str(&line);
        out.push('\n');
    }
    // The same read-it-from-the-item fix, applied to the `note` line for
    // completeness rather than to correct anything a user saw: nothing
    // reconciled winget until Phase 4b Task 8, so no winget ownership record
    // was ever dropped before this branch, and none was ever printed as
    // scoop's. Task 8 gave the winget half a backend on day one.
    for (backend, name) in &ex.dropped_ghosts {
        out.push_str(&format!(
            "  note    {backend:<6} {name:<14} ownership record dropped: nothing by that name is installed\n"
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
        // Every ready shape prints the same `ready` marker and the same
        // right-hand side, because the right-hand side describes the ACTION,
        // not how it was prepared: a winget `ReadyToSet` renders "1.2.3
        // (install)" or "1.2.3 -> 1.2.4 (upgrade)" exactly as a scoop
        // `ReadyToFetch` does. What was fetched, staged or merely confirmed is
        // dotpkg's business, not something a user reads a table to learn.
        Outcome::ReadyToFetch { .. } | Outcome::ReadyToRemove => {
            ("ready", ready_rest(&item.action))
        }
        // **A winget `Downgrade`'s `ReadyToSet` is not this table's idea of
        // ready.** The pin really is live in winget's index -- that is what
        // the outcome means, and `check_pin_is_live` has no way to know it
        // is a downgrade, so it must not guess -- but `execute` fires
        // `install --version <pin>` and winget refuses it, every run,
        // forever (`Plan::change_count`'s own doc comment). Post-merge audit
        // I2: this arm used to fall into the `ready` case above and
        // `--prepare` printed `ready … (downgrade)` and exited 0 for a
        // package `apply` is guaranteed to fail on. Now says the identical
        // words `render(plan)`'s own `Action::Downgrade` arm does, not a
        // paraphrase, so a user reading both tables for one run sees one
        // sentence, not two.
        Outcome::ReadyToSet { .. } if matches!(&item.action, Action::Downgrade { backend, .. } if backend == WINGET) => {
            ("!", refused_downgrade_rest(&item.action))
        }
        Outcome::ReadyToSet { .. } => ("ready", ready_rest(&item.action)),
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
/// ready outcome for these four action shapes (`ReadyToFetch` for a scoop
/// install/upgrade/downgrade, `ReadyToSet` for a winget one, `ReadyToRemove`
/// for either backend's `Prune`), so the fallback below is unreachable in
/// practice; it stays total rather than panicking if that ever changes.
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

/// The right-hand side of a winget `Downgrade`'s refusal line in this
/// table -- deliberately the same words `render(plan)`'s own
/// `Action::Downgrade` arm uses, not a paraphrase, so a user who reads both
/// tables for one run sees one sentence about one refusal, not two. Its own
/// function rather than a shared one, because the two tables are sized
/// differently (`{:<18}` here, matching `ready_rest`; `{:<24}` there) and a
/// string shared across both would misalign one of them.
fn refused_downgrade_rest(action: &Action) -> String {
    match action {
        // `{:<18} ` -- a literal space after the padded field, `prepared_line`'s
        // own convention and for the same reason: `{from} -> {to}` easily
        // exceeds 18 characters (Brave.Brave's real shape,
        // `151.1.93.134 -> 151.1.93.132`, is 29), and `{:<N}` inserts no
        // minimum gap once the value is already that wide -- it would glue
        // the closing version straight onto `(dotpkg` with nothing between.
        Action::Downgrade { from, to, arch, .. } => format!(
            "{:<18} (dotpkg will not downgrade a winget package -- run `dotpkg update`{})",
            format!("{from} -> {to}"),
            arch_suffix(arch)
        ),
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
        assert!(render(&Plan::default(), false).contains("nothing to do"));
    }

    // -- Unmanaged collapsing ------------------------------------------------

    #[test]
    fn thirty_six_unmanaged_winget_packages_collapse_to_one_line_per_backend() {
        // 36 is the measured count on a14 (`docs/measurements-2026-08-11-
        // phase5-guard-unmanaged-retry.md` §4), and the fixture carries 36
        // real entries: a `vec![]` or a one-entry plan cannot tell the
        // collapsed form from the per-line form at all.
        //
        // Scoop pushed first, winget second -- matching the order `plan()`
        // actually produces (`plan::plan`'s `backends` array is
        // `[SCOOP, WINGET]`, and its one `actions.extend(reports)` appends
        // each backend's `Unmanaged` reports in the order that backend's
        // view ran, not the order this test happens to build them in). A
        // fixture that instead pushed winget first, as an earlier version of
        // this test did, cannot tell "collapsed lines follow this backend
        // order" from "collapsed lines follow whatever order the fixture
        // used" -- reviewed and named as a gap: this task's own new tests
        // were the only fixtures in the crate asserting an order production
        // cannot produce.
        let mut plan = Plan::default();
        for i in 0..6 {
            plan.actions.push(Action::Unmanaged {
                backend: SCOOP.to_string(),
                name: Name::new(format!("app{i}")),
                version: "1.0".to_string(),
            });
        }
        for i in 0..36 {
            plan.actions.push(Action::Unmanaged {
                backend: WINGET.to_string(),
                name: Name::new(format!("Vendor.Pkg{i}")),
                version: "1.0".to_string(),
            });
        }
        let out = render(&plan, false);
        assert!(
            out.contains("? winget   36 installed outside dotpkg"),
            "was:\n{out}"
        );
        assert!(
            out.contains("? scoop    6 installed outside dotpkg"),
            "was:\n{out}"
        );
        // The order claim, pinned rather than left as a comment only
        // `plan()` proves: scoop's collapsed line must come first, matching
        // the backend order production actually emits, not merely the order
        // this fixture happens to build actions in.
        let scoop_at = out.find("? scoop").expect("scoop line present");
        let winget_at = out.find("? winget").expect("winget line present");
        assert!(
            scoop_at < winget_at,
            "scoop's collapsed line must precede winget's, matching plan()'s \
             own backend order: {out}"
        );
        assert!(out.contains("--show-unmanaged"), "was:\n{out}");
        // The hint is printed once for the whole run, never once per
        // backend, even though two backends are collapsed here -- a mutant
        // that moved this push inside the per-backend loop would satisfy
        // every `contains` check above while printing it twice.
        assert_eq!(
            out.matches("--show-unmanaged").count(),
            1,
            "one hint total, not one per backend: {out}"
        );
        // Collapsed means collapsed: no individual id survives.
        assert!(!out.contains("Vendor.Pkg17"), "was:\n{out}");
        assert!(!out.contains("app3"), "was:\n{out}");
        // The clause is mandatory. `change_count` counts an Unmanaged as
        // nothing, so without it 42 printed facts sit under "0 change(s), 0
        // skipped" -- the exact shape `refused_downgrade_count` earned its own
        // clause to avoid.
        assert!(
            out.contains("0 change(s), 0 skipped, 42 unmanaged"),
            "was:\n{out}"
        );
    }

    #[test]
    fn show_unmanaged_restores_every_line_and_drops_the_hint() {
        let mut plan = Plan::default();
        for i in 0..36 {
            plan.actions.push(Action::Unmanaged {
                backend: WINGET.to_string(),
                name: Name::new(format!("Vendor.Pkg{i}")),
                version: "1.0".to_string(),
            });
        }
        let out = render(&plan, true);
        assert!(out.contains("Vendor.Pkg17"), "was:\n{out}");
        assert_eq!(
            out.lines()
                .filter(|l| l.contains("(unmanaged -- no action)"))
                .count(),
            36
        );
        assert!(!out.contains("--show-unmanaged"), "was:\n{out}");
        // The clause stays: the count is true in both forms.
        assert!(out.contains("36 unmanaged"), "was:\n{out}");
    }

    #[test]
    fn a_single_unmanaged_package_is_still_collapsed_so_there_is_one_shape_not_two() {
        // Deliberate: no threshold. A threshold is a magic number and gives the
        // output two shapes a reader has to learn.
        let mut plan = Plan::default();
        plan.actions.push(Action::Unmanaged {
            backend: WINGET.to_string(),
            name: Name::new("Vendor.One"),
            version: "1.0".to_string(),
        });
        let out = render(&plan, false);
        assert!(
            out.contains("? winget   1 installed outside dotpkg"),
            "was:\n{out}"
        );
        assert!(!out.contains("Vendor.One"), "was:\n{out}");
        assert!(
            out.contains("0 change(s), 0 skipped, 1 unmanaged"),
            "was:\n{out}"
        );
    }

    #[test]
    fn a_plan_with_no_unmanaged_packages_gains_no_clause_and_no_line() {
        let mut plan = Plan::default();
        plan.actions.push(Action::Install {
            backend: WINGET.to_string(),
            name: Name::new("Git.Git"),
            version: "2.0".to_string(),
            arch: None,
        });
        let out = render(&plan, false);
        assert!(!out.contains("unmanaged"), "was:\n{out}");
        assert!(out.contains("1 change(s), 0 skipped"), "was:\n{out}");
    }

    #[test]
    fn a_plan_whose_only_actions_are_unmanaged_is_not_reported_as_nothing_to_do() {
        // `plan.actions.is_empty()` gates the "nothing to do" line. A plan
        // built entirely from `Unmanaged` actions is not empty -- it holds
        // 36 of them -- so this must print the collapsed line and the
        // summary clause, never "nothing to do".
        let mut plan = Plan::default();
        for i in 0..36 {
            plan.actions.push(Action::Unmanaged {
                backend: WINGET.to_string(),
                name: Name::new(format!("Vendor.Pkg{i}")),
                version: "1.0".to_string(),
            });
        }
        let out = render(&plan, false);
        assert!(
            !out.contains("nothing to do"),
            "36 unmanaged reports are not nothing: {out}"
        );
        assert!(
            out.contains("? winget   36 installed outside dotpkg"),
            "was:\n{out}"
        );
        assert!(
            out.contains("0 change(s), 0 skipped, 36 unmanaged"),
            "was:\n{out}"
        );
    }

    // -- render_preparation ---------------------------------------------

    #[test]
    fn the_preparation_table_does_not_promise_anything_about_mutations() {
        // `apply` prints this same table before it starts changing things, so
        // the promise cannot live here. main.rs prints it in the --prepare
        // branch, and tests/cli.rs asserts it appears there.
        let out = render_preparation(&Preparation::default(), false);
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
        assert_eq!(render_preparation(&p, false), expected);
    }

    #[test]
    fn a_refused_winget_downgrade_is_not_printed_as_ready_by_prepare() {
        // I2 (post-merge audit): `apply --prepare` printed `ready …
        // (downgrade)` and exited 0 for a package `apply` is guaranteed to
        // fail on, every run, forever -- the same shape
        // `a_winget_downgrade_is_announced_as_a_refusal_and_is_not_counted_
        // as_a_change` fixed for `render(plan)`, but `render_preparation`
        // reads a `Preparation` built from `check_pin_is_live`'s
        // `Outcome::ReadyToSet` (correct: the pin really is live in winget's
        // index), never from `Plan::change_count`'s exclusion, so fixing the
        // plan line never touched this table at all.
        let p = Preparation {
            prepared: vec![Prepared {
                action: Action::Downgrade {
                    backend: WINGET.into(),
                    name: "Brave.Brave".into(),
                    from: "151.1.93.134".into(),
                    to: "151.1.93.132".into(),
                    arch: None,
                },
                outcome: Outcome::ReadyToSet {
                    id: Name::new("Brave.Brave"),
                    version: "151.1.93.132".into(),
                },
            }],
        };
        let out = render_preparation(&p, false);
        assert!(
            out.lines().next().is_some_and(|l| l.starts_with("  !")),
            "a package winget is guaranteed to refuse must not be marked \
             ready: {out}"
        );
        assert!(
            !out.contains("ready   winget"),
            "no winget row in this run may be called ready: {out}"
        );
        assert!(
            out.contains("will not downgrade"),
            "this table must say what render(plan) already says about the \
             identical action: {out}"
        );
        assert!(
            out.contains("0 of 0 changes ready"),
            "the summary must agree with the per-row line it just printed, \
             not count the refusal as one of its own \"ready\" changes: {out}"
        );
        assert!(
            out.contains("1 winget downgrade(s) that will be refused"),
            "the summary must account for the line it just printed, the \
             same as render(plan)'s own summary: {out}"
        );

        // The counterweight: a genuine winget install must still be counted
        // and printed as ready.
        let genuine = Preparation {
            prepared: vec![Prepared {
                action: Action::Install {
                    backend: WINGET.into(),
                    name: "Git.Git".into(),
                    version: "2.52.0".into(),
                    arch: None,
                },
                outcome: Outcome::ReadyToSet {
                    id: Name::new("Git.Git"),
                    version: "2.52.0".into(),
                },
            }],
        };
        let genuine_out = render_preparation(&genuine, false);
        assert!(
            genuine_out.contains("  ready   winget"),
            "a real install must stay ready: {genuine_out}"
        );
        assert!(
            genuine_out.contains("1 of 1 changes ready"),
            "and stay counted: {genuine_out}"
        );
    }

    #[test]
    fn a_name_at_least_14_characters_long_still_gets_a_separator_before_its_rest() {
        // The dogfood-found shape, reproduced directly: a real winget id long
        // enough to exhaust `{name:<14}`'s padding entirely -- nothing left to
        // pad with, so the literal space after it is the only thing that still
        // separates it from what follows. Without that literal space, this
        // renders as "...JanDeDobbeleer.OhMyPosh29.36.0.0 ->..." -- exactly
        // what a14's real `apply --prepare` output showed.
        //
        // It was a report-only skip when the dogfood found it, because that was
        // all a winget difference could be; Phase 4b Task 13 made the same difference a
        // real `Upgrade` prepared to `ReadyToSet`. The name and the column are
        // what this test is about, so it follows that shape rather than keeping
        // a deleted one alive.
        let p = Preparation {
            prepared: vec![Prepared {
                action: Action::Upgrade {
                    backend: WINGET.into(),
                    name: Name::new("JanDeDobbeleer.OhMyPosh"),
                    from: "29.36.0.0".into(),
                    to: "30.6.3".into(),
                    arch: None,
                },
                outcome: Outcome::ReadyToSet {
                    id: Name::new("JanDeDobbeleer.OhMyPosh"),
                    version: "30.6.3".into(),
                },
            }],
        };
        let out = render_preparation(&p, false);
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
        let out = render_preparation(&p, false);
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
        let out = render_preparation(&p, false);
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
        let out = render_preparation(&p, false);
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
        let out = render_preparation(&p, false);
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
        let out = render_preparation(&p, false);
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
        // `true`: this test reads the individual `?       scoop  antigravity`
        // line below, which the default (`false`) collapses away.
        let out = render_preparation(&p, true);
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

    // -- render_preparation's own Unmanaged collapse (review Important 1) --
    //
    // `render(plan)` collapsed from the start of this task; `render_
    // preparation` did not, because the brief that started this task never
    // named it. Review measured the real binary printing a collapsed line
    // and its hint from the plan table, then every individual line the hint
    // had just promised the flag would restore, from the preparation table
    // two lines below -- on a full `apply` run, both tables print. These
    // tests are `render_preparation`'s side of the same guarantee
    // `thirty_six_unmanaged_winget_packages_collapse_to_one_line_per_
    // backend` pins for `render(plan)`.

    fn unmanaged_report(backend: &str, name: &str) -> Prepared {
        Prepared {
            action: Action::Unmanaged {
                backend: backend.to_string(),
                name: Name::new(name),
                version: "1.0".to_string(),
            },
            outcome: Outcome::Report,
        }
    }

    #[test]
    fn render_preparation_collapses_unmanaged_reports_by_default_and_restores_them_with_the_flag() {
        let p = Preparation {
            prepared: vec![
                unmanaged_report(SCOOP, "app0"),
                unmanaged_report(SCOOP, "app1"),
                unmanaged_report(SCOOP, "app2"),
            ],
        };
        let collapsed = render_preparation(&p, false);
        assert!(
            collapsed.contains("? scoop    3 installed outside dotpkg"),
            "was:\n{collapsed}"
        );
        assert!(collapsed.contains("--show-unmanaged"), "was:\n{collapsed}");
        assert!(
            !collapsed.contains("app0"),
            "collapsed means collapsed here too: {collapsed}"
        );
        // A `Preparation` whose only items are `Unmanaged` reports is not
        // empty -- `p.prepared.is_empty()` is what gates "nothing to
        // prepare", and it still holds 3 items -- so this must not claim
        // there was nothing to prepare.
        assert!(
            !collapsed.contains("nothing to prepare"),
            "3 unmanaged reports are not nothing: {collapsed}"
        );

        let shown = render_preparation(&p, true);
        assert!(shown.contains("app0"), "was:\n{shown}");
        assert_eq!(
            shown
                .lines()
                .filter(|l| l.contains("(unmanaged -- no action)"))
                .count(),
            3
        );
        assert!(!shown.contains("--show-unmanaged"), "was:\n{shown}");
    }

    #[test]
    fn render_preparation_never_collapses_arch_drift_alongside_unmanaged() {
        // Important 1's own precision requirement: `Outcome::Report` covers
        // `Action::ArchDrift` as well as `Action::Unmanaged`, and drift
        // prints its own `~` marker and is not the flood this task measured
        // or scoped. A version that collapsed every `Report`, not just
        // `Unmanaged` ones, would still pass every unmanaged-only assertion
        // above.
        let p = Preparation {
            prepared: vec![
                unmanaged_report(SCOOP, "app0"),
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
        let out = render_preparation(&p, false);
        assert!(
            out.contains("? scoop    1 installed outside dotpkg"),
            "the one Unmanaged report still collapses: {out}"
        );
        assert!(
            out.contains("~       scoop  python"),
            "the ArchDrift report must still print its own line, uncollapsed: {out}"
        );
        assert!(
            out.contains("(architecture drift -- reported, not fixed)"),
            "was:\n{out}"
        );
    }

    #[test]
    fn render_preparation_prints_the_hint_once_across_two_backends_not_once_per_backend() {
        // The `render_preparation` half of Minor 4: `unmanaged_collapse_
        // lines` is the one function both tables call, so this and
        // `thirty_six_unmanaged_winget_packages_collapse_to_one_line_per_
        // backend` together prove the property from both call sites --
        // asserted with `.matches(..).count()`, not `contains`, which a
        // duplicated hint would also satisfy.
        let p = Preparation {
            prepared: vec![
                unmanaged_report(SCOOP, "app0"),
                unmanaged_report(WINGET, "Vendor.Pkg0"),
            ],
        };
        let out = render_preparation(&p, false);
        assert!(
            out.contains("? scoop    1 installed outside dotpkg"),
            "{out}"
        );
        assert!(
            out.contains("? winget   1 installed outside dotpkg"),
            "{out}"
        );
        assert_eq!(
            out.matches("--show-unmanaged").count(),
            1,
            "one hint total, not one per backend: {out}"
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
                // A winget skip, to prove the `!` marker is not scoop-only.
                // It was a report-only skip until Phase 4b Task 13; a declared winget
                // package with no lock entry is the shape that still produces
                // one, and it is now spelled exactly as scoop's is.
                Action::Skip {
                    backend: WINGET.into(),
                    name: "Git.Git".into(),
                    reason: SkipReason::NotLocked,
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
        // `true`: this test reads the individual `? scoop  antigravity` line
        // below, which the default (`false`) collapses away.
        let out = render(&plan, true);
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
        let out = render(&plan, false);
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
        let out = render(&plan, false);
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
        let out = render(&plan, false);
        assert!(out.contains("(install)"), "got: {out}");
        assert!(out.contains("(upgrade)"), "got: {out}");
        assert!(out.contains("(downgrade, from lock)"), "got: {out}");
        assert!(
            !out.contains("arm64"),
            "no architecture was resolved, so none should appear: {out}"
        );
    }

    #[test]
    fn a_winget_package_that_differs_from_the_lock_is_a_real_upgrade_and_never_says_reported_only()
    {
        // **This reverses a test**, name and all: it was
        // `..._prints_both_versions_and_says_reported_only`, and it asserted
        // `out.contains("reported only")` on exactly this input. That sentence
        // was true for as long as winget had no executor and became false the
        // moment it got one, so the assertion is inverted rather than deleted
        // -- the string must now be absent from the very plan that used to
        // require it, which is a stronger guard than the separate
        // scoop-only counterweight this replaces
        // (`a_plan_with_no_winget_divergence_never_prints_reported_only`,
        // deleted: it asserted the absence of a string on a plan that never
        // had a reason to contain it, and no code path can produce that string
        // for any plan now).
        //
        // What has NOT changed is the reason the diff is rendered at all: both
        // versions must be visible, not just the fact that one exists. Hiding
        // that Brave is `151.1 -> 151.2` throws away the whole product of
        // `status`.
        let plan = Plan {
            actions: vec![Action::Upgrade {
                backend: WINGET.into(),
                name: "Brave.Brave".into(),
                from: "151.1.93.132".into(),
                to: "151.1.93.134".into(),
                arch: None,
            }],
        };
        let out = render(&plan, false);
        assert!(out.contains("^ winget Brave.Brave"), "got: {out}");
        assert!(
            out.contains("151.1.93.132") && out.contains("151.1.93.134"),
            "both versions must be visible, not just \"differs\": {out}"
        );
        assert!(
            !out.contains("reported only"),
            "dotpkg installs winget packages now: {out}"
        );
        assert!(
            !out.contains("cannot install"),
            "the four sentences that said this were deleted with \
             `Divergence`; none may come back through any other path: {out}"
        );
        assert_eq!(plan.change_count(), 1, "and it counts as a change");
    }

    #[test]
    fn a_winget_downgrade_is_announced_as_a_refusal_and_is_not_counted_as_a_change() {
        // **The plan must not promise something the executor has decided never
        // to do.** An ahead-of-pin winget package -- the measured `Brave.Brave`
        // shape, installed 151.1.93.134 against an index newest of
        // 151.1.93.132 -- reaches `execute` and comes back
        // "dotpkg will not downgrade a winget package", by design and forever.
        // Rendering it as `v ... (downgrade, from lock)` and counting it in
        // `change_count()` puts it in the "N change(s)" the user consents to,
        // and exit 1 is then the only thing that tells them otherwise.
        //
        // The design reasoned this was safe because `Divergence::Change { from,
        // to }` printed no arrow and carried one shape for both directions.
        // Phase 4b Task 13 deleted `Divergence`, so that premise expired inside
        // this branch and the decision it rested on was never made.
        //
        // Fixed at the render and the count, never at the planner: gating on
        // `plan::is_older` would promote a function whose own doc comment says
        // it is cosmetic, and winget versions include
        // `v0.2026.07.15.08.55.stable_01`.
        let plan = Plan {
            actions: vec![Action::Downgrade {
                backend: WINGET.into(),
                name: "Brave.Brave".into(),
                from: "151.1.93.134".into(),
                to: "151.1.93.132".into(),
                arch: None,
            }],
        };
        let out = render(&plan, false);
        assert!(
            out.contains("  ! winget"),
            "a run that will be refused is not a change dotpkg is about to make: {out}"
        );
        assert!(
            !out.contains("(downgrade, from lock)"),
            "scoop's downgrade wording announces a downgrade this backend will \
             not perform: {out}"
        );
        assert!(
            out.contains("will not downgrade"),
            "the line must say what will actually happen: {out}"
        );
        assert!(
            out.contains("151.1.93.134") && out.contains("151.1.93.132"),
            "both versions still visible -- that is the whole product of `status`: {out}"
        );
        assert_eq!(
            plan.change_count(),
            0,
            "a refusal is not a change, and must not enter the number the user \
             says yes to"
        );
        // But it must be accounted for SOMEWHERE: a printed line counted in no
        // number reads as "0 change(s), 0 skipped" above a line the user can see.
        assert!(
            out.contains("0 change(s), 0 skipped, 1 winget downgrade(s) that will be refused"),
            "the summary must account for the line it just printed: {out}"
        );

        // The counterweight: scoop's downgrade really happens, and its line and
        // its count must be untouched by this.
        let scoop_plan = Plan {
            actions: vec![Action::Downgrade {
                backend: SCOOP.into(),
                name: "fzf".into(),
                from: "0.74.2".into(),
                to: "0.74.1".into(),
                arch: None,
            }],
        };
        let scoop_out = render(&scoop_plan, false);
        assert!(
            scoop_out.contains("  v scoop") && scoop_out.contains("(downgrade, from lock)"),
            "scoop downgrades are real and stay announced as downgrades: {scoop_out}"
        );
        assert_eq!(
            scoop_plan.change_count(),
            1,
            "and they stay counted -- this fix is scoped to winget"
        );
    }

    #[test]
    fn a_declared_unlocked_winget_package_is_told_to_run_dotpkg_update() {
        // Two reversals, in order. Task 15 made `update::run` resolve winget
        // packages too (`src/update.rs:403-459`), which reversed
        // `a_declared_unlocked_winget_package_is_not_told_to_run_update` into
        // this test. Phase 4b Task 13 then changed what shape carries the message:
        // `SkipReason::ReportedOnly(Divergence::NotLocked)` existed only
        // because refusing the whole run over a missing pin helped nobody for
        // a backend that could not act anyway. Winget acts now, so the planner
        // emits plain `SkipReason::NotLocked` and this renders exactly as
        // scoop's does.
        //
        // Still its own test rather than folded into
        // `a_skip_says_what_to_do_about_it`: the advice must reach a WINGET
        // package, and for two phases it deliberately did not.
        let plan = Plan {
            actions: vec![Action::Skip {
                backend: WINGET.into(),
                name: "Git.Git".into(),
                reason: SkipReason::NotLocked,
            }],
        };
        let out = render(&plan, false);
        assert!(out.contains("! winget Git.Git"), "got: {out}");
        assert!(
            out.contains("dotpkg update"),
            "update can resolve a winget package's version, so the \
             fix must be named: {out}"
        );
        // Paired with the assertion above rather than left standing alone: a
        // regression that kept "run `dotpkg update`" but also brought back a
        // "cannot" claim would still print a false line, and would slip past
        // the assertion above by itself.
        assert!(
            !out.contains("cannot resolve") && !out.contains("cannot install"),
            "nothing here is impossible any more -- the text must not claim \
             otherwise: {out}"
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
        assert!(render(&plan, false).contains("dotpkg update"));
    }

    // -- render_execution --------------------------------------------------

    /// An `ItemOutcome` for a scoop package. Most tests below are about the
    /// summary wording or the name column and are scoop-only; the backend
    /// column has its own test.
    fn scoop_item(name: &str, result: ItemResult) -> crate::execute::ItemOutcome {
        crate::execute::ItemOutcome {
            backend: SCOOP.to_string(),
            name: Name::new(name),
            result,
        }
    }

    #[test]
    fn the_summary_never_claims_more_than_the_run_verified() {
        let ex = Execution {
            results: vec![
                scoop_item("bat", ItemResult::Done),
                scoop_item(
                    "fzf",
                    ItemResult::Failed {
                        why: "install did not happen".into(),
                        touched: false,
                    },
                ),
                scoop_item("kanata", ItemResult::Held("started running".into())),
            ],
            dropped_ghosts: vec![(SCOOP.to_string(), Name::new("stale"))],
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
            results: vec![scoop_item(
                "fzf",
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
        // render.rs:286's `changed() > 0 || touched() > 0` looked like it
        // might be an equivalent mutant under `>` -> `<`, reasoning that
        // `touched()` is a superset of `changed()` (the comment above the
        // line argues `touched()` catches cases `changed()` misses). But the
        // two sets are disjoint, not nested: `changed()` counts `Done`;
        // `touched()` counts `Failed { touched: true }` (`execute.rs`'s
        // `Execution::changed`/`touched`). `touched() >= changed()` only
        // holds when `changed() == 0`.
        //
        // This is the case where it does not: one `Done` and one
        // `Failed { touched: false }` (reachable -- `ScoopStep::Remove`'s
        // uninstall-command-failed arm, `execute.rs:383-388`, never touches
        // the machine -- post-merge audit M1: this citation had drifted to
        // `:328`, inside a different arm entirely whose `touched` is a
        // mutable variable that CAN be `true`, a counterexample to this very
        // sentence; the sentence was right, the line number had rotted).
        // `changed() == 1`, `touched() == 0`. The real code's
        // `changed() > 0` disjunct is true and prints the "some changed"
        // sentence. The `>` -> `<` mutant compares a `usize` against 0 with
        // `<`, which is always false, so both disjuncts are permanently
        // false and it prints "Nothing was changed" instead -- a false
        // claim about a machine that just gained a package.
        let ex = Execution {
            results: vec![
                scoop_item("bat", ItemResult::Done),
                scoop_item(
                    "fzf",
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
            results: vec![scoop_item("windows-terminal", ItemResult::Done)],
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
            results: vec![scoop_item("fzf", ItemResult::Done)],
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

    #[test]
    fn every_execution_line_names_the_backend_that_actually_acted_never_a_hardcoded_scoop() {
        // **The bug this test exists for shipped, briefly, inside Phase 4b Task
        // 13's own commit.** `render_execution` hardcoded the literal `scoop`
        // on all four of its line shapes. That was correct for as long as no
        // `Step::Winget` could reach `execute`: under `Capability::ReportsOnly`
        // a winget difference was `Action::Skip { reason: ReportedOnly }` ->
        // `Intent::Skip` -> `Outcome::Skipped`, routed into `unusable` by
        // `plan_to_steps`'s `Skipped` arm -- never a step, and never the
        // routing-bug arm either, which needs a ready outcome and was reachable
        // only from a hand-built `Preparation`. The very change that gave winget
        // an executor turned those lines into `FAILED  scoop  Brave.Brave`.
        //
        // No grep for a deleted sentence could have found it, because the false
        // word was a backend name, which is why this asserts the column
        // directly for every shape rather than trusting the plumbing.
        //
        // The `note` line is here for completeness, not because a user ever saw
        // it wrong: `reconcile_ghosts` reconciled scoop alone until Phase 4b
        // Task 8, so no winget ownership record was ever dropped before this
        // branch.
        let winget_item = |name: &str, result: ItemResult| crate::execute::ItemOutcome {
            backend: WINGET.to_string(),
            name: Name::new(name),
            result,
        };
        let ex = Execution {
            results: vec![
                winget_item("Brave.Brave", ItemResult::Done),
                winget_item(
                    "7zip.7zip",
                    ItemResult::Failed {
                        why: "winget install exited 1".into(),
                        touched: false,
                    },
                ),
                winget_item("Discord.Discord", ItemResult::Held("still running".into())),
                // One scoop line in the same table, so a mutant that hardcodes
                // `winget` instead of reading the field cannot pass either.
                scoop_item("fzf", ItemResult::Done),
            ],
            dropped_ghosts: vec![(WINGET.to_string(), Name::new("OpenAI.Codex"))],
            ..Default::default()
        };
        let out = render_execution(&ex);
        for expected in [
            format!(
                "  done    {:<6} {:<14} verified on disk",
                WINGET, "Brave.Brave"
            ),
            format!(
                "  FAILED  {:<6} {:<14} winget install exited 1",
                WINGET, "7zip.7zip"
            ),
            format!(
                "  held    {:<6} {:<14} still running",
                WINGET, "Discord.Discord"
            ),
            format!(
                "  note    {:<6} {:<14} ownership record",
                WINGET, "OpenAI.Codex"
            ),
            format!("  done    {:<6} {:<14} verified on disk", SCOOP, "fzf"),
        ] {
            assert!(out.contains(&expected), "missing {expected:?} in:\n{out}");
        }
        // The negative control the four assertions above cannot give on their
        // own: `{:<6}` pads `winget` to exactly its own width, so a line that
        // still said `scoop` would fail them -- but only because the name
        // column would shift. This says the false word is simply not there.
        assert_eq!(
            out.matches("scoop").count(),
            1,
            "exactly one line in this table is scoop's: {out}"
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
