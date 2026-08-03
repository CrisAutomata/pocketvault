//! Building and printing the current folder's contents, and resolving a
//! user-typed name or index (from the last such listing) back to an id.

use owo_colors::OwoColorize;

use pocketvault_core::{Vault, VaultKey};

use crate::session::CliSession;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Folder,
    File,
}

#[derive(Clone)]
pub struct ListedItem {
    pub kind: ItemKind,
    pub id: String,
    pub name: String,
}

/// Folders then files in `folder_id`, decrypting each file's metadata just
/// far enough to read its original name (cheap — `read_metadata` only reads
/// the header + small encrypted JSON blob, never the file body).
pub fn build_listing(vault: &Vault, key: &VaultKey, folder_id: Option<&str>) -> Vec<ListedItem> {
    let mut items: Vec<ListedItem> = vault
        .folders(folder_id)
        .iter()
        .map(|f| ListedItem {
            kind: ItemKind::Folder,
            id: f.id.clone(),
            name: f.name.clone(),
        })
        .collect();

    items.extend(vault.files_in_folder(folder_id).iter().map(|f| {
        let name = vault
            .read_metadata(&f.id, key)
            .map(|m| m.original_name)
            .unwrap_or_else(|_| f.pv_filename.clone());
        ListedItem {
            kind: ItemKind::File,
            id: f.id.clone(),
            name,
        }
    }));

    items
}

/// Resolves a typed token against the most recent listing: either the index
/// number shown by `ls` (1-based), or an exact name match. Ambiguous names
/// (two items in the same folder sharing a display name) are rejected in
/// favor of the index, which is always unambiguous.
pub fn resolve<'a>(items: &'a [ListedItem], token: &str) -> Result<&'a ListedItem, String> {
    if let Ok(n) = token.parse::<usize>() {
        return match n.checked_sub(1).and_then(|i| items.get(i)) {
            Some(item) => Ok(item),
            None => Err(format!(
                "No item #{n} in the current listing — run `ls` first."
            )),
        };
    }

    let matches: Vec<&ListedItem> = items.iter().filter(|i| i.name == token).collect();
    match matches.len() {
        0 => Err(format!("'{token}' not found in the current folder.")),
        1 => Ok(matches[0]),
        _ => Err(format!(
            "'{token}' matches multiple items here — use the index number from `ls` instead."
        )),
    }
}

pub fn path_string(session: &CliSession) -> String {
    match &session.current_folder_id {
        None => "/".to_string(),
        Some(id) => session
            .vault
            .meta
            .folder_path(id)
            .iter()
            .map(|f| format!("/{}", f.name))
            .collect(),
    }
}

pub fn vault_total_size(vault: &Vault, key: &VaultKey) -> u64 {
    vault
        .meta
        .files
        .iter()
        .filter_map(|f| vault.read_metadata(&f.id, key).ok())
        .map(|m| m.original_size)
        .sum()
}

/// Recursive, decrypted-size total for everything under `folder_id` — used
/// to decide whether exporting/deleting a folder counts as a "big" job.
pub fn folder_total_size(vault: &Vault, key: &VaultKey, folder_id: &str) -> u64 {
    let mut total: u64 = vault
        .files_in_folder(Some(folder_id))
        .iter()
        .map(|f| vault.read_metadata(&f.id, key).map(|m| m.original_size).unwrap_or(0))
        .sum();
    for sub in vault.folders(Some(folder_id)) {
        total += folder_total_size(vault, key, &sub.id);
    }
    total
}

pub fn print_listing(session: &CliSession) {
    let items = &session.last_listing;

    if items.is_empty() {
        println!("(empty — use `encrypt <path>` to add files)");
    }

    let folders: Vec<(usize, &ListedItem)> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.kind == ItemKind::Folder)
        .collect();
    let files: Vec<(usize, &ListedItem)> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.kind == ItemKind::File)
        .collect();

    if !folders.is_empty() {
        println!("{}", "FOLDERS".bold());
        for (idx, item) in &folders {
            let count = session.vault.meta.folder_file_count(&item.id);
            println!(
                "  {:>2}  \u{1F4C1} {:<30} {} item{}",
                idx + 1,
                item.name,
                count,
                if count == 1 { "" } else { "s" }
            );
        }
    }

    if !files.is_empty() {
        println!("{}", "FILES".bold());
        for (idx, item) in &files {
            let meta = session.vault.read_metadata(&item.id, &session.key).ok();
            let size = meta
                .as_ref()
                .map(|m| format_size(m.original_size))
                .unwrap_or_else(|| "—".into());
            let modified = meta
                .as_ref()
                .map(|m| format_ts(m.modified_ts))
                .unwrap_or_else(|| "—".into());
            println!(
                "  {:>2}  \u{1F4C4} {:<30} {:>10}  {}",
                idx + 1,
                item.name,
                size,
                modified
            );
        }
    }

    let total = folders.len() + files.len();
    println!();
    println!(
        "{} item{} — vault total: {}",
        total,
        if total == 1 { "" } else { "s" },
        format_size(vault_total_size(&session.vault, &session.key))
    );
}

pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub fn format_ts(ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let diff = now - ts;
    if diff < 60 {
        "Just now".into()
    } else if diff < 3600 {
        format!("{} min ago", diff / 60)
    } else if diff < 86400 {
        format!("{} hr ago", diff / 3600)
    } else if diff < 86400 * 2 {
        "Yesterday".into()
    } else if diff < 86400 * 7 {
        format!("{} days ago", diff / 86400)
    } else if diff < 86400 * 14 {
        "1 week ago".into()
    } else {
        format!("{} weeks ago", diff / (86400 * 7))
    }
}
