use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use pocketvault_core::{Vault, VaultKey};

use crate::jobs;
use crate::listing::ListedItem;
use crate::menu::{self, MenuOutcome};
use crate::prompt;
use crate::tui::{self, TuiOutcome};

pub struct CliSession {
    pub vault: Vault,
    pub key: VaultKey,
    pub current_folder_id: Option<String>,
    /// The folders/files shown by the most recent listing — lets commands
    /// address an item by the index number just shown, not just by typing
    /// its full name again.
    pub last_listing: Vec<ListedItem>,
}

impl CliSession {
    fn new(vault: Vault, key: VaultKey) -> Self {
        Self {
            vault,
            key,
            current_folder_id: None,
            last_listing: Vec::new(),
        }
    }
}

enum SessionOutcome {
    Lock,
    Exit,
}

pub fn run(base_dir: PathBuf) {
    // Both front ends (the arrow-key menu and the full-screen command UI)
    // take over the terminal's raw mode — there's no meaningful degraded
    // mode for piped/non-interactive input, unlike the old scrolling REPL.
    if !std::io::stdin().is_terminal() {
        eprintln!("PocketVault needs an interactive terminal to run.");
        std::process::exit(1);
    }

    println!("PocketVault — interactive vault session");
    println!("Vault directory: {}", base_dir.display());

    let cancel_slot = jobs::install_ctrlc_handler();

    loop {
        let Some(mut session) = authenticate(&base_dir) else {
            println!("Goodbye.");
            return;
        };

        let mut in_menu = true;
        let outcome = loop {
            if in_menu {
                match menu::run(&mut session, &cancel_slot) {
                    MenuOutcome::UseCommandLine => in_menu = false,
                    MenuOutcome::Lock => break SessionOutcome::Lock,
                    MenuOutcome::Exit => break SessionOutcome::Exit,
                }
            } else {
                match tui::run(&mut session, &cancel_slot) {
                    TuiOutcome::UseMenu => in_menu = true,
                    TuiOutcome::Lock => break SessionOutcome::Lock,
                    TuiOutcome::Exit => break SessionOutcome::Exit,
                }
            }
        };

        match outcome {
            SessionOutcome::Lock => {
                println!("Vault locked.");
                continue;
            }
            SessionOutcome::Exit => {
                println!("Goodbye.");
                return;
            }
        }
    }
}

/// Unlocks the vault at `base_dir`, creating one first if none exists yet.
/// Loops on recoverable mistakes (wrong password, mismatched confirm); gives
/// up (returns `None`) on EOF/explicit decline, which ends the whole session.
fn authenticate(base_dir: &Path) -> Option<CliSession> {
    if !Vault::exists(base_dir) {
        return create_vault(base_dir);
    }

    loop {
        let prompt_text = format!("Unlock vault at {} — master password: ", base_dir.display());
        let password = prompt::read_password(&prompt_text)?;

        let vault = match Vault::open(base_dir) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Error: {e}");
                return None;
            }
        };
        match vault.meta.unlock(&password) {
            Ok(key) => {
                println!("Vault unlocked.");
                return Some(CliSession::new(vault, key));
            }
            Err(_) => eprintln!("Invalid password."),
        }
    }
}

fn create_vault(base_dir: &Path) -> Option<CliSession> {
    println!("No vault found in {}.", base_dir.display());
    if !prompt::confirm("Create a new vault here?") {
        return None;
    }

    loop {
        let password = prompt::read_password("Choose a master password (min 8 characters): ")?;
        if password.len() < 8 {
            eprintln!("Password must be at least 8 characters.");
            continue;
        }
        let confirm = prompt::read_password("Confirm password: ")?;
        if password != confirm {
            eprintln!("Passwords do not match.");
            continue;
        }

        let vault = match Vault::create(base_dir, &password) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Error: {e}");
                return None;
            }
        };
        return match vault.meta.unlock(&password) {
            Ok(key) => {
                println!("Vault created at {}.", base_dir.display());
                Some(CliSession::new(vault, key))
            }
            Err(e) => {
                eprintln!("Error: {e}");
                None
            }
        };
    }
}
