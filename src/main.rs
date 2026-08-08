use anyhow::Result;
use clap::{Parser, Subcommand};
use dotpkg::backend::{scoop::Scoop, Backend};
use dotpkg::state::State;
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status { config, lock } => {
            let declared = dotpkg::config::load(&config)?;
            let locked = dotpkg::lock::load_or_empty(&lock)?;
            let state = State::load_or_empty(&State::default_path())?;
            let installed = Scoop::discover().scan()?;
            let running = dotpkg::sys::running_process_names();

            let plan = dotpkg::plan::plan(&declared, &locked, &installed, &state, &running);
            print!("{}", dotpkg::render::render(&plan));
        }
    }
    Ok(())
}
