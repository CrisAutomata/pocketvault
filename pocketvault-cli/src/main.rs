use std::path::PathBuf;

use clap::Parser;

mod actions;
mod commands;
mod jobs;
mod listing;
mod log;
mod menu;
mod prompt;
mod session;
mod theme;

/// PocketVault — interactive vault session, mirroring the desktop app's
/// unlock/create -> browse/encrypt/export -> lock flow as a REPL.
#[derive(Parser)]
#[command(name = "pocketvault", version, about)]
struct Cli {
    /// Directory containing (or to create) the vault. Defaults to the
    /// current directory, same convention as `git init`.
    #[arg(short = 'C', long = "vault-dir", value_name = "DIR")]
    vault_dir: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();
    let base_dir = cli
        .vault_dir
        .unwrap_or_else(|| std::env::current_dir().expect("cannot read current directory"));

    if let Err(e) = std::fs::create_dir_all(&base_dir) {
        eprintln!("Can't use '{}' as the vault directory: {e}", base_dir.display());
        std::process::exit(1);
    }

    session::run(base_dir);
}
