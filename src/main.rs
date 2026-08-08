use anyhow::Result;
use clap::{Parser, Subcommand};
use dotpkg::backend::{scoop::Scoop, Backend};
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
        } => {
            // Phase 2b-1 ships only the read side of apply: everything up to,
            // but not including, the first destructive act. Bailing here,
            // loudly, before touching any file, is the point -- apply must
            // never silently do nothing, and never quietly behave as
            // --prepare.
            anyhow::ensure!(
                prepare,
                "apply has no executor yet -- it lands in Phase 2b-2. \
                 Pass --prepare to stage and fetch everything without changing anything."
            );

            let declared = dotpkg::config::load(&config)?;
            let locked = dotpkg::lock::load_or_empty(&lock)?;
            let state = State::load_or_empty(&State::default_path())?;

            if !allow_empty_config {
                dotpkg::apply::mass_prune_guard(&declared, &state)?;
            }
            dotpkg::apply::lock_coherence_guard(&declared, &locked)?;

            let scoop = Scoop::discover();
            let scan = scoop.scan()?;
            for w in &scan.warnings {
                eprintln!("warning: scoop: {w}");
            }
            let procs = dotpkg::sys::running_processes();
            let running = scoop.running_set(&procs);

            let plan = dotpkg::plan::plan(&declared, &locked, &scan.installed, &state, &running);
            let staging_root = dotpkg::apply::default_staging_root();
            let preparation = dotpkg::apply::prepare(&plan, &locked, &scoop, &staging_root);

            print!("{}", dotpkg::render::render_preparation(&preparation));
            // `process::exit` below skips the normal `main` teardown that
            // would otherwise flush a block-buffered stdout (piped output,
            // as in the Step 7 smoke test), so the render above would
            // otherwise risk being lost right when a non-zero exit needs it
            // most.
            std::io::stdout().flush().ok();

            if !preparation.is_ok() {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
