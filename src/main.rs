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
        /// Required, in addition to consent, for a run that removes
        /// anything. The cheapest answer to one surviving declared package
        /// disarming the mass-prune guard while everything else it owned
        /// gets pruned.
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status { config, lock } => {
            let declared = dotpkg::config::load(&config)?;
            let locked = dotpkg::lock::load_or_empty(&lock)?;
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
            anyhow::ensure!(
                state_path.is_absolute(),
                "the state file resolves to {}, which is relative to the current \
                 directory. Pass --state with an absolute path.",
                state_path.display()
            );

            // Both guards run before anything reads the machine: an empty
            // pkg.toml or an incoherent lock are file-corruption cases, and
            // no amount of scanning or staging makes either one more
            // trustworthy.
            let declared_only = dotpkg::config::load(&config)?;
            let state_only = State::load_or_empty(&state_path)?;
            if !allow_empty_config {
                dotpkg::apply::mass_prune_guard(&declared_only, &state_only)?;
            }
            let locked_only = dotpkg::lock::load_or_empty(&lock)?;
            dotpkg::apply::lock_coherence_guard(&locked_only)?;

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
            let preparation =
                dotpkg::apply::prepare(&plan, &d.locked, &d.scoop, &staging_root, &d.declared);
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
                if !preparation.is_ok() {
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

            let removals = steps
                .iter()
                .filter(|s| matches!(s, Step::Remove { .. }))
                .count();
            if removals > 0 && yes && !allow_prune {
                anyhow::bail!(
                    "this run would remove {removals} package(s) and --yes was passed. \
                     Removals need --allow-prune as well."
                );
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
            if code != 0 {
                std::process::exit(code);
            }
        }
    }
    Ok(())
}
