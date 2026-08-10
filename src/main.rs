use anyhow::Result;
use clap::{Parser, Subcommand};
use dotpkg::backend::winget_exec::RealWingetMutator;
use dotpkg::backend::{
    scoop::Scoop,
    winget::{installed_at_user_scope, RealWinget, Winget},
    Backend, Scan, ScanOutcome,
};
use dotpkg::execute::{ScoopStep, Step, WingetStep};
use dotpkg::model::{Installed, Name, WINGET};
use dotpkg::state::State;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "dotpkg",
    version,
    about = "Declarative package management for Windows"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print what `apply` would do. Changes nothing.
    Status {
        #[arg(long, default_value = "pkg.toml")]
        config: PathBuf,
        #[arg(long, default_value = "pkg.lock")]
        lock: PathBuf,
    },
    /// Bring the machine to the state pkg.toml and pkg.lock describe.
    Apply {
        #[arg(long, default_value = "pkg.toml")]
        config: PathBuf,
        #[arg(long, default_value = "pkg.lock")]
        lock: PathBuf,
        /// Stage and fetch everything the plan needs, then stop before
        /// changing anything.
        #[arg(long)]
        prepare: bool,
        /// Proceed even though pkg.toml declares nothing while dotpkg owns
        /// packages. Only pass this if the empty file is deliberate.
        #[arg(long)]
        allow_empty_config: bool,
        /// Skip the confirmation prompt. Answers that one question and
        /// nothing else -- it does not authorise a prune (pass
        /// --allow-prune for that) and does not bypass any other guard.
        #[arg(long)]
        yes: bool,
        /// Required, in addition to `--yes`, for an unattended run that
        /// removes anything. Answering the confirmation prompt directly
        /// still authorises a prune on its own -- this only gates the
        /// `--yes` fast path, which is the cheapest answer to one surviving
        /// declared package disarming the mass-prune guard while
        /// everything else it owned gets pruned.
        #[arg(long)]
        allow_prune: bool,
        /// Install what is ready even though some packages could not be
        /// prepared. Removals stay held regardless -- this flag never opens
        /// that gate.
        #[arg(long)]
        keep_going: bool,
        /// Clone every bucket pkg.toml declares that is not already on
        /// disk, before staging begins.
        #[arg(long)]
        clone_missing_buckets: bool,
        /// Where dotpkg records what it owns. Defaults to the platform
        /// state directory. Must be an absolute path if given.
        #[arg(long)]
        state: Option<PathBuf>,
    },
    /// Re-resolve pkg.toml against the buckets and rewrite pkg.lock. The only
    /// command that asks what is newest, and the only one that fetches.
    Update {
        #[arg(long, default_value = "pkg.toml")]
        config: PathBuf,
        #[arg(long, default_value = "pkg.lock")]
        lock: PathBuf,
        /// Do not fetch. `latest` then means whatever this machine last
        /// pulled, and the output says so.
        #[arg(long)]
        offline: bool,
        /// Resolve only these packages. Nothing else is rewritten and no
        /// entry is dropped.
        packages: Vec<String>,
    },
    /// Bring already-installed packages under management. Writes pkg.lock,
    /// pkg.toml and state.json; installs and removes nothing.
    Adopt {
        #[arg(long, default_value = "pkg.toml")]
        config: PathBuf,
        #[arg(long, default_value = "pkg.lock")]
        lock: PathBuf,
        /// Where dotpkg records what it owns. Must be absolute if given.
        #[arg(long)]
        state: Option<PathBuf>,
        /// Which backend to adopt from. Defaults to "scoop", unchanged from
        /// before winget adoption existed.
        #[arg(long, default_value = "scoop")]
        backend: String,
        /// The packages to adopt. At least one -- there is deliberately no
        /// "adopt everything", which would be one keystroke from letting a
        /// later pkg.toml edit delete the whole machine.
        #[arg(required = true)]
        packages: Vec<String>,
    },
}

/// Print a refusal and exit 2.
///
/// A guard firing, the user saying no, or no answer being available are all
/// the same fact from a caller's point of view: refused, and the machine was
/// not touched. `?` propagation up through `main() -> Result<()>` would print
/// the same text but exit 1 -- indistinguishable from `--prepare` finding a
/// package it could not prepare, which is a different fact a CI script needs
/// to be able to tell apart without parsing stderr. `--prepare`'s own exit 1
/// is deliberately untouched by this: it is not a refusal, and its own test
/// pins it.
fn refuse(err: anyhow::Error) -> ! {
    eprintln!("{err:#}");
    std::process::exit(2);
}

/// The `apply` exit-code floor, lifted out of `main` so it can be observed.
///
/// A package that failed to PREPARE never becomes a `Step`, so `Execution`
/// cannot see it; a package skipped for a reason that could differ on the
/// next run (`Preparation::outstanding_skips` -- running or opaque) is
/// outstanding work the user asked for and did not get. Either of those means
/// 0 would tell a scheduled task the machine is fine when it is not.
///
/// It had a third parameter, `has_reported_only`, from Phase 4 Task 14 until Phase 4b Task 13:
/// a winget package that differed from the lock and that dotpkg could not act
/// on floored the code too. Winget acts now, so such a package is an
/// `Install`/`Upgrade`/`Downgrade`/`Prune` that either succeeds (0) or fails
/// through `Execution`'s own code, and there is nothing left for a separate
/// parameter to say. It was deleted rather than kept as an argument every
/// caller passes `false` for.
///
/// A floor, not an override: a non-zero code passes through untouched.
fn floor_exit_code(code: i32, preparation_ok: bool, has_running_skips: bool) -> i32 {
    if code == 0 && (!preparation_ok || has_running_skips) {
        1
    } else {
        code
    }
}

/// How many `steps` are a version-change performed as uninstall-then-install
/// and how many put a package on the machine, for the confirmation prompt's
/// "N will be uninstalled and reinstalled, M installed" wording.
///
/// Written as an exhaustive match over every `ScoopStep`/`WingetStep` leaf,
/// not `steps.iter().filter(|s| matches!(s, Step::Scoop(ScoopStep::Install {
/// .. }))).count()`: `matches!` against a partial pattern silently returns
/// `false` for a variant it does not name, so the day `plan_to_steps` started
/// producing `Step::Winget(WingetStep::Set { .. })` this prompt would have
/// gone on saying "0 installed" for a run that installs a winget package,
/// with no compiler error to force anyone to notice. Naming every leaf turned
/// that day into a compile error instead -- and that day is Phase 4b Task 13, which
/// answered it below.
///
/// **`WingetStep::Set` counts as an install, in both of its meanings.** A
/// fresh install obviously is one. A winget *version change* is counted as an
/// install rather than a replacement because `replaces` is not "how many
/// versions change", it is the number the prompt's second sentence is about:
/// "Every version change is an uninstall followed by an install, in both
/// directions" -- true of `ScoopStep::Replace`, which is the only way scoop
/// can change a version, and measurably false of `WingetStep::Set`, which is
/// one `install --version` call that opens no window where the package is
/// absent (see `WingetStep`'s own doc comment). Counting it as a replacement
/// would make the prompt warn about a risk this step does not carry; counting
/// it as neither would put a false zero in the one line the user reads before
/// saying yes.
///
/// `WingetStep::Remove` counts as neither, exactly like
/// `ScoopStep::Remove`: removals have their own count at the call site.
fn count_replaces_and_installs(steps: &[Step]) -> (usize, usize) {
    steps
        .iter()
        .fold((0, 0), |(replaces, installs), s| match s {
            Step::Scoop(ScoopStep::Replace { .. }) => (replaces + 1, installs),
            Step::Scoop(ScoopStep::Install { .. }) => (replaces, installs + 1),
            Step::Scoop(ScoopStep::Remove { .. }) => (replaces, installs),
            Step::Winget(WingetStep::Set { .. }) => (replaces, installs + 1),
            Step::Winget(WingetStep::Remove { .. }) => (replaces, installs),
        })
}

