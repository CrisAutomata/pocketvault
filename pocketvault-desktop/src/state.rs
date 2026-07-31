use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use iced::window;
use pocketvault_core::{JobControl, PvMetadata, Vault, VaultKey};

/// Encrypt/export/delete jobs at or above this size run as a background job
/// (status banner, cancel-where-applicable) instead of blocking the UI thread
/// synchronously.
pub const BIG_JOB_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;

/// Max total encrypt jobs allowed between the one running and the ones
/// waiting — past this, starting another shows a "queue full" message
/// instead of silently accepting it.
pub const MAX_QUEUE_TOTAL: usize = 10;

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
        .filter_map(|f| vault.read_metadata(&f.id, key).ok().map(|m| (f.id.clone(), m)))
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
        .map(|f| session.meta_cache.get(&f.id).map(|m| m.original_size).unwrap_or(0))
        .sum();
    for sub in session.vault.folders(Some(folder_id)) {
        total += vault_folder_total_size(session, &sub.id);
    }
    total
}

/// What an encrypt job needs in order to actually start once it's its turn —
/// stashed in `QueuedEncryptJob` for jobs still waiting behind another.
pub enum EncryptJobInput {
    Files { paths: Vec<PathBuf>, dest_folder_id: Option<String> },
    Folder { path: PathBuf, dest_folder_id: Option<String> },
}

/// An encrypt job that's been accepted but hasn't started yet because one is
/// already running (only one runs at a time — see `PocketVault::running_encrypt`).
pub struct QueuedEncryptJob {
    pub id: u64,
    pub label: String,
    pub input: EncryptJobInput,
    pub total_bytes: u64,
}

/// The single currently-executing encrypt job. `control` is shared with the
/// background thread doing the real work: `view()` reads `control.bytes_done`
/// directly on every redraw to compute live progress/ETA, no message-passing
/// needed for that part.
pub struct RunningEncryptJob {
    pub id: u64,
    pub label: String,
    pub control: Arc<JobControl>,
    pub total_bytes: u64,
    pub started_at: Instant,
    pub cancelling: bool,
}

/// The single currently-executing delete job (big folder/file delete only —
/// small ones stay instant/synchronous). No cancellation: an unlinked `.pv`
/// file can't be undone, so there's nothing for Cancel to mean here.
pub struct RunningDeleteJob {
    pub label: String,
    /// `Some(folder_id)` when deleting a folder, so the completion handler
    /// can back out of the current view if it was the one being browsed.
    pub target_folder_id: Option<String>,
}

/// The single currently-executing export job — read-only, so it never
/// contends with encrypt/delete and is always allowed to run.
pub struct RunningExportJob {
    pub label: String,
    pub control: Arc<JobControl>,
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
    ConfirmCancelJob(CancelTarget),
}

/// Which running job a pending "confirm cancel" modal refers to — encrypt and
/// export can be active at the same time (export is independent of the
/// encrypt pool), so a bare "cancel the job" isn't enough to disambiguate.
#[derive(Debug, Clone, Copy)]
pub enum CancelTarget {
    Encrypt,
    Export,
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
    pub running_encrypt: Option<RunningEncryptJob>,
    pub encrypt_queue: VecDeque<QueuedEncryptJob>,
    pub active_delete_job: Option<RunningDeleteJob>,
    pub active_export_job: Option<RunningExportJob>,
    pub next_job_id: u64,
}

impl PocketVault {
    /// True while an encrypt job is running or waiting its turn — used to
    /// gate the exclusive slot (New Folder/Rename/Delete/Change Password),
    /// which must never run concurrently with an in-flight encrypt.
    pub fn encrypt_pool_busy(&self) -> bool {
        self.running_encrypt.is_some() || !self.encrypt_queue.is_empty()
    }

    /// True when the exclusive slot (New Folder/Rename/Delete/Change
    /// Password) must stay disabled.
    pub fn exclusive_slot_busy(&self) -> bool {
        self.encrypt_pool_busy() || self.active_delete_job.is_some()
    }
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

/// Live "~Ns remaining" (or "~Nm Ns") label for a running job, computed from
/// bytes processed so far vs. elapsed time — "Estimating…" until at least one
/// chunk has landed, since a rate can't be known yet.
pub fn eta_label(control: &JobControl, total_bytes: u64, started_at: Instant) -> String {
    let done = control.bytes_done.load(std::sync::atomic::Ordering::Relaxed);
    if done == 0 || total_bytes == 0 {
        return "Estimating…".to_string();
    }
    let elapsed = started_at.elapsed().as_secs_f64();
    let rate = done as f64 / elapsed.max(0.001);
    let remaining = total_bytes.saturating_sub(done) as f64;
    let eta_secs = (remaining / rate.max(1.0)).round() as u64;

    if eta_secs < 60 {
        format!("~{eta_secs}s remaining")
    } else {
        format!("~{}m {}s remaining", eta_secs / 60, eta_secs % 60)
    }
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
