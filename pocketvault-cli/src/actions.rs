//! The actual mutating operations (encrypt/export/delete/mkdir/rename/change
//! password), shared by both front ends — the menu UI and the command-line
//! mode differ only in how they gather arguments (arrow-key prompts vs typed
//! words); once an operation's inputs are resolved, both call the same
//! function here and get the same colored ✔/⚠/✖ result line.

use std::path::PathBuf;

use pocketvault_core::VaultError;

use crate::jobs::{self, CancelSlot, EncryptInput, ExportTarget};
use crate::listing::{self, ItemKind, ListedItem};
use crate::log;
use crate::session::CliSession;

pub fn encrypt_files(session: &mut CliSession, cancel_slot: &CancelSlot, paths: Vec<PathBuf>) {
    let dest_folder_id = session.current_folder_id.clone();
    let total_bytes: u64 = paths.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
    let label = if paths.len() == 1 {
        paths[0]
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into())
    } else {
        format!("{} files", paths.len())
    };

    let vault = session.vault.clone();
    let key = session.key.clone();
    let input = EncryptInput::Files { paths, dest_folder_id };
    let (new_vault, result) = jobs::run_encrypt(vault, key, input, total_bytes, &label, cancel_slot);
    session.vault = new_vault;

    match result {
        Ok(ids) => log::ok(format!(
            "Encrypted {} file{} into the vault.",
            ids.len(),
            if ids.len() == 1 { "" } else { "s" }
        )),
        Err(VaultError::Cancelled) => log::warn("Cancelled — nothing was added."),
        Err(e) => log::fail(format!("Encrypt failed: {e}")),
    }
}

pub fn encrypt_folder(session: &mut CliSession, cancel_slot: &CancelSlot, path: PathBuf) {
    let dest_folder_id = session.current_folder_id.clone();
    let total_bytes = pocketvault_core::dir_total_size(&path).unwrap_or(0);
    let label = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "folder".into());

    let vault = session.vault.clone();
    let key = session.key.clone();
    let input = EncryptInput::Folder { path, dest_folder_id };
    let (new_vault, result) = jobs::run_encrypt(vault, key, input, total_bytes, &label, cancel_slot);
    session.vault = new_vault;

    match result {
        Ok(ids) => log::ok(format!(
            "Encrypted folder '{label}' — {} file{} added.",
            ids.len(),
            if ids.len() == 1 { "" } else { "s" }
        )),
        Err(VaultError::Cancelled) => log::warn("Cancelled — nothing was added."),
        Err(e) => log::fail(format!("Encrypt failed: {e}")),
    }
}

pub fn export_item(session: &mut CliSession, cancel_slot: &CancelSlot, item: &ListedItem, dest_dir: PathBuf) {
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        log::fail(format!("Can't create destination folder: {e}"));
        return;
    }

    let (target, total_bytes) = match item.kind {
        ItemKind::File => {
            let size = session
                .vault
                .read_metadata(&item.id, &session.key)
                .map(|m| m.original_size)
                .unwrap_or(0);
            (ExportTarget::File { file_id: item.id.clone(), dest_dir }, size)
        }
        ItemKind::Folder => {
            let size = listing::folder_total_size(&session.vault, &session.key, &item.id);
            (ExportTarget::Folder { folder_id: item.id.clone(), dest_dir }, size)
        }
    };

    let vault = session.vault.clone();
    let key = session.key.clone();
    match jobs::run_export(vault, key, target, total_bytes, &item.name, cancel_slot) {
        Ok(path) => log::ok(format!("Exported to {}", path.display())),
        Err(VaultError::Cancelled) => log::warn("Cancelled — no partial file left behind."),
        Err(e) => log::fail(format!("Export failed: {e}")),
    }
}

pub fn delete_item(session: &mut CliSession, item: &ListedItem) {
    match item.kind {
        ItemKind::Folder => match session.vault.delete_folder(&item.id) {
            Ok(removed) => {
                log::ok(format!(
                    "Deleted folder '{}' ({} file{} removed).",
                    item.name,
                    removed.len(),
                    if removed.len() == 1 { "" } else { "s" }
                ));
                if session.current_folder_id.as_deref() == Some(item.id.as_str()) {
                    session.current_folder_id = None;
                }
            }
            Err(e) => log::fail(format!("Delete failed: {e}")),
        },
        ItemKind::File => match session.vault.delete_file(&item.id) {
            Ok(()) => log::ok(format!("Deleted '{}'.", item.name)),
            Err(e) => log::fail(format!("Delete failed: {e}")),
        },
    }
}

pub fn create_folder(session: &mut CliSession, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        log::fail("Folder name cannot be empty.");
        return;
    }
    match session.vault.create_folder(name, session.current_folder_id.as_deref()) {
        Ok(_) => log::ok(format!("Created folder '{name}'.")),
        Err(e) => log::fail(format!("Error: {e}")),
    }
}

pub fn rename_folder(session: &mut CliSession, folder_id: &str, new_name: &str) {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        log::fail("New name cannot be empty.");
        return;
    }
    match session.vault.rename_folder(folder_id, new_name) {
        Ok(()) => log::ok(format!("Renamed to '{new_name}'.")),
        Err(e) => log::fail(format!("Error: {e}")),
    }
}

pub fn change_password(session: &mut CliSession, old_password: &str, new_password: &str) {
    match session.vault.meta.change_password(old_password, new_password) {
        Ok(()) => match session.vault.save() {
            Ok(()) => log::ok("Password changed."),
            Err(e) => log::fail(format!("Error saving vault: {e}")),
        },
        Err(e) => log::fail(format!("Error: {e}")),
    }
}
