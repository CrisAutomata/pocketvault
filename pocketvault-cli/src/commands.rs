use std::path::PathBuf;

use owo_colors::OwoColorize;

use crate::actions;
use crate::jobs::CancelSlot;
use crate::listing::{self, ItemKind};
use crate::log;
use crate::prompt;
use crate::session::CliSession;

pub enum Outcome {
    Continue,
    Menu,
    Lock,
    Exit,
}

pub fn dispatch(session: &mut CliSession, cancel_slot: &CancelSlot, line: &str, menu_available: bool) -> Outcome {
    let args = match shell_words::split(line) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Couldn't parse that line: {e}");
            return Outcome::Continue;
        }
    };
    let Some((cmd, rest)) = args.split_first() else {
        return Outcome::Continue;
    };

    match cmd.as_str() {
        "ls" | "dir" => cmd_ls(session),
        "cd" => cmd_cd(session, rest),
        "pwd" => cmd_pwd(session),
        "mkdir" => cmd_mkdir(session, rest),
        "rename" | "mv" => cmd_rename(session, rest),
        "rm" | "del" => cmd_rm(session, rest),
        "encrypt" | "add" | "import" => cmd_encrypt(session, cancel_slot, rest),
        "export" => cmd_export(session, cancel_slot, rest),
        "cat" | "preview" | "view" => cmd_cat(session, rest),
        "passwd" | "change-password" => cmd_passwd(session),
        "clear" | "cls" => print!("\x1B[2J\x1B[1;1H"),
        "help" | "?" => print_help(menu_available),
        "menu" | "ui" => {
            if menu_available {
                return Outcome::Menu;
            }
            log::fail("No interactive terminal attached — menu mode isn't available here.");
        }
        "lock" => return Outcome::Lock,
        "exit" | "quit" | "q" => return Outcome::Exit,
        other => eprintln!("Unknown command: '{other}' (try `help`)"),
    }

    Outcome::Continue
}

fn refresh_listing(session: &mut CliSession) {
    session.last_listing = listing::build_listing(&session.vault, &session.key, session.current_folder_id.as_deref());
}

fn cmd_ls(session: &mut CliSession) {
    refresh_listing(session);
    listing::print_listing(session);
}

fn cmd_cd(session: &mut CliSession, rest: &[String]) {
    let target = rest.first().map(String::as_str).unwrap_or("/");

    if target == "/" {
        session.current_folder_id = None;
    } else if target == "." {
        // no-op
    } else if target == ".." {
        if let Some(id) = session.current_folder_id.clone() {
            let chain = session.vault.meta.folder_path(&id);
            session.current_folder_id = if chain.len() >= 2 {
                Some(chain[chain.len() - 2].id.clone())
            } else {
                None
            };
        }
    } else {
        refresh_listing(session);
        match listing::resolve(&session.last_listing, target) {
            Ok(item) if item.kind == ItemKind::Folder => {
                session.current_folder_id = Some(item.id.clone());
            }
            Ok(_) => {
                eprintln!("'{target}' is a file, not a folder.");
                return;
            }
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        }
    }

    cmd_ls(session);
}

fn cmd_pwd(session: &CliSession) {
    println!("{}", listing::path_string(session));
}

fn cmd_mkdir(session: &mut CliSession, rest: &[String]) {
    let Some(name) = rest.first() else {
        eprintln!("Usage: mkdir <name>");
        return;
    };
    actions::create_folder(session, name);
}

fn cmd_rename(session: &mut CliSession, rest: &[String]) {
    if rest.len() < 2 {
        eprintln!("Usage: rename <folder> <new-name>");
        return;
    }
    refresh_listing(session);
    let folder_id = match listing::resolve(&session.last_listing, &rest[0]) {
        Ok(item) if item.kind == ItemKind::Folder => item.id.clone(),
        Ok(_) => {
            eprintln!("Only folders can be renamed.");
            return;
        }
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    actions::rename_folder(session, &folder_id, &rest[1]);
}

fn cmd_rm(session: &mut CliSession, rest: &[String]) {
    let force = rest.iter().any(|a| a == "-f" || a == "--force");
    let Some(name) = rest.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("Usage: rm <name> [-f]");
        return;
    };

    refresh_listing(session);
    let item = match listing::resolve(&session.last_listing, name) {
        Ok(item) => item.clone(),
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };

    if !force {
        let message = if item.kind == ItemKind::Folder {
            format!("Delete folder '{}' and everything inside it? This cannot be undone.", item.name)
        } else {
            format!("Permanently delete '{}' from the vault? This cannot be undone.", item.name)
        };
        if !prompt::confirm(&message) {
            log::warn("Cancelled.");
            return;
        }
    }

    actions::delete_item(session, &item);
}

