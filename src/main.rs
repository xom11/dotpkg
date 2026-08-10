use anyhow::Result;
use clap::{Parser, Subcommand};
use dotpkg::backend::winget_exec::RealWingetMutator;
use dotpkg::backend::{
    scoop::Scoop,
    winget::{RealWinget, Winget},
    Backend, Scan, ScanOutcome,
};
use dotpkg::execute::{ScoopStep, Step};
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
/// next run (`Preparation::outstanding_skips` -- running, opaque, or
/// reported-only) is outstanding work the user asked for and did not get.
/// Any of those means 0 would tell a scheduled task the machine is fine when
/// it is not.
///
/// `has_reported_only` is Task 14's addition and is deliberately its own
/// parameter rather than folded into `has_running_skips`: `apply::
/// is_outstanding` already floors a `SkipReason::ReportedOnly` the same way
/// it floors `Running`/`Opaque`, so at `apply`'s own call site the two
/// arguments agree by construction today. Kept separate anyway so this
/// function's own tests can pin the winget rule directly, the same way they
/// already pin the running/opaque one, rather than trusting that the two
/// callers' inputs happen to overlap.
///
/// A floor, not an override: a non-zero code passes through untouched.
fn floor_exit_code(
    code: i32,
    preparation_ok: bool,
    has_running_skips: bool,
    has_reported_only: bool,
) -> i32 {
    if code == 0 && (!preparation_ok || has_running_skips || has_reported_only) {
        1
    } else {
        code
    }
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
            // Whether this plan carries any package dotpkg has scanned,
            // planned and decided differs from the lock but cannot act on --
            // outstanding work the user asked for and did not get, same as a
            // running or opaque skip, and floored the same way below.
            let has_reported_only = plan.actions.iter().any(|a| {
                matches!(
                    a,
                    dotpkg::plan::Action::Skip {
                        reason: dotpkg::plan::SkipReason::ReportedOnly(_),
                        ..
                    }
                )
            });

            if clone_missing_buckets {
                for (name, why) in d.scoop.clone_missing_buckets(&d.declared, &d.scoop) {
                    eprintln!("warning: could not add bucket {name}: {why}");
                }
            }

            let staging_root = dotpkg::apply::default_staging_root();
            let preparation = dotpkg::apply::prepare(
                &plan,
                &d.locked,
                &d.scoop,
                &d.scoop,
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
                // next run (running, opaque, or a winget package that
                // differs from the lock -- `SkipReason::ReportedOnly`) does
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

            let (steps, unusable) = dotpkg::apply::plan_to_steps(&preparation);
            let raw_removals = steps.iter().filter(|s| s.is_remove()).count();

            if !preparation.is_ok() && !keep_going {
                eprintln!(
                    "\n{} package(s) could not be prepared, so nothing has been changed. \
                     Fix them, or pass --keep-going to install the {} that are ready \
                     (removals stay held either way).",
                    unusable.len(),
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
            for app in &held {
                eprintln!(
                    "note: {app} was ready to be removed, but is held: this run also has \
                     package(s) that could not be prepared, and a removal only proceeds \
                     when the whole preparation is ok. Fix them and rerun to let it through."
                );
            }

            // A converged machine: nothing to install, nothing to remove,
            // nothing held back, and nothing reported as needing attention
            // either. Asking "0 installed, 0 removed, continue?" here has no
            // meaningful answer, and an unreadable stdin would refuse it
            // anyway -- exit 2, "go look", every single night, about
            // nothing. There is nothing to look at.
            if steps.is_empty() && unusable.is_empty() && held.is_empty() {
                println!("  Nothing to do -- the machine already matches pkg.toml and pkg.lock.");
                std::io::stdout().flush().ok();
                return Ok(());
            }

            let removals = steps.iter().filter(|s| s.is_remove()).count();
            if removals > 0 && yes && !allow_prune {
                refuse(anyhow::anyhow!(
                    "this run would remove {removals} package(s) and --yes was passed. \
                     Removals need --allow-prune as well."
                ));
            }

            let question = format!(
                "\n{} package(s) will be uninstalled and reinstalled, {} installed, \
                 {} removed. Every version change is an uninstall followed by an \
                 install, in both directions. Continue? [y/N] ",
                steps
                    .iter()
                    .filter(|s| matches!(s, Step::Scoop(ScoopStep::Replace { .. })))
                    .count(),
                steps
                    .iter()
                    .filter(|s| matches!(s, Step::Scoop(ScoopStep::Install { .. })))
                    .count(),
                removals,
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
            for app in &held {
                ex.results.push((
                    app.clone(),
                    dotpkg::execute::ItemResult::Held(
                        "removal held: another package in this run could not be prepared".into(),
                    ),
                ));
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
            for (app, why) in outstanding_skips.iter().cloned() {
                ex.results
                    .push((app, dotpkg::execute::ItemResult::Held(why)));
            }

            // Report only what a fresh scan confirms.
            let after = <Scoop as Backend>::scan(&d.scoop)?;
            let present: Vec<_> = after.installed.iter().map(|i| i.name.clone()).collect();
            ex.dropped_ghosts = d.state.reconcile(dotpkg::model::SCOOP, &present);
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
            // running, opaque, or a winget package that differs from the
            // lock) floors the same way, and for the same reason: it is
            // outstanding work the user asked for and did not get. It is
            // pushed into `ex` above as `Held`, which already makes
            // `exit_code` return 1 -- but this checks `preparation` directly,
            // rather than trusting that push alone, for the same reason the
            // line above does not trust `ex` alone: a skipped package is a
            // fact about the plan, not about what `execute` happened to see,
            // and 0 would tell a scheduled task the machine is fine for as
            // long as the editor stays open, for as long as a package's
            // state could not be read, or for as long as a winget package
            // keeps drifting from its pin.
            //
            // `has_reported_only` is computed straight from `plan` (above),
            // not from `outstanding_skips`: `is_outstanding` already floors a
            // `ReportedOnly` skip the same way, so the two agree here by
            // construction, but `floor_exit_code`'s own tests need to be able
            // to pin the winget rule independently of that agreement holding
            // -- see its doc comment.
            let code = floor_exit_code(
                code,
                preparation.is_ok(),
                !outstanding_skips.is_empty(),
                has_reported_only,
            );
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
        assert_eq!(floor_exit_code(0, true, false, false), 0);
    }

    #[test]
    fn outstanding_work_floors_a_zero_to_one() {
        assert_eq!(
            floor_exit_code(0, false, false, false),
            1,
            "a package that failed to prepare"
        );
        assert_eq!(
            floor_exit_code(0, true, true, false),
            1,
            "a package skipped because it is running"
        );
        assert_eq!(floor_exit_code(0, false, true, false), 1, "both");
    }

    #[test]
    fn outstanding_reported_only_work_floors_the_exit_code_to_one() {
        // Same rule already applied to running skips: work the user asked for
        // and did not get must not report success to a scheduled task.
        assert_eq!(floor_exit_code(0, true, false, true), 1);
        assert_eq!(
            floor_exit_code(0, true, false, false),
            0,
            "the positive sibling"
        );
    }

    #[test]
    fn a_nonzero_code_is_never_lowered_or_raised() {
        assert_eq!(floor_exit_code(2, true, false, false), 2);
        assert_eq!(
            floor_exit_code(2, false, true, false),
            2,
            "the floor is a floor, not an override"
        );
        assert_eq!(
            floor_exit_code(2, true, false, true),
            2,
            "a reported-only skip does not override a nonzero code either"
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
            floor_exit_code(0, prep.is_ok(), !prep.outstanding_skips().is_empty(), false),
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
            floor_exit_code(
                0,
                clean.is_ok(),
                !clean.outstanding_skips().is_empty(),
                false
            ),
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
}