/// The warning for a ready outcome that no executor step claimed, or `None`
/// when every one of them was routed.
///
/// Every ready outcome must become exactly one step -- one `ScoopStep` per
/// `ReadyToFetch`, one `Remove` per `ReadyToRemove`, one `WingetStep::Set` per
/// `ReadyToSet` -- with `gate_removals` moving some of them into `held` rather
/// than dropping them. So `routed < ready` can only mean a `plan_to_steps` arm
/// did not claim something `prepare` made ready: `apply::plan_to_steps`'s
/// "routing bug" arm, a package the plan promised and rendered as `ready`, that
/// the machine will never get. That is `is_ok()`-invisible by construction (a
/// ready outcome raises neither failure count), so without this it is printed
/// nowhere at all.
///
/// A warning, not a refusal: no real input can produce it today, and refusing a
/// run over a can't-happen bug would be worse than reporting it.
///
/// Lifted out of `main` for the same reason `floor_exit_code` was -- it is
/// unreachable from `tests/cli.rs` by construction (the whole point is that no
/// real plan produces it), so a function is the only way to pin the sentence at
/// all. `saturating_sub`, not `-`: the `if` above it already makes underflow
/// unreachable, and a subtraction that panics in a debug build if that guard is
/// ever loosened is not what this warning should turn into.
fn unrouted_warning(ready: usize, routed: usize) -> Option<String> {
    if routed >= ready {
        return None;
    }
    Some(format!(
        "warning: {} package(s) were prepared and shown as ready above, but no executor \
         step was built for them, so they will NOT be changed. This is a routing bug in \
         dotpkg, not a problem with your packages.",
        ready.saturating_sub(routed)
    ))
}

/// Prints what each scan could not read, attributed to its own backend, then
/// hands back what `plan()` needs from all three: `installed` and `opaque`
/// concatenate -- `plan()` already filters `installed` by backend, and
/// `Scan::opaque` is backend-agnostic by construction (see its own doc
/// comment) -- and `unscannable` names every backend whose scan failed
/// outright rather than merely finding nothing (`ScanOutcome::Unscannable`).
/// `warnings` deliberately does NOT concatenate into one untagged list; each
/// scan's warnings (or its one `Unscannable` cause) are printed here,
/// separately, so a winget warning is never mislabelled "scoop:" or vice
/// versa.
///
/// Called before the plan is built, for the same reason `status`'s scoop
/// warning always was, and now doubled for a second backend:
/// `docs/phase3-notes.md` records `adopt` discarding `scan.warnings` and
/// calling a package dotpkg simply could not read "not installed" -- every
/// command that scans must print what it could not read, or that mistake
/// happens again, once per backend.
fn print_scan_warnings_and_merge(
    scoop_scan: &Scan,
    winget_scan: &ScanOutcome,
) -> (Vec<Installed>, Vec<Name>, Vec<&'static str>) {
    for w in &scoop_scan.warnings {
        eprintln!("warning: scoop: {w}");
    }
    let mut installed = scoop_scan.installed.clone();
    let mut opaque = scoop_scan.opaque.clone();
    let mut unscannable = Vec::new();
    match winget_scan {
        ScanOutcome::Scanned(scan) => {
            for w in &scan.warnings {
                eprintln!("warning: winget: {w}");
            }
            installed.extend(scan.installed.iter().cloned());
            opaque.extend(scan.opaque.iter().cloned());
        }
        ScanOutcome::Unscannable(why) => {
            eprintln!("warning: winget: {why}");
            unscannable.push(WINGET);
        }
    }
    (installed, opaque, unscannable)
}

/// Drops ghosts -- state entries whose package is no longer there -- for
/// every backend a run just acted on, not only scoop. Extracted out of the
/// `apply` handler as its own function, behaviour unchanged, so this exact
/// sequence (not a reimplementation of it) can be unit tested without
/// spawning `winget.exe`: `tests/cli.rs` cannot drive this, because
/// `Fixture::run` strips every `winget`/`winget.exe` file off the `PATH` it
/// hands the spawned process, and `Winget::scan`'s own `NotFound` arm turns
/// an absent binary into `Ok(Scan::default())`, not an error -- so `present`
/// would always be empty there, indistinguishable from "nothing is
/// implemented" (see `tests/cli.rs`'s own comment at the top of its winget
/// section). Taking an already-computed `ScanOutcome` rather than a
/// `Backend` is what makes both outcomes constructible from a test with no
/// subprocess involved at all.
///
/// A run interrupted between a verified uninstall and the state write
/// leaves an entry with no package behind. It is inert for planning --
/// `plan()` consults `owns` only while iterating installed packages -- but
/// it inflates `owned_count`, which is what `mass_prune_guard` reads, so it
/// is cleaned up here, at the end of the run that made it.
///
/// An `Unscannable` winget deliberately reconciles nothing below: a scan
/// that failed is not evidence that anything is absent, and this is the
/// direction where acting on that mistake deletes an ownership record
/// dotpkg needs in order to prune the package later. Do not "simplify" the
/// `if let ScanOutcome::Scanned(..)` guard below into an unconditional
/// reconcile, even one that means well (e.g. falling back to some other
/// already-computed present list when the scan came back `Unscannable`) --
/// that reintroduces exactly this silent data loss, and
/// `reconcile_ghosts_leaves_winget_untouched_when_its_scan_failed` below is
/// what catches it.
/// Each dropped record is returned with **the backend it was dropped from**.
/// This function has reconciled both backends since Phase 4 Task 14, but its
/// return type did not say which was which, so `render_execution` printed every
/// dropped winget record as `note scoop <id>` -- a false word in user-facing
/// output, older than Phase 4b Task 13 and fixed with it.
fn reconcile_ghosts(
    state: &mut State,
    scoop: &Scoop,
    winget_scan: &ScanOutcome,
) -> Result<Vec<(String, Name)>> {
    let after_scoop = <Scoop as Backend>::scan(scoop)?;
    let mut dropped: Vec<(String, Name)> = state
        .reconcile(dotpkg::model::SCOOP, &present_after(&after_scoop))
        .into_iter()
        .map(|n| (dotpkg::model::SCOOP.to_string(), n))
        .collect();

    if let ScanOutcome::Scanned(after_winget) = winget_scan {
        dropped.extend(
            state
                .reconcile(dotpkg::model::WINGET, &present_after(after_winget))
                .into_iter()
                .map(|n| (dotpkg::model::WINGET.to_string(), n)),
        );
    }
    Ok(dropped)
}

