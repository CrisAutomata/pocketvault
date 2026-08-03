use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use pocketvault_core::{Vault, VaultKey};

use crate::commands::{self, Outcome};
use crate::jobs::{self, CancelSlot};
use crate::listing::{self, ListedItem};
use crate::menu::{self, MenuOutcome};
use crate::prompt::{self, Prompter};

pub struct CliSession {
    pub vault: Vault,
    pub key: VaultKey,
    pub current_folder_id: Option<String>,
    /// The folders/files shown by the most recent `ls` (or auto-listing after
    /// `cd`) — lets commands address an item by the index number just shown,
    /// not just by typing its full name again.
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

pub fn run(base_dir: PathBuf) {
    println!("PocketVault — interactive vault session");
    println!("Vault directory: {}", base_dir.display());

    let cancel_slot = jobs::install_ctrlc_handler();
    // The menu UI (dialoguer/console raw-mode key reads) needs a real TTY —
    // without one (piped input, CI, tests) we stay in command-line mode for
    // the whole session, same graceful degradation as `Prompter`/`rpassword`.
    let menu_available = std::io::stdin().is_terminal();

    loop {
        let Some(mut session) = authenticate(&base_dir) else {
            println!("Goodbye.");
            return;
        };

        let mut in_menu = menu_available;
        let outcome = loop {
            if in_menu {
                match menu::run(&mut session, &cancel_slot) {
                    MenuOutcome::UseCommandLine => in_menu = false,
                    MenuOutcome::Lock => break Outcome::Lock,
                    MenuOutcome::Exit => break Outcome::Exit,
                }
            } else {
                match repl(&mut session, &cancel_slot, menu_available) {
                    Outcome::Menu => in_menu = true,
                    Outcome::Lock => break Outcome::Lock,
                    Outcome::Exit => break Outcome::Exit,
                    Outcome::Continue => unreachable!("repl() only returns on menu/lock/exit"),
                }
            }
        };

        match outcome {
            Outcome::Lock => {
                println!("Vault locked.");
                continue;
            }
            Outcome::Exit => {
                println!("Goodbye.");
                return;
            }
            _ => unreachable!("loop above only breaks with Lock or Exit"),
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

fn repl(session: &mut CliSession, cancel_slot: &CancelSlot, menu_available: bool) -> Outcome {
    println!("Type `help` for commands, `lock` to lock the vault, `exit` to quit.");
    if menu_available {
        println!("Type `menu` to switch back to the menu-driven UI.");
    }
    session.last_listing = listing::build_listing(&session.vault, &session.key, session.current_folder_id.as_deref());
    listing::print_listing(session);

    let mut prompter = Prompter::new();
    loop {
        let prompt_text = format!("vault:{}> ", listing::path_string(session));
        let Some(line) = prompter.read_line(&prompt_text) else {
            println!();
            return Outcome::Exit;
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match commands::dispatch(session, cancel_slot, line, menu_available) {
            Outcome::Continue => {}
            outcome => return outcome,
        }
    }
}