fn cmd_encrypt(session: &mut CliSession, cancel_slot: &CancelSlot, rest: &[String]) {
    if rest.is_empty() {
        eprintln!("Usage: encrypt <path> [<path> ...]   (a single folder path encrypts recursively)");
        return;
    }

    if rest.len() == 1 {
        let path = PathBuf::from(&rest[0]);
        if !path.exists() {
            eprintln!("No such file or folder: {}", path.display());
            return;
        }
        if path.is_dir() {
            actions::encrypt_folder(session, cancel_slot, path);
            return;
        }
        actions::encrypt_files(session, cancel_slot, vec![path]);
        return;
    }

    let mut paths = Vec::with_capacity(rest.len());
    for raw in rest {
        let path = PathBuf::from(raw);
        if !path.exists() {
            eprintln!("No such file: {}", path.display());
            return;
        }
        if path.is_dir() {
            eprintln!("Encrypt one folder at a time — got '{}' alongside other paths.", path.display());
            return;
        }
        paths.push(path);
    }
    actions::encrypt_files(session, cancel_slot, paths);
}

fn cmd_export(session: &mut CliSession, cancel_slot: &CancelSlot, rest: &[String]) {
    if rest.len() < 2 {
        eprintln!("Usage: export <name> <destination-folder>");
        return;
    }

    refresh_listing(session);
    let item = match listing::resolve(&session.last_listing, &rest[0]) {
        Ok(item) => item.clone(),
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };

    actions::export_item(session, cancel_slot, &item, PathBuf::from(&rest[1]));
}

fn cmd_cat(session: &mut CliSession, rest: &[String]) {
    let Some(name) = rest.first() else {
        eprintln!("Usage: cat <name>");
        return;
    };

    refresh_listing(session);
    let item = match listing::resolve(&session.last_listing, name) {
        Ok(item) if item.kind == ItemKind::File => item.clone(),
        Ok(_) => {
            eprintln!("'{name}' is a folder.");
            return;
        }
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };

    match session.vault.read_to_memory(&item.id, &session.key) {
        Ok((meta, data)) => {
            if meta.mime_type == "text/plain" {
                println!("{}", String::from_utf8_lossy(&data));
            } else if meta.mime_type.starts_with("image/") {
                log::warn(format!(
                    "'{}' is an image ({}) — terminals can't render images. Use `export {} <dest>` to save it to disk.",
                    item.name, meta.mime_type, item.name
                ));
            } else {
                log::warn(format!(
                    "Preview isn't supported for '{}' ({}). Use `export {} <dest>` instead.",
                    item.name, meta.mime_type, item.name
                ));
            }
        }
        Err(e) => log::fail(format!("Error: {e}")),
    }
}

fn cmd_passwd(session: &mut CliSession) {
    println!("Changing master password.");
    let Some(old_password) = prompt::read_password("Current password: ") else {
        log::warn("Cancelled.");
        return;
    };
    let Some(new_password) = prompt::read_password("New password (min 8 characters): ") else {
        log::warn("Cancelled.");
        return;
    };
    if new_password.len() < 8 {
        log::fail("New password must be at least 8 characters.");
        return;
    }
    let Some(confirm) = prompt::read_password("Confirm new password: ") else {
        log::warn("Cancelled.");
        return;
    };
    if new_password != confirm {
        log::fail("Passwords do not match.");
        return;
    }

    actions::change_password(session, &old_password, &new_password);
}

fn print_help(menu_available: bool) {
    println!("{}", "Commands:".bold());
    println!("  ls                        List the current folder's contents");
    println!("  cd <name>|..|/            Change folder ('..' = up, '/' = vault root)");
    println!("  pwd                       Show the current folder path");
    println!("  mkdir <name>              Create a folder here");
    println!("  rename <folder> <name>    Rename a folder");
    println!("  rm <name> [-f]            Delete a file or folder (asks to confirm)");
    println!("  encrypt <path> [...]      Encrypt file(s), or one folder, into the vault here");
    println!("  export <name> <dest>      Decrypt a file or folder out to disk");
    println!("  cat <name>                Print a text file's contents");
    println!("  passwd                    Change the master password");
    println!("  lock                      Lock the vault (back to the unlock/create prompt)");
    if menu_available {
        println!("  menu                      Switch to the menu-driven UI");
    }
    println!("  clear                     Clear the screen");
    println!("  help                      Show this help");
    println!("  exit | quit               Leave PocketVault");
    println!();
    println!("Names may be given exactly, or as the index number shown by the last `ls`.");
    println!("Ctrl-C cancels an in-progress encrypt/export; it won't quit the session.");
}
