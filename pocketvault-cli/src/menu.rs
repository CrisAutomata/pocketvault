//! The default, menu-driven front end: arrow keys + Enter over `[x]`/`[ ]`
//! style lists instead of typed commands. `commands.rs` (shell mode) remains
//! available as an escape hatch for anyone who'd rather type — both call
//! into the same `actions` module, so results and their ✔/⚠/✖ reporting are
//! identical either way.

use std::path::PathBuf;

use dialoguer::{Confirm, Input, Password, Select};
use owo_colors::OwoColorize;

use crate::actions;
use crate::jobs::CancelSlot;
use crate::listing::{self, ItemKind, ListedItem};
use crate::log;
use crate::session::CliSession;
use crate::theme::menu_theme;

pub enum MenuOutcome {
    UseCommandLine,
    Lock,
    Exit,
}

const MAIN_ITEMS: &[&str] = &[
    "Encrypt File",
    "Encrypt Folder",
    "Browse Vault",
    "Settings",
    "Use command line",
    "Exit",
];

pub fn run(session: &mut CliSession, cancel_slot: &CancelSlot) -> MenuOutcome {
    loop {
        print_location(session);

        let idx = match select("", MAIN_ITEMS) {
            Some(idx) => idx,
            None => continue,
        };

        match idx {
            0 => encrypt_file_flow(session, cancel_slot),
            1 => encrypt_folder_flow(session, cancel_slot),
            2 => browse(session, cancel_slot),
            3 => {
                if let SettingsOutcome::Lock = settings(session) {
                    return MenuOutcome::Lock;
                }
            }
            4 => return MenuOutcome::UseCommandLine,
            5 => return MenuOutcome::Exit,
            _ => unreachable!(),
        }
    }
}

fn print_location(session: &CliSession) {
    println!();
    println!("{}", format!("vault:{}", listing::path_string(session)).bold().cyan());
}

/// A `Select` over `items`, rendered with the `[x]`/`[ ]` theme. `None` on
/// Esc/Ctrl-C/any input error — callers treat that as "redraw, don't act."
fn select(prompt: &str, items: &[&str]) -> Option<usize> {
    Select::with_theme(&menu_theme())
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .interact_opt()
        .ok()
        .flatten()
}

fn input(prompt: &str) -> Option<String> {
    Input::<String>::with_theme(&menu_theme()).with_prompt(prompt).interact_text().ok()
}

fn encrypt_file_flow(session: &mut CliSession, cancel_slot: &CancelSlot) {
    let Some(raw) = input("Path to the file to encrypt") else { return };
    let path = PathBuf::from(raw);
    if !path.is_file() {
        log::fail("That's not an existing file.");
        return;
    }
    actions::encrypt_files(session, cancel_slot, vec![path]);
}

fn encrypt_folder_flow(session: &mut CliSession, cancel_slot: &CancelSlot) {
    let Some(raw) = input("Path to the folder to encrypt") else { return };
    let path = PathBuf::from(raw);
    if !path.is_dir() {
        log::fail("That's not an existing folder.");
        return;
    }
    actions::encrypt_folder(session, cancel_slot, path);
}

fn browse(session: &mut CliSession, cancel_slot: &CancelSlot) {
    loop {
        session.last_listing =
            listing::build_listing(&session.vault, &session.key, session.current_folder_id.as_deref());
        let item_count = session.last_listing.len();

        let mut labels: Vec<String> = session
            .last_listing
            .iter()
            .map(|item| match item.kind {
                ItemKind::Folder => {
                    let count = session.vault.meta.folder_file_count(&item.id);
                    format!("\u{1F4C1} {} ({} item{})", item.name, count, if count == 1 { "" } else { "s" })
                }
                ItemKind::File => {
                    let size = session
                        .vault
                        .read_metadata(&item.id, &session.key)
                        .map(|m| listing::format_size(m.original_size))
                        .unwrap_or_else(|_| "—".into());
                    format!("\u{1F4C4} {} ({size})", item.name)
                }
            })
            .collect();

        let has_parent = session.current_folder_id.is_some();
        if has_parent {
            labels.push(".. Up one level".to_string());
        }
        labels.push("+ New Folder".to_string());
        labels.push("< Back to Main Menu".to_string());

        print_location(session);
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let Some(idx) = select("", &refs) else { continue };

        if idx < item_count {
            item_action(session, cancel_slot, idx);
            continue;
        }

        let control = idx - item_count;
        let is_up = has_parent && control == 0;
        let is_new_folder = control == has_parent as usize;
        if is_up {
            go_up(session);
        } else if is_new_folder {
            if let Some(name) = input("New folder name") {
                actions::create_folder(session, &name);
            }
        } else {
            return; // "Back to Main Menu"
        }
    }
}

