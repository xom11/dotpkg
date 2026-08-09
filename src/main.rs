use anyhow::Result;
use clap::{Parser, Subcommand};
use dotpkg::backend::{scoop::Scoop, Backend};
use dotpkg::execute::Step;
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
            let procs = dotpkg::sys::running_processes();
            let running = scoop.running_set(&procs);

            // Before the plan, not after: a package missing from the plan
            // because dotpkg could not read it is the one thing the plan
            // itself cannot say.
            for w in &scan.warnings {
                eprintln!("warning: scoop: {w}");
            }

            let plan = dotpkg::plan::plan(&declared, &locked, &scan.installed, &state, &running);
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
            for w in &d.scan.warnings {
                eprintln!("warning: scoop: {w}");
            }

            let plan = dotpkg::plan::plan(
                &d.declared,
                &d.locked,
                &d.scan.installed,
                &d.state,
                &d.running,
            );
            print!("{}", dotpkg::render::render(&plan));

            if clone_missing_buckets {
                for (name, why) in d.scoop.clone_missing_buckets(&d.declared) {
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
                // A package skipped because its own process is running does
                // not fail `is_ok()` -- deliberately, see
                // `Preparation::running_skips`'s doc comment -- but it is
                // still outstanding work the user asked for and did not get.
                // The same fact, the same reasoning, and the same "Exit
                // codes" promise apply here as to the floor the full `apply`
                // path below applies after `execute` returns: 2 would be
                // wrong regardless, since `--prepare` genuinely changed
                // nothing, so what is left to distinguish is 0 (fully
                // realised, nothing outstanding) from 1 (something is), and
                // a running skip is the latter.
                if !preparation.is_ok() || !preparation.running_skips().is_empty() {
                    std::process::exit(1);
                }
                return Ok(());
            }

            let (steps, unusable) = dotpkg::apply::plan_to_steps(&preparation);
            let raw_removals = steps
                .iter()
                .filter(|s| matches!(s, Step::Remove { .. }))
                .count();

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

            let removals = steps
                .iter()
                .filter(|s| matches!(s, Step::Remove { .. }))
                .count();
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
                    .filter(|s| matches!(s, Step::Replace { .. }))
                    .count(),
                steps
                    .iter()
                    .filter(|s| matches!(s, Step::Install { .. }))
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
            let mut ex = match dotpkg::execute::execute(
                d.scoop.root(),
                steps,
                &d.scoop,
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

            // A package skipped at prepare time because its own process was
            // running never becomes a `Step`, so `execute` never sees it and
            // the closing table would otherwise say nothing about it at all
            // -- the same blind spot the loop above closes for a held
            // removal, but for a package that never even tried. Pushed in
            // here, not inside `execute` itself, because `execute` only ever
            // sees the steps a preparation actually produced.
            let running_skips = preparation.running_skips();
            for (app, why) in running_skips.iter().cloned() {
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
            // A package SKIPPED at prepare time because its own process was
            // running floors the same way, and for the same reason: it is
            // outstanding work the user asked for and did not get. It is
            // pushed into `ex` above as `Held`, which already makes
            // `exit_code` return 1 -- but this checks `preparation` directly,
            // rather than trusting that push alone, for the same reason the
            // line above does not trust `ex` alone: a skipped package is a
            // fact about the plan, not about what `execute` happened to see,
            // and 0 would tell a scheduled task the machine is fine for as
            // long as the editor stays open.
            let code = if code == 0 && (!preparation.is_ok() || !running_skips.is_empty()) {
                1
            } else {
                code
            };
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
                    if !declared.scoop.packages.contains(n) {
                        refuse(anyhow::anyhow!(
                            "{n} is not declared in {}. `update` re-resolves what pkg.toml \
                             already asks for; add it there first.",
                            config.display()
                        ));
                    }
                }
            }

            let scoop = Scoop::discover();
            let (u, warnings) = dotpkg::update::run(scoop.root(), &declared, &old, &scope, offline);
            for w in &warnings {
                eprintln!("warning: {w}");
            }
            print!("{}", dotpkg::render::render_update(&u));
            std::io::stdout().flush().ok();

            if u.wrote_anything() {
                dotpkg::lock::save(&u.lock, &lock)?;
            }
            if u.failed_count() > 0 {
                std::process::exit(1);
            }
        }
        Command::Adopt {
            config,
            lock,
            state,
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
            let out = dotpkg::adopt::run(scoop.root(), &names, &config, &lock, &state_path)?;
            // Before the outcome, not after, and for the same reason `status`
            // and `apply` print theirs first: a package dotpkg could not read
            // is refused as "not installed", and that line is false on its own.
            for w in &out.warnings {
                eprintln!("warning: scoop: {w}");
            }
            print!("{}", dotpkg::render::render_adopt(&out));
            std::io::stdout().flush().ok();
            if !out.refused.is_empty() {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
