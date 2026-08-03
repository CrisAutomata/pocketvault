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
    println!(
        "{}",
        format!("vault:{}", listing::path_string(session))
            .bold()
            .cyan()
    );
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
    Input::<String>::with_theme(&menu_theme())
        .with_prompt(prompt)
        .interact_text()
        .ok()
}

fn encrypt_file_flow(session: &mut CliSession, cancel_slot: &CancelSlot) {
    let Some(raw) = input("Path to the file to encrypt") else {
        return;
    };
    let path = PathBuf::from(raw);
    if !path.is_file() {
        log::fail("That's not an existing file.");
        return;
    }
    actions::encrypt_files(session, cancel_slot, vec![path]);
}

fn encrypt_folder_flow(session: &mut CliSession, cancel_slot: &CancelSlot) {
    let Some(raw) = input("Path to the folder to encrypt") else {
        return;
    };
    let path = PathBuf::from(raw);
    if !path.is_dir() {
        log::fail("That's not an existing folder.");
        return;
    }
    actions::encrypt_folder(session, cancel_slot, path);
}

enum BrowseRow {
    Item(usize),
    Up,
    EncryptFile,
    EncryptFolder,
    NewFolder,
    Back,
}

/// Encrypt File/Folder are offered here too, not just from the main menu —
/// otherwise encrypting into a subfolder you've navigated into meant backing
/// all the way out to the main menu first (it would still land in the right
/// folder, since `current_folder_id` isn't reset by that, but nothing here
/// hinted that was possible).
fn browse(session: &mut CliSession, cancel_slot: &CancelSlot) {
    loop {
        session.last_listing = listing::build_listing(
            &session.vault,
            &session.key,
            session.current_folder_id.as_deref(),
        );

        let mut labels: Vec<String> = Vec::new();
        let mut rows: Vec<BrowseRow> = Vec::new();

        for (i, item) in session.last_listing.iter().enumerate() {
            labels.push(listing::item_label(&session.vault, &session.key, item));
            rows.push(BrowseRow::Item(i));
        }

        if session.current_folder_id.is_some() {
            labels.push(".. Up one level".to_string());
            rows.push(BrowseRow::Up);
        }
        labels.push("+ Encrypt File".to_string());
        rows.push(BrowseRow::EncryptFile);
        labels.push("+ Encrypt Folder".to_string());
        rows.push(BrowseRow::EncryptFolder);
        labels.push("+ New Folder".to_string());
        rows.push(BrowseRow::NewFolder);
        labels.push("< Back to Main Menu".to_string());
        rows.push(BrowseRow::Back);

        print_location(session);
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let Some(idx) = select("", &refs) else {
            continue;
        };

        match &rows[idx] {
            BrowseRow::Item(item_idx) => item_action(session, cancel_slot, *item_idx),
            BrowseRow::Up => go_up(session),
            BrowseRow::EncryptFile => encrypt_file_flow(session, cancel_slot),
            BrowseRow::EncryptFolder => encrypt_folder_flow(session, cancel_slot),
            BrowseRow::NewFolder => {
                if let Some(name) = input("New folder name") {
                    actions::create_folder(session, &name);
                }
            }
            BrowseRow::Back => return,
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
    let Some(idx) = select(&item.name, OPTIONS) else {
        return;
    };
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
    let Some(idx) = select(&item.name, OPTIONS) else {
        return;
    };
    match idx {
        0 => preview_flow(session, item),
        1 => export_flow(session, cancel_slot, item),
        2 => delete_flow(session, item),
        _ => {}
    }
}

fn export_flow(session: &mut CliSession, cancel_slot: &CancelSlot, item: &ListedItem) {
    let Some(dest) = input("Export to which folder on disk?") else {
        return;
    };
    actions::export_item(session, cancel_slot, item, PathBuf::from(dest));
}

fn delete_flow(session: &mut CliSession, item: &ListedItem) {
    let message = if item.kind == ItemKind::Folder {
        format!(
            "Delete folder '{}' and everything inside it? This cannot be undone.",
            item.name
        )
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

fn preview_flow(session: &CliSession, item: &ListedItem) {
    actions::preview_item(session, item);
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
        let Some(idx) = select("", OPTIONS) else {
            return SettingsOutcome::Back;
        };
        match idx {
            0 => change_password_flow(session),
            1 => return SettingsOutcome::Lock,
            2 => return SettingsOutcome::Back,
            _ => unreachable!(),
        }
    }
}

fn change_password_flow(session: &mut CliSession) {
    let Ok(old_password) = Password::with_theme(&menu_theme())
        .with_prompt("Current password")
        .interact()
    else {
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