fn go_up(session: &mut CliSession) {
    if let Some(id) = session.current_folder_id.clone() {
        let chain = session.vault.meta.folder_path(&id);
        session.current_folder_id = if chain.len() >= 2 {
            Some(chain[chain.len() - 2].id.clone())
        } else {
            None
        };
    }
}

fn item_action(session: &mut CliSession, cancel_slot: &CancelSlot, idx: usize) {
    let item = session.last_listing[idx].clone();
    match item.kind {
        ItemKind::Folder => folder_action(session, cancel_slot, &item),
        ItemKind::File => file_action(session, cancel_slot, &item),
    }
}

fn folder_action(session: &mut CliSession, cancel_slot: &CancelSlot, item: &ListedItem) {
    const OPTIONS: &[&str] = &["Open", "Export", "Rename", "Delete", "Cancel"];
    let Some(idx) = select(&item.name, OPTIONS) else { return };
    match idx {
        0 => session.current_folder_id = Some(item.id.clone()),
        1 => export_flow(session, cancel_slot, item),
        2 => {
            if let Some(new_name) = input("New name") {
                actions::rename_folder(session, &item.id, &new_name);
            }
        }
        3 => delete_flow(session, item),
        _ => {}
    }
}

fn file_action(session: &mut CliSession, cancel_slot: &CancelSlot, item: &ListedItem) {
    const OPTIONS: &[&str] = &["Preview", "Export", "Delete", "Cancel"];
    let Some(idx) = select(&item.name, OPTIONS) else { return };
    match idx {
        0 => preview_flow(session, item),
        1 => export_flow(session, cancel_slot, item),
        2 => delete_flow(session, item),
        _ => {}
    }
}

fn export_flow(session: &mut CliSession, cancel_slot: &CancelSlot, item: &ListedItem) {
    let Some(dest) = input("Export to which folder on disk?") else { return };
    actions::export_item(session, cancel_slot, item, PathBuf::from(dest));
}

fn delete_flow(session: &mut CliSession, item: &ListedItem) {
    let message = if item.kind == ItemKind::Folder {
        format!("Delete folder '{}' and everything inside it? This cannot be undone.", item.name)
    } else {
        format!("Permanently delete '{}'? This cannot be undone.", item.name)
    };
    let confirmed = Confirm::with_theme(&menu_theme())
        .with_prompt(message)
        .default(false)
        .interact()
        .unwrap_or(false);

    if confirmed {
        actions::delete_item(session, item);
    } else {
        log::warn("Cancelled.");
    }
}

fn preview_flow(session: &mut CliSession, item: &ListedItem) {
    match session.vault.read_to_memory(&item.id, &session.key) {
        Ok((meta, data)) => {
            if meta.mime_type == "text/plain" {
                println!("{}", String::from_utf8_lossy(&data));
            } else if meta.mime_type.starts_with("image/") {
                log::warn(format!("'{}' is an image — terminals can't render it. Use Export instead.", item.name));
            } else {
                log::warn(format!("Preview isn't supported for '{}' ({}).", item.name, meta.mime_type));
            }
        }
        Err(e) => log::fail(format!("Error: {e}")),
    }
}

enum SettingsOutcome {
    Back,
    Lock,
}

fn settings(session: &mut CliSession) -> SettingsOutcome {
    const OPTIONS: &[&str] = &["Change Password", "Lock Vault", "Back"];
    loop {
        println!();
        println!("{}", "Settings".bold().cyan());
        let Some(idx) = select("", OPTIONS) else { return SettingsOutcome::Back };
        match idx {
            0 => change_password_flow(session),
            1 => return SettingsOutcome::Lock,
            2 => return SettingsOutcome::Back,
            _ => unreachable!(),
        }
    }
}

fn change_password_flow(session: &mut CliSession) {
    let Ok(old_password) = Password::with_theme(&menu_theme()).with_prompt("Current password").interact() else {
        return;
    };
    let Ok(new_password) = Password::with_theme(&menu_theme())
        .with_prompt("New password (min 8 characters)")
        .with_confirmation("Confirm new password", "Passwords don't match")
        .interact()
    else {
        return;
    };
    if new_password.len() < 8 {
        log::fail("New password must be at least 8 characters.");
        return;
    }
    actions::change_password(session, &old_password, &new_password);
}