/// Every name a scan says is **on the machine** -- `installed` and `opaque`
/// together, which is the whole reason this is a function rather than two
/// inline `map`s.
///
/// `Scan::opaque` means *"installed, but this backend could not establish its
/// state"* -- its own doc comment's first line, and `plan()` is already built
/// around not reading a name's absence from `installed` as "not installed".
/// `reconcile_ghosts` is the other direction of that same rule, and it is the
/// direction where getting it wrong **destroys data**: a ghost's record is
/// deleted, so an owned package that lands in `opaque` loses the ownership
/// dotpkg needs in order to prune it later, and `render_execution` prints
/// "ownership record dropped: nothing by that name is installed" about a
/// package that is installed.
///
/// **For winget, `opaque` is the ordinary shape, not an edge case.** Measured on
/// a14: 84 of 126 ids sourceless, plus a `"> "`-prefixed pair and three ids
/// whose rows disagreed on version. A side-by-side second version or a rotated
/// source registration is enough. Three costs, all silent: `owned_count(WINGET)`
/// shrinks, which is the number `mass_prune_guard` reads; an
/// `Ownership::Adopted` record is unrecoverable, since a later dotpkg install
/// writes `Installed` rather than `Adopted`; and an adopted-then-undeclared
/// package becomes permanently `Unmanaged`.
///
/// **Applied to scoop too, deliberately and not silently.** The scoop half
/// carried the identical defect at this branch's base -- an app whose
/// `manifest.json` cannot be read or has no `version` goes to `opaque` -- and
/// leaving one backend fixed and the other not would put two rules in one
/// function for no reason a reader could find. It is the same one-line rule for
/// both, so both get it.
fn present_after(scan: &Scan) -> Vec<Name> {
    scan.installed
        .iter()
        .map(|i| i.name.clone())
        .chain(scan.opaque.iter().cloned())
        .collect()
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status { config, lock } => {
            let declared = dotpkg::config::load(&config)?;
            let locked = dotpkg::lock::load_or_empty(&lock)?;

            // A warning, not a refusal. `apply` exits 2 on this lock, and
            // until now `status` printed an actionable plan from it in
            // silence. Refusing here would withhold exactly the information
            // the user needs to fix it, so the plan is still printed --
            // `status` is read-only and its whole product is the truth about
            // this machine.
            if let Err(e) = dotpkg::apply::lock_coherence_guard(&locked) {
                eprintln!("warning: {e:#}");
                eprintln!("warning: `dotpkg apply` will refuse this lock. The plan below is what it describes, not what apply would do.");
            }

            let state = State::load_or_empty(&State::default_path())?;
            let scoop = Scoop::discover();
            let scan = scoop.scan()?;
            let winget = Winget::new(RealWinget);
            // Not `?`: a winget hiccup (winget signals failure through its
            // exit code far more readily than scoop does) must not abort
            // scoop's entirely unrelated half of this run. See
            // `scan_or_warn`'s own doc comment.
            let winget_scan = dotpkg::backend::scan_or_warn(&winget);
            let procs = dotpkg::sys::running_processes();
            let running = scoop.running_set(&procs);

            // Before the plan, not after: a package missing from the plan
            // because dotpkg could not read it is the one thing the plan
            // itself cannot say.
            let (installed, opaque, unscannable) =
                print_scan_warnings_and_merge(&scan, &winget_scan);

            let plan = dotpkg::plan::plan(
                &declared,
                &locked,
                &installed,
                &opaque,
                &state,
                &running,
                &unscannable,
            );
            print!("{}", dotpkg::render::render(&plan));
        }
        Command::Apply {
            config,
            lock,
            prepare,
            allow_empty_config,
            yes,
            allow_prune,
            keep_going,
            clone_missing_buckets,
            state,
        } => {
            let state_path = state.unwrap_or_else(State::default_path);
            if !state_path.is_absolute() {
                refuse(anyhow::anyhow!(
                    "the state file resolves to {}, which is relative to the current \
                     directory. Pass --state with an absolute path.",
                    state_path.display()
                ));
            }

            // Both guards run before anything reads the machine: an empty
            // pkg.toml or an incoherent lock are file-corruption cases, and
            // no amount of scanning or staging makes either one more
            // trustworthy.
            let declared_only = dotpkg::config::load(&config)?;
            let state_only = State::load_or_empty(&state_path)?;
            if !allow_empty_config {
                if let Err(e) = dotpkg::apply::mass_prune_guard(&declared_only, &state_only) {
                    refuse(e);
                }
            }
            let locked_only = dotpkg::lock::load_or_empty(&lock)?;
            if let Err(e) = dotpkg::apply::lock_coherence_guard(&locked_only) {
                refuse(e);
            }

            let mut d = dotpkg::apply::load_everything(&config, &lock, &state_path)?;
            let (installed, opaque, unscannable) =
                print_scan_warnings_and_merge(&d.scan, &d.winget_scan);

            let plan = dotpkg::plan::plan(
                &d.declared,
                &d.locked,
                &installed,
                &opaque,
                &d.state,
                &d.running,
                &unscannable,
            );
            print!("{}", dotpkg::render::render(&plan));

            if clone_missing_buckets {
                for (name, why) in d.scoop.clone_missing_buckets(&d.declared, &d.scoop) {
                    eprintln!("warning: could not add bucket {name}: {why}");
                }
            }

            let staging_root = dotpkg::apply::default_staging_root();
            // `RealWinget`, winget's READ-ONLY seam: `prepare` asks it `winget
            // show` and nothing else. The mutating seam (`RealWingetMutator`)
            // is constructed further down, for `execute` alone.
            let preparation = dotpkg::apply::prepare(
                &plan,
                &d.locked,
                &d.scoop,
                &d.scoop,
                &RealWinget,
                &staging_root,
                &d.declared,
            );
            print!("{}", dotpkg::render::render_preparation(&preparation));
            // `process::exit` below skips the normal `main` teardown that
            // would otherwise flush a block-buffered stdout (piped output,
            // as in the CLI smoke tests), so the render above would
            // otherwise risk being lost right when a non-zero exit needs it
            // most.
            std::io::stdout().flush().ok();

            if prepare {
                // `render_preparation` no longer prints this: the same table
                // is also printed by a full `apply` run, before the
                // mutations start, and there the promise would be false.
                // Here, on the `--prepare` branch, it is true, so it is
                // printed here instead.
                println!("  Nothing has been changed.");
                std::io::stdout().flush().ok();
                // A package skipped for a reason that could differ on the
                // next run (running, or a state that could not be read) does
                // not fail `is_ok()` -- deliberately, see `Preparation::
                // outstanding_skips`'s doc comment -- but it is still
                // outstanding work the user asked for and did not get. The
                // same fact, the same reasoning,
                // and the same "Exit codes" promise apply here as to the
                // floor the full `apply` path below applies after `execute`
                // returns: 2 would be wrong regardless, since `--prepare`
                // genuinely changed nothing, so what is left to distinguish
                // is 0 (fully realised, nothing outstanding) from 1
                // (something is), and an outstanding skip is the latter.
                if !preparation.is_ok() || !preparation.outstanding_skips().is_empty() {
                    std::process::exit(1);
                }
                return Ok(());
            }

            // `installed` is handed over for the winget steps' process guards:
            // a `WingetStep` carries the names a live process might report for
            // its package, and the only source for them is the scan's own
            // `Installed.bins` (see `apply::guard_for`).
            let (steps, unusable) = dotpkg::apply::plan_to_steps(&preparation, &installed);
            let raw_removals = steps.iter().filter(|s| s.is_remove()).count();

            if !preparation.is_ok() && !keep_going {
                eprintln!(
                    "\n{} package(s) could not be prepared, so nothing has been changed. \
                     Fix them, or pass --keep-going to install the {} that are ready \
                     (removals stay held either way).",
                    // NOT `unusable.len()`, which is a wider set and made this
                    // sentence say "3" for one real failure plus two running
                    // apps. See `Preparation::unpreparable_count`.
                    preparation.unpreparable_count(),
                    steps.len() - raw_removals
                );
                std::process::exit(2);
            }

            // Removals are gated on the WHOLE preparation being ok, and no
            // flag opens that gate -- not `--yes`, not `--keep-going`: every
            // newly typed package name is `NotLocked` until `update` exists,
            // so "installs nothing, deletes something" is the one shape
            // reachable today with a not-ok preparation.
            let (steps, held) = dotpkg::apply::gate_removals(steps, preparation.is_ok());
            for (backend, app) in &held {
                eprintln!(
                    "note: {backend} {app} was ready to be removed, but is held: this run \
                     also has package(s) that could not be prepared, and a removal only \
                     proceeds when the whole preparation is ok. Fix them and rerun to let \
                     it through."
                );
            }

            if let Some(w) = unrouted_warning(preparation.ready_count(), steps.len() + held.len()) {
                eprintln!("{w}");
            }

            // The three checks between a prepared step list and `execute` --
            // converged, `--allow-prune`, and the elevation pre-check -- are
            // one `apply::` function rather than three blocks here, so that
            // the ORDER they run in is tested code. Task 15's review found
            // that the pre-check could be moved after the confirmation prompt,
            // or handed a constant, with the whole suite still green: the
            // tests pinned the function, never its place in this arm. What is
            // left to get wrong is this one call and its arguments.
            //
            // The closure is the only place a `None` from the scope query is
            // turned into a decision, and it turns it into "not blocked" --
            // the same rule `sys::elevated()`'s own `None` follows, and it
            // names the package on stderr rather than deciding quietly.
            // "not blocked" rather than "being removed" because another id in
            // the same run may still refuse it, and then this removal never
            // happens at all. `run_winget_step`'s `CANNOT_UNINSTALL_ELEVATED`
            // translation is what catches whatever this lets through: a
            // pre-check plus a translation, not either alone.
            let winget_read = RealWinget;
            let is_user_scope = |id: &Name| match installed_at_user_scope(&winget_read, id) {
                Some(answer) => answer,
                None => {
                    eprintln!(
                        "warning: could not tell whether {id} is installed for user scope, so \
                         it is not being treated as blocked. If winget refuses to uninstall \
                         it, the failure will say so and re-running unelevated is the fix."
                    );
                    false
                }
            };
            match dotpkg::apply::gate_the_run(
                &steps,
                &unusable,
                &held,
                yes,
                allow_prune,
                dotpkg::sys::elevated(),
                &is_user_scope,
            ) {
                dotpkg::apply::RunGate::Proceed => {}
                // The sentence lives here, not in `gate_the_run`: it is what a
                // scheduled run's log is read for, and `tests/cli.rs` pins it.
                dotpkg::apply::RunGate::NothingToDo => {
                    println!(
                        "  Nothing to do -- the machine already matches pkg.toml and pkg.lock."
                    );
                    std::io::stdout().flush().ok();
                    return Ok(());
                }
                dotpkg::apply::RunGate::Refuse(why) => refuse(anyhow::anyhow!("{why}")),
            }

            let removals = steps.iter().filter(|s| s.is_remove()).count();
            let (replacements, installs) = count_replaces_and_installs(&steps);
            let question = format!(
                "\n{replacements} package(s) will be uninstalled and reinstalled, {installs} \
                 installed, {removals} removed. Every version change is an uninstall followed \
                 by an install, in both directions. Continue? [y/N] "
            );
            if !yes {
                let stdin = std::io::stdin();
                let mut lock_in = stdin.lock();
                let mut errout = std::io::stderr();
                if !dotpkg::apply::confirm(&question, &mut lock_in, &mut errout)? {
                    eprintln!("Nothing has been changed.");
                    std::process::exit(2);
                }
            }

            let recovery_path = staging_root.parent().map(|p| p.join("recover.cmd"));
            let opts = dotpkg::execute::ExecOptions {
                recovery_path: recovery_path.clone(),
            };
            let sample = || d.scoop.running_set(&dotpkg::sys::running_processes());
            // Constructed here, not passed in: `main.rs` is the one place a
            // real winget mutation is ever allowed to happen, same as
            // `d.scoop` for the scoop half of this call. Every test uses
            // `FakeWingetMutator` instead.
            let wm = RealWingetMutator;
            let mut ex = match dotpkg::execute::execute(
                d.scoop.root(),
                steps,
                &d.scoop,
                &wm,
                &mut d.state,
                &sample,
                &opts,
            ) {
                Ok(ex) => ex,
                // `execute` returning `Err` means the root is not a scoop
                // install and NOTHING was attempted -- not one package.
                Err(why) => {
                    eprintln!("{why}");
                    std::process::exit(2);
                }
            };

            // The `eprintln!` above satisfies "printed as held" at the time
            // it happens, but the closing table is what a user actually
            // reads at the end of a run -- and until now it disagreed,
            // reporting "0 held" while a prune really was held.
            for (backend, app) in &held {
                ex.results.push(dotpkg::execute::ItemOutcome {
                    backend: backend.clone(),
                    name: app.clone(),
                    result: dotpkg::execute::ItemResult::Held(
                        "removal held: another package in this run could not be prepared".into(),
                    ),
                });
            }

            // A package skipped at prepare time for a reason that could
            // differ on the next run (its own process running, or its state
            // unreadable) never becomes a `Step`, so `execute` never sees it
            // and the closing table would otherwise say nothing about it at
            // all -- the same blind spot the loop above closes for a held
            // removal, but for a package that never even tried. Pushed in
            // here, not inside `execute` itself, because `execute` only ever
            // sees the steps a preparation actually produced.
            let outstanding_skips = preparation.outstanding_skips();
            for (backend, app, why) in outstanding_skips.iter().cloned() {
                ex.results.push(dotpkg::execute::ItemOutcome {
                    backend,
                    name: app,
                    result: dotpkg::execute::ItemResult::Held(why),
                });
            }

            // Report only what a fresh scan confirms -- for every backend
            // that acted, not just scoop. A winget entry whose package was
            // removed outside dotpkg would otherwise sit in state.json
            // forever, inflating the count `mass_prune_guard` reads. Not
            // `?` for the winget scan itself: a winget hiccup here must not
            // stop scoop's ghosts from being cleared, same reasoning as the
            // prepare-time scan above (`scan_or_warn`'s own doc comment).
            let winget = Winget::new(RealWinget);
            let winget_scan = dotpkg::backend::scan_or_warn(&winget);
            ex.dropped_ghosts = reconcile_ghosts(&mut d.state, &d.scoop, &winget_scan)?;
            d.state.save(&state_path)?;

            // A stale recover.cmd from an earlier, failed run is misleading
            // once a later run finishes with nothing outstanding: it would
            // offer to reinstall packages nobody touched tonight. Only ever
            // removed on a zero-failure run, and only ever the exact file
            // this run itself would have written -- a run that fails part
            // way still needs it left in place.
            if ex.failed() == 0 {
                if let Some(p) = &recovery_path {
                    let _ = std::fs::remove_file(p);
                }
            }

            print!("{}", dotpkg::render::render_execution(&ex));
            std::io::stdout().flush().ok();
            let code = ex.exit_code(false);
            // A package that failed to PREPARE never becomes a Step, so `ex`
            // cannot see it. Without this floor, `--keep-going` reports
            // success for a run that left a declared package uninstalled.
            //
            // A package SKIPPED at prepare time for a reason that could
            // differ on the next run (`Preparation::outstanding_skips` --
            // running, or a state that could not be read) floors the same
            // way, and for the same reason: it is outstanding work the user
            // asked for and did not get. It is pushed into `ex` above as
            // `Held`, which already makes `exit_code` return 1 -- but this
            // checks `preparation` directly, rather than trusting that push
            // alone, for the same reason the line above does not trust `ex`
            // alone: a skipped package is a fact about the plan, not about
            // what `execute` happened to see, and 0 would tell a scheduled
            // task the machine is fine for as long as the editor stays open,
            // or for as long as a package's state could not be read.
            let code = floor_exit_code(code, preparation.is_ok(), !outstanding_skips.is_empty());
            if code != 0 {
                std::process::exit(code);
            }
        }
        Command::Update {
            config,
            lock,
            offline,
            packages,
        } => {
            let declared = dotpkg::config::load(&config)?;
            let old = dotpkg::lock::load_or_empty(&lock)?;
            let scope = if packages.is_empty() {
                dotpkg::update::Scope::WholeRun
            } else {
                dotpkg::update::Scope::Named(dotpkg::model::fold_names(
                    packages,
                    "the packages named on the command line",
                )?)
            };
            if let dotpkg::update::Scope::Named(names) = &scope {
                for n in names {
                    // Both backends: a name declared only under [winget] is
                    // not "not declared" just because the check only ever
                    // looked at [scoop] packages.
                    if !declared.scoop.packages.contains(n) && !declared.winget.packages.contains(n)
                    {
                        refuse(anyhow::anyhow!(
                            "{n} is not declared in {}. `update` re-resolves what pkg.toml \
                             already asks for; add it there first.",
                            config.display()
                        ));
                    }
                }
            }

            let scoop = Scoop::discover();
            let winget = Winget::new(RealWinget);
            let (u, warnings) =
                dotpkg::update::run(scoop.root(), &winget, &declared, &old, &scope, offline);
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            print!("{}", dotpkg::render::render_update(&u));
            std::io::stdout().flush().ok();

            if u.wrote_anything() {
                if let Err(e) = dotpkg::lock::save(&u.lock, &lock) {
                    // `render_update` has already printed the diff, which
                    // reads as an accomplished fact. It is not one.
                    eprintln!(
                        "\npkg.lock was NOT written. The diff above is what this run \
                         resolved, not what is on disk -- {} still holds what it held \
                         before.",
                        lock.display()
                    );
                    // `lock::save` refuses a lock `apply` would reject, and
                    // `update` carries an entry it could not re-resolve
                    // forward unchanged -- so one malformed entry anywhere
                    // blocks the whole write and discards every other
                    // package's resolution. `apply.rs`'s "Run `dotpkg update`
                    // to rewrite it" is right for `apply` and `status` and is
                    // nonsense here, so this names the blocking entries and
                    // what actually repairs them instead.
                    let blocking = dotpkg::apply::incoherent_entries(&u.lock);
                    if blocking.is_empty() {
                        eprintln!("{e:#}");
                        std::process::exit(1);
                    }
                    // The reason names its own package already -- every check
                    // behind `incoherent_entries` prefixes it -- so printing
                    // the key as well would say "broken: broken: ...".
                    for (_, why) in &blocking {
                        eprintln!("  {why}");
                    }
                    eprintln!(
                        "\n`update` only rewrites an entry it could re-resolve, so a \
                         malformed one survives every run until it is repaired. Either \
                         delete its `[scoop.<name>]` block from {} and run again -- the \
                         next run writes it fresh -- or run `dotpkg update <name>` for \
                         it, which replaces the entry when that package resolves.",
                        lock.display()
                    );
                    std::process::exit(2);
                }
            }
            if u.failed_count() > 0 {
                std::process::exit(1);
            }
        }
        Command::Adopt {
            config,
            lock,
            state,
            backend,
            packages,
        } => {
            let state_path = state.unwrap_or_else(State::default_path);
            if !state_path.is_absolute() {
                refuse(anyhow::anyhow!(
                    "the state file resolves to {}, which is relative to the current \
                     directory. Pass --state with an absolute path.",
                    state_path.display()
                ));
            }
            // A directory at exactly this path is almost always a truncated
            // --state (the state directory, not state.json inside it), and
            // `State::load_or_empty` would otherwise report it as a generic
            // I/O error surfacing from inside state.rs. Named here, before
            // anything runs, rather than left to whichever package happens
            // to hit it first.
            if state_path.is_dir() {
                refuse(anyhow::anyhow!(
                    "the state file resolves to {}, which is a directory. Pass \
                     --state with the file itself, e.g. .../state.json.",
                    state_path.display()
                ));
            }
            let names =
                dotpkg::model::fold_names(packages, "the packages named on the command line")?;
            let scoop = Scoop::discover();
            let winget = Winget::new(RealWinget);
            let out = dotpkg::adopt::run(
                scoop.root(),
                &winget,
                &backend,
                &names,
                &config,
                &lock,
                &state_path,
            )?;
            // Before the outcome, not after, and for the same reason `status`
            // and `apply` print theirs first: a package dotpkg could not read
            // is refused as "not installed", and that line is false on its own.
            // Attributed to the backend this run actually asked for -- the
            // hardcoded "scoop" here used to be right by construction, before
            // a winget `adopt` existed at all.
            for w in &out.warnings {
                eprintln!("warning: {backend}: {w}");
            }
            print!("{}", dotpkg::render::render_adopt(&backend, &out));
            std::io::stdout().flush().ok();
            // A partial write is not a refusal -- files changed -- so exit 2
            // ("refused, and nothing was touched") would be a lie about it.
            // Exit 1, the same code a refusal uses, because from a script's
            // point of view both mean the same thing: work the user asked for
            // did not happen.
            if !out.refused.is_empty() || out.partial_write.is_some() {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_successful_run_with_nothing_outstanding_keeps_its_own_exit_code() {
        // The case tests/cli.rs cannot construct: no fixture may provide a fake
        // scoop binary, so a fully successful non-empty apply is unreachable there.
        // This is the only case that distinguishes `&&` from `||` and `!` from ``.
        assert_eq!(floor_exit_code(0, true, false), 0);
    }

    #[test]
    fn outstanding_work_floors_a_zero_to_one() {
        assert_eq!(
            floor_exit_code(0, false, false),
            1,
            "a package that failed to prepare"
        );
        assert_eq!(
            floor_exit_code(0, true, true),
            1,
            "a package skipped because it is running"
        );
        assert_eq!(floor_exit_code(0, false, true), 1, "both");
    }

    #[test]
    fn count_replaces_and_installs_counts_a_winget_set_as_an_install_and_neither_remove_as_either()
    {
        // Pins the exhaustive match itself, not just its current output: a
        // mutant swapping the Replace/Install increments, or moving either
        // `Remove` into one of the two counts, or putting `WingetStep::Set`
        // back into `replaces` or into neither, must turn this red.
        //
        // `WingetStep::Set` counted as neither until Phase 4b Task 13 -- deliberately,
        // as a decision written down rather than a silent default, because
        // nothing produced one yet. It does now, and a prompt that said "0
        // installed" for a run that installs a winget package would be the
        // false-number-in-the-line-the-user-reads defect this project has
        // already fixed twice.
        let steps = vec![
            Step::Scoop(ScoopStep::Replace {
                app: Name::new("bat"),
                staged: PathBuf::from("/stage/bat/2/bat.json"),
                arch: None,
            }),
            Step::Scoop(ScoopStep::Install {
                app: Name::new("fzf"),
                staged: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                arch: None,
            }),
            Step::Scoop(ScoopStep::Remove {
                app: Name::new("aichat"),
            }),
            Step::Winget(WingetStep::Set {
                id: Name::new("Brave.Brave"),
                version: "151.1.93.134".to_string(),
                guard: vec![],
            }),
            Step::Winget(WingetStep::Remove {
                id: Name::new("Vivaldi.Vivaldi"),
                version: "8.1.4087.62".to_string(),
                guard: vec![],
            }),
        ];
        assert_eq!(
            count_replaces_and_installs(&steps),
            (1, 2),
            "one scoop replace; one scoop install plus one winget Set are two \
             installs; neither `Remove` moves either count"
        );
    }

    #[test]
    fn an_unrouted_ready_package_is_warned_about_and_a_fully_routed_run_is_silent() {
        // New user-facing output, in a codebase that pins the strings it
        // prints. It cannot be reached from `tests/cli.rs` -- the whole point
        // is that no real plan produces it -- so this is the only place the
        // sentence can be pinned at all.
        let w = unrouted_warning(3, 1).expect("two ready packages became no step");
        assert!(w.starts_with("warning: 2 package(s)"), "got {w}");
        assert!(
            w.contains("routing bug in dotpkg, not a problem with your packages"),
            "the user must be told this is not their fault: {w}"
        );
        assert!(
            w.contains("will NOT be changed"),
            "and what the consequence is: {w}"
        );

        // The silent cases, which are every real run: all routed, and the
        // converged machine with nothing ready at all. Without these, a
        // version that always warned would pass the assertions above.
        assert_eq!(unrouted_warning(2, 2), None, "everything was routed");
        assert_eq!(unrouted_warning(0, 0), None, "a converged machine");
        // `held` removals count as routed, so a run whose only ready action is
        // a held prune must also stay silent -- `routed` is `steps + held`.
        assert_eq!(unrouted_warning(1, 1), None, "one ready, one held");
        // Cannot happen, must not panic: `saturating_sub` rather than `-`.
        assert_eq!(unrouted_warning(1, 5), None, "more routed than ready");
    }

    #[test]
    fn a_nonzero_code_is_never_lowered_or_raised() {
        assert_eq!(floor_exit_code(2, true, false), 2);
        assert_eq!(
            floor_exit_code(2, false, true),
            2,
            "the floor is a floor, not an override"
        );
    }

    #[test]
    fn an_opaque_only_preparation_is_ok_but_still_outstanding_and_floors_the_exit_code() {
        // The consequence review caught: `Opaque` fails neither `is_ok()`
        // nor becomes a `Step`, so nothing about an otherwise "successful"
        // apply run notices it was never verified -- unless
        // `outstanding_skips()` counts it, which is exactly what this run's
        // real call site (below) feeds into `floor_exit_code`'s third
        // argument.
        let prep = dotpkg::apply::Preparation {
            prepared: vec![dotpkg::apply::Prepared {
                action: dotpkg::plan::Action::Skip {
                    backend: dotpkg::model::SCOOP.into(),
                    name: dotpkg::model::Name::new("zellij"),
                    reason: dotpkg::plan::SkipReason::Opaque,
                },
                outcome: dotpkg::apply::Outcome::Skipped {
                    why: "installed, but its state could not be read -- see the warnings above"
                        .to_string(),
                },
            }],
        };
        assert!(
            prep.is_ok(),
            "an opaque skip must not fail the run on its own"
        );
        assert!(
            !prep.outstanding_skips().is_empty(),
            "an opaque skip is outstanding work"
        );
        assert_eq!(
            floor_exit_code(0, prep.is_ok(), !prep.outstanding_skips().is_empty()),
            1,
            "an otherwise-clean run with only an opaque package must not report success"
        );

        // The counterweight: a preparation with nothing outstanding at all
        // must not be floored -- otherwise the assertion above would pass
        // for a `floor_exit_code` that always returns 1 regardless of what
        // `prep` actually contains.
        let clean = dotpkg::apply::Preparation::default();
        assert!(clean.outstanding_skips().is_empty());
        assert_eq!(
            floor_exit_code(0, clean.is_ok(), !clean.outstanding_skips().is_empty()),
            0,
            "a preparation with nothing outstanding must keep its own exit code"
        );
    }

    // -- print_scan_warnings_and_merge --------------------------------------

    fn installed(backend: &str, name: &str) -> Installed {
        Installed {
            backend: backend.to_string(),
            name: Name::new(name),
            version: "1".to_string(),
            arch: None,
            bucket: None,
            bins: Vec::new(),
        }
    }

    #[test]
    fn print_scan_warnings_and_merge_concatenates_installed_and_opaque_from_both_backends() {
        let scoop_scan = Scan {
            installed: vec![installed(dotpkg::model::SCOOP, "fzf")],
            opaque: vec![Name::new("zellij")],
            warnings: vec!["zellij: manifest.json is not usable".to_string()],
        };
        let winget_scan = ScanOutcome::Scanned(Scan {
            installed: vec![installed(dotpkg::model::WINGET, "Git.Git")],
            opaque: vec![Name::new("7zip.7zip")],
            warnings: vec!["7zip.7zip: installed at 2 disagreeing versions".to_string()],
        });
        let (merged_installed, merged_opaque, unscannable) =
            print_scan_warnings_and_merge(&scoop_scan, &winget_scan);
        assert_eq!(
            merged_installed,
            vec![
                installed(dotpkg::model::SCOOP, "fzf"),
                installed(dotpkg::model::WINGET, "Git.Git"),
            ],
            "both backends' installed packages must be present, scoop first"
        );
        assert_eq!(
            merged_opaque,
            vec![Name::new("zellij"), Name::new("7zip.7zip")],
            "both backends' opaque names must be present, scoop first"
        );
        assert!(
            unscannable.is_empty(),
            "a scan that succeeded must not be named unscannable: {unscannable:?}"
        );
    }

    #[test]
    fn print_scan_warnings_and_merge_of_two_empty_scans_merges_to_nothing() {
        // The counterweight: without it, a version that always appended a
        // hardcoded entry would still pass the test above.
        let (merged_installed, merged_opaque, unscannable) =
            print_scan_warnings_and_merge(&Scan::default(), &ScanOutcome::Scanned(Scan::default()));
        assert!(merged_installed.is_empty(), "got {merged_installed:?}");
        assert!(merged_opaque.is_empty(), "got {merged_opaque:?}");
        assert!(unscannable.is_empty(), "got {unscannable:?}");
    }

    #[test]
    fn print_scan_warnings_and_merge_names_winget_as_unscannable_and_leaves_scoop_alone() {
        // The other outcome of Task 6's `ScanOutcome`: a genuine winget scan
        // failure contributes nothing to `installed`/`opaque` (there is
        // nothing to contribute -- the scan never got that far) and must be
        // named in the `unscannable` list `plan()` needs, so it does not read
        // that emptiness as "winget has nothing installed".
        let scoop_scan = Scan {
            installed: vec![installed(dotpkg::model::SCOOP, "fzf")],
            opaque: vec![Name::new("zellij")],
            warnings: Vec::new(),
        };
        let (merged_installed, merged_opaque, unscannable) = print_scan_warnings_and_merge(
            &scoop_scan,
            &ScanOutcome::Unscannable("Access is denied. (os error 5)".to_string()),
        );
        assert_eq!(
            merged_installed,
            vec![installed(dotpkg::model::SCOOP, "fzf")],
            "scoop's own half of the run must be unaffected"
        );
        assert_eq!(merged_opaque, vec![Name::new("zellij")]);
        assert_eq!(unscannable, vec![WINGET], "got {unscannable:?}");
    }

    // -- reconcile_ghosts ----------------------------------------------------
    //
    // Task 8's own test location was corrected here: `tests/cli.rs` cannot
    // drive this. `Fixture::run` strips every `winget`/`winget.exe` file off
    // the `PATH` it hands the spawned `dotpkg` process, and a missing
    // `winget.exe` makes `Winget::scan` return `Ok(Scan::default())`, not an
    // error -- so `present` would always come back empty there, which
    // `State::reconcile`'s own refuse-to-drop-everything guard would then
    // treat exactly like "nothing implemented" (see `tests/cli.rs`'s own
    // comment on this: "these two tests still cannot exercise the 'winget IS
    // installed' branch"). `reconcile_ghosts` takes an already-computed
    // `ScanOutcome`, so both outcomes are constructible here directly, with
    // no subprocess and no `WingetCmd` fake needed.

    /// A scoop root with one real installed app, in the shape `Scoop::scan`
    /// reads it. Mirrors `tests/cli.rs`'s `install_app` helper, duplicated
    /// rather than shared: that helper lives in a separate integration-test
    /// crate this binary's own unit tests cannot reach.
    fn scoop_root_with(tmp: &std::path::Path, app: &str, version: &str) {
        let cur = tmp.join("apps").join(app).join("current");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::write(
            cur.join("manifest.json"),
            format!(r#"{{"version":"{version}"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn reconcile_ghosts_drops_a_ghost_per_backend_while_each_backends_live_entry_survives() {
        // `State::reconcile` itself refuses to drop everything when
        // `present` comes back empty and the map is not (see its own doc
        // comment) -- so a live entry alongside the ghost, for BOTH
        // backends, is not optional: without it this test would pass
        // whether or not the winget half of `reconcile_ghosts` does
        // anything at all. The scoop ghost is asserted too, not just
        // winget's, so a change that reconciles winget and silently stops
        // reconciling scoop cannot slip through unnoticed.
        let tmp = tempfile::tempdir().unwrap();
        scoop_root_with(tmp.path(), "fzf", "1.0.0");
        let scoop = Scoop::new(tmp.path().to_path_buf());

        let mut state = State::default();
        state.set(
            dotpkg::model::SCOOP,
            &Name::new("fzf"),
            dotpkg::state::Ownership::Installed,
        );
        state.set(
            dotpkg::model::SCOOP,
            &Name::new("scoop-ghost"),
            dotpkg::state::Ownership::Installed,
        );
        state.set(
            WINGET,
            &Name::new("Git.Git"),
            dotpkg::state::Ownership::Installed,
        );
        state.set(
            WINGET,
            &Name::new("winget-ghost"),
            dotpkg::state::Ownership::Installed,
        );

        let winget_scan = ScanOutcome::Scanned(Scan {
            installed: vec![installed(WINGET, "Git.Git")],
            opaque: Vec::new(),
            warnings: Vec::new(),
        });

        let dropped = reconcile_ghosts(&mut state, &scoop, &winget_scan).unwrap();

        // Each dropped record names the backend it came from. Without that,
        // `render_execution` printed `note scoop <winget id>` for the winget
        // half -- false since Phase 4 Task 14 taught this function both
        // backends, and only fixed when Phase 4b Task 13 gave every execution line a
        // real backend column.
        assert_eq!(
            dropped,
            vec![
                (dotpkg::model::SCOOP.to_string(), Name::new("scoop-ghost")),
                (WINGET.to_string(), Name::new("winget-ghost")),
            ],
            "both ghosts, each with its own backend, scoop first: {dropped:?}"
        );
        assert!(
            state.owns(dotpkg::model::SCOOP, &Name::new("fzf")),
            "scoop's live entry must survive"
        );
        assert!(
            !state.owns(dotpkg::model::SCOOP, &Name::new("scoop-ghost")),
            "scoop's ghost must be gone"
        );
        assert!(
            state.owns(WINGET, &Name::new("Git.Git")),
            "winget's live entry must survive"
        );
        assert!(
            !state.owns(WINGET, &Name::new("winget-ghost")),
            "winget's ghost must be gone"
        );
    }

    #[test]
    fn an_installed_but_opaque_package_is_present_and_keeps_its_ownership_record() {
        // **`opaque` is not "absent". It is the ORDINARY shape of a winget
        // machine**: measured on a14, 84 of 126 ids came back with no Source at
        // all, plus a `"> "`-prefixed pair and three ids whose two rows disagreed
        // on version. A second side-by-side version, or a rotated source
        // registration, puts an owned package there while it sits installed on
        // the machine -- and `present` built from `installed` alone then reads
        // that as a ghost, deletes the ownership record, and makes
        // `render_execution` print "ownership record dropped: nothing by that
        // name is installed" about a package that IS installed.
        //
        // Three separate costs, which is why this is not cosmetic: it shrinks
        // `owned_count(WINGET)` -- the number `mass_prune_guard` reads -- it
        // destroys an `Ownership::Adopted` record that no reinstall recreates,
        // and it can leave an adopted-then-undeclared package permanently
        // `Unmanaged`, unprunable by the tool that adopted it.
        //
        // The pre-existing test above constructs `opaque: Vec::new()`, which is
        // exactly why nothing caught this. `opaque` is non-empty here in BOTH
        // halves: the scoop half had the identical defect at this branch's base
        // and is fixed in the same commit rather than left as a known one.
        let tmp = tempfile::tempdir().unwrap();
        // A real ghost for each backend, so `State::reconcile`'s own
        // refuse-to-drop-everything guard is not what makes this pass -- and so
        // a mutant that stops reconciling altogether goes red here too.
        scoop_root_with(tmp.path(), "fzf", "1.0.0");
        // `manifest.json` with no `version` key: `Scoop::scan` puts this in
        // `opaque`, not `installed` (src/backend/scoop.rs:299-303).
        let opaque_cur = tmp.path().join("apps").join("busybox").join("current");
        std::fs::create_dir_all(&opaque_cur).unwrap();
        std::fs::write(opaque_cur.join("manifest.json"), "{}").unwrap();
        let scoop = Scoop::new(tmp.path().to_path_buf());

        let mut state = State::default();
        for (backend, name) in [
            (dotpkg::model::SCOOP, "fzf"),
            (dotpkg::model::SCOOP, "busybox"),
            (dotpkg::model::SCOOP, "scoop-ghost"),
            (WINGET, "Git.Git"),
            (WINGET, "Brave.Brave"),
            (WINGET, "winget-ghost"),
        ] {
            state.set(
                backend,
                &Name::new(name),
                dotpkg::state::Ownership::Installed,
            );
        }
        // And one adopted record in `opaque`, because that is the record whose
        // loss is unrecoverable: a reinstall by dotpkg would write
        // `Ownership::Installed` back, never `Adopted`.
        state.set(
            WINGET,
            &Name::new("Obsidian.Obsidian"),
            dotpkg::state::Ownership::Adopted,
        );

        let winget_scan = ScanOutcome::Scanned(Scan {
            installed: vec![installed(WINGET, "Git.Git")],
            opaque: vec![Name::new("Brave.Brave"), Name::new("Obsidian.Obsidian")],
            warnings: Vec::new(),
        });

        let dropped = reconcile_ghosts(&mut state, &scoop, &winget_scan).unwrap();

        assert_eq!(
            dropped,
            vec![
                (dotpkg::model::SCOOP.to_string(), Name::new("scoop-ghost")),
                (WINGET.to_string(), Name::new("winget-ghost")),
            ],
            "only the two real ghosts -- an opaque package is installed: {dropped:?}"
        );
        assert!(
            state.owns(WINGET, &Name::new("Brave.Brave")),
            "an installed-but-opaque winget package must keep its record"
        );
        assert_eq!(
            state.ownership(WINGET, &Name::new("Obsidian.Obsidian")),
            Some(dotpkg::state::Ownership::Adopted),
            "and an Adopted record must survive as Adopted, not merely survive"
        );
        assert!(
            state.owns(dotpkg::model::SCOOP, &Name::new("busybox")),
            "scoop's opaque half is fixed in the same commit, not left standing"
        );
    }

    #[test]
    fn reconcile_ghosts_leaves_winget_untouched_when_its_scan_failed() {
        // The other outcome of Task 6's `ScanOutcome`: a winget scan that
        // FAILED is not evidence that nothing is installed, and treating it
        // as if it were would delete an ownership record dotpkg needs in
        // order to prune the package later. `fzf` is real and present here
        // (unlike the test above's scoop side, which is a red herring on
        // its own) precisely so that a mutant which "simplifies" the guard
        // by reusing scoop's own already-computed `present` list for
        // winget too -- a realistic copy-paste, since both blocks name
        // their local variable `present` -- has something non-empty and
        // wrong to hand `state.reconcile`, and gets caught doing it: see
        // the report for a genuine RED run against exactly that mutant.
        let tmp = tempfile::tempdir().unwrap();
        scoop_root_with(tmp.path(), "fzf", "1.0.0");
        let scoop = Scoop::new(tmp.path().to_path_buf());

        let mut state = State::default();
        state.set(
            dotpkg::model::SCOOP,
            &Name::new("fzf"),
            dotpkg::state::Ownership::Installed,
        );
        state.set(
            WINGET,
            &Name::new("winget-ghost"),
            dotpkg::state::Ownership::Installed,
        );

        let winget_scan = ScanOutcome::Unscannable("winget list exited 1".to_string());

        let dropped = reconcile_ghosts(&mut state, &scoop, &winget_scan).unwrap();

        assert!(
            dropped.is_empty(),
            "an unscannable winget must drop nothing at all: {dropped:?}"
        );
        assert!(
            state.owns(WINGET, &Name::new("winget-ghost")),
            "a failed scan must never be read as \"nothing is installed\""
        );
        assert!(
            state.owns(dotpkg::model::SCOOP, &Name::new("fzf")),
            "scoop's own half must be unaffected by winget's scan failure"
        );
    }
}
