use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

use iced::window;
use pocketvault_core::{PvMetadata, Vault, VaultKey};

/// Encrypt/export/delete jobs at or above this size run as a background job
/// (status banner, disabled interaction, cancel-where-applicable) instead of
/// blocking the UI thread synchronously.
pub const BIG_JOB_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;

pub struct Session {
    pub vault: Vault,
    pub key: VaultKey,
    pub meta_cache: HashMap<String, PvMetadata>,
}

pub fn build_meta_cache(vault: &Vault, key: &VaultKey) -> HashMap<String, PvMetadata> {
    vault
        .meta
        .files
        .iter()
        .filter_map(|f| {
            vault
                .read_metadata(&f.id, key)
                .ok()
                .map(|m| (f.id.clone(), m))
        })
        .collect()
}

/// Total bytes of every file under `folder_id` in the vault, recursively —
/// used to decide whether exporting/deleting a vault folder counts as "big."
/// Reads from the already-decrypted `meta_cache` rather than the vault itself,
/// since only the desktop session knows decrypted sizes.
pub fn vault_folder_total_size(session: &Session, folder_id: &str) -> u64 {
    let mut total: u64 = session
        .vault
        .files_in_folder(Some(folder_id))
        .iter()
        .map(|f| {
            session
                .meta_cache
                .get(&f.id)
                .map(|m| m.original_size)
                .unwrap_or(0)
        })
        .sum();
    for sub in session.vault.folders(Some(folder_id)) {
        total += vault_folder_total_size(session, &sub.id);
    }
    total
}

/// What a background vault job is doing — carried in `VaultJob` so the
/// completion handler knows how to interpret the result and update state.
#[derive(Clone)]
pub enum JobKind {
    EncryptFiles,
    EncryptFolder,
    ExportFile,
    ExportFolder,
    DeleteFile,
    /// Carries the folder id so the completion handler can back out of the
    /// current view if the folder being browsed was the one just deleted.
    DeleteFolder(String),
}

/// A running (or just-requested-to-cancel) background vault job. Only one
/// runs at a time — starting another is disabled while this is `Some`.
pub struct VaultJob {
    pub label: String,
    pub kind: JobKind,
    /// `None` for delete jobs — an already-deleted file can't be undone, so
    /// there's nothing for Cancel to mean there.
    pub cancel: Option<Arc<AtomicBool>>,
    pub cancelling: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Unlock,
    Create,
    ChangePassword,
}

pub struct AuthState {
    pub mode: AuthMode,
    pub password: String,
    pub confirm: String,
    pub old_password: String,
    pub error: String,
    pub busy: bool,
}

impl AuthState {
    pub fn new(mode: AuthMode) -> Self {
        Self {
            mode,
            password: String::new(),
            confirm: String::new(),
            old_password: String::new(),
            error: String::new(),
            busy: false,
        }
    }
}

pub enum Screen {
    Auth(AuthState),
    Vault,
}

pub enum Modal {
    NewFolder {
        name: String,
    },
    Rename {
        folder_id: String,
        text: String,
    },
    DeleteConfirm {
        file_id: Option<String>,
        folder_id: Option<String>,
    },
    ConfirmCancelJob,
}

pub struct PreviewData {
    pub file_name: String,
    pub is_image: bool,
    pub image_rgba: Option<(u32, u32, Vec<u8>)>,
    pub text: String,
}

/// Displayed folder row: id, name, and how many files it directly contains.
pub struct FolderItem {
    pub id: String,
    pub name: String,
    pub item_count: usize,
}

/// A row in the sidebar's VS Code Explorer-style folder tree. Only `depth == 0`
/// rows ever get a chevron — depth-1 (and any deeper) folders are reachable only
/// by navigating into them, not by expanding further in the sidebar.
pub struct FolderTreeRow {
    pub id: String,
    pub name: String,
    pub item_count: usize,
    pub depth: u8,
    pub has_children: bool,
}

/// Displayed file row.
pub struct FileItem {
    pub id: String,
    pub display_name: String,
    pub modified_str: String,
    pub size_str: String,
    pub kind: String,
    pub can_preview: bool,
}

pub struct PocketVault {
    pub base_dir: PathBuf,
    pub session: Option<Session>,
    pub screen: Screen,
    pub current_folder_id: Option<String>,
    pub selected_id: String,
    pub expanded_folders: HashSet<String>,
    pub modal: Option<Modal>,
    pub previews: HashMap<window::Id, PreviewData>,
    pub main_window: window::Id,
    pub active_job: Option<VaultJob>,
}

pub fn vault_base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Total bytes across every encrypted file in the vault, regardless of folder.
pub fn vault_total_size(session: &Session) -> u64 {
    session.meta_cache.values().map(|m| m.original_size).sum()
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
