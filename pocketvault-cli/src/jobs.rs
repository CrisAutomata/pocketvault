//! Runs encrypt/export operations either inline (small jobs) or on a
//! background thread with a live progress bar (big jobs) — the CLI analogue
//! of the desktop's `BIG_JOB_THRESHOLD_BYTES` split, minus the job queue
//! (the REPL is inherently single-command-at-a-time, so there's nothing to
//! queue behind).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};

use pocketvault_core::{JobControl, Result, Vault, VaultKey};

pub const BIG_JOB_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;

/// Holds the `JobControl` of whatever job is currently running, if any, so
/// the process-wide Ctrl-C handler (installed once at startup) has something
/// to cancel. `None` while idle — at the REPL prompt, Ctrl-C never reaches
/// this handler at all (the line editor consumes it as a plain keypress), so
/// leaving this empty during idle time is never observed.
pub type CancelSlot = Arc<Mutex<Option<Arc<JobControl>>>>;

/// Installs a process-wide SIGINT handler that cancels whatever job is
/// currently registered in the returned slot. Ignored (not treated as fatal)
/// if the platform/environment refuses to let us install one.
pub fn install_ctrlc_handler() -> CancelSlot {
    let slot: CancelSlot = Arc::new(Mutex::new(None));
    let slot_for_handler = slot.clone();
    let _ = ctrlc::set_handler(move || {
        if let Ok(guard) = slot_for_handler.lock() {
            if let Some(control) = guard.as_ref() {
                control.cancel.store(true, Ordering::Relaxed);
            }
        }
    });
    slot
}

pub enum EncryptInput {
    Files {
        paths: Vec<PathBuf>,
        dest_folder_id: Option<String>,
    },
    Folder {
        path: PathBuf,
        dest_folder_id: Option<String>,
    },
}

pub enum ExportTarget {
    File {
        file_id: String,
        dest_dir: PathBuf,
    },
    Folder {
        folder_id: String,
        dest_dir: PathBuf,
    },
}

/// Runs `op` on a background thread so there's always something to animate —
/// a byte-progress bar with ETA for jobs at/above the threshold (registered
/// in `cancel_slot` so Ctrl-C can reach them), or a lightweight spinner for
/// everything smaller (not cancellable, same as the desktop's synchronous
/// small-job path — it's just not blocking the terminal while it runs).
fn with_progress<T, F>(
    verb: &str,
    label: &str,
    total_bytes: u64,
    cancel_slot: &CancelSlot,
    op: F,
) -> T
where
    T: Send + 'static,
    F: FnOnce(Arc<JobControl>) -> T + Send + 'static,
{
    let big = total_bytes >= BIG_JOB_THRESHOLD_BYTES;
    let control = Arc::new(JobControl::default());
    if big {
        if let Ok(mut guard) = cancel_slot.lock() {
            *guard = Some(control.clone());
        }
    }

    let pb = if big {
        let pb = ProgressBar::new(total_bytes);
        if let Ok(style) = ProgressStyle::with_template(
            "{spinner:.cyan} {msg} [{bar:30.cyan/blue}] {bytes}/{total_bytes} ({eta} left)",
        ) {
            pb.set_style(style.progress_chars("=> "));
        }
        pb.set_message(format!("{verb} {label} — Ctrl-C to cancel"));
        pb
    } else {
        let pb = ProgressBar::new_spinner();
        if let Ok(style) = ProgressStyle::with_template("{spinner:.cyan} {msg}") {
            pb.set_style(style.tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "));
        }
        pb.set_message(format!("{verb} {label}…"));
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    };

    let control_for_thread = control.clone();
    let handle = std::thread::spawn(move || op(control_for_thread));

    while !handle.is_finished() {
        if big {
            pb.set_position(control.bytes_done.load(Ordering::Relaxed).min(total_bytes));
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    if big {
        pb.set_position(control.bytes_done.load(Ordering::Relaxed).min(total_bytes));
    }
    pb.finish_and_clear();

    if big {
        if let Ok(mut guard) = cancel_slot.lock() {
            *guard = None;
        }
    }

    handle.join().expect("job thread panicked")
}

/// Encrypts `input` into `vault`, returning the (possibly rolled-back, but
/// always persisted) vault alongside the outcome — mirrors the desktop's
/// `EncryptJobFinished` handling, where the vault clone comes back either way.
pub fn run_encrypt(
    vault: Vault,
    key: VaultKey,
    input: EncryptInput,
    total_bytes: u64,
    label: &str,
    cancel_slot: &CancelSlot,
) -> (Vault, Result<Vec<String>>) {
    with_progress(
        "Encrypting",
        label,
        total_bytes,
        cancel_slot,
        move |control| {
            let mut vault = vault;
            let result = match input {
                EncryptInput::Files {
                    paths,
                    dest_folder_id,
                } => vault.encrypt_files(&paths, dest_folder_id.as_deref(), &key, &control),
                EncryptInput::Folder {
                    path,
                    dest_folder_id,
                } => vault.encrypt_folder(&path, dest_folder_id.as_deref(), &key, &control),
            };
            (vault, result)
        },
    )
}

/// Exports `target` — read-only, so there's no vault to hand back.
pub fn run_export(
    vault: Vault,
    key: VaultKey,
    target: ExportTarget,
    total_bytes: u64,
    label: &str,
    cancel_slot: &CancelSlot,
) -> Result<PathBuf> {
    with_progress(
        "Exporting",
        label,
        total_bytes,
        cancel_slot,
        move |control| match target {
            ExportTarget::File { file_id, dest_dir } => {
                vault.export_file(&file_id, &dest_dir, &key, &control)
            }
            ExportTarget::Folder {
                folder_id,
                dest_dir,
            } => vault.export_folder(&folder_id, &dest_dir, &key, &control),
        },
    )
}

// ── Non-blocking job API — used by the full-screen `tui` UI ─────────────
//
// The menu UI above can afford to block: `dialoguer` isn't doing anything
// else while a job runs. The full-screen UI owns a render loop that must
// keep redrawing (and keep watching for a Ctrl-C keypress) while a job is
// in flight, so it needs to *start* a job and poll it from that loop instead
// of blocking on it.

pub enum JobOutcome {
    Encrypt(Vault, Result<Vec<String>>),
    Export(Result<PathBuf>),
}

pub struct RunningJob {
    pub label: String,
    pub verb: &'static str,
    pub control: Arc<JobControl>,
    pub total_bytes: u64,
    pub started_at: Instant,
    pub big: bool,
    receiver: Receiver<JobOutcome>,
}

impl RunningJob {
    /// Non-blocking: `None` while the background thread is still working.
    pub fn poll(&self) -> Option<JobOutcome> {
        self.receiver.try_recv().ok()
    }
}

fn spawn_job<F>(
    verb: &'static str,
    label: String,
    total_bytes: u64,
    cancel_slot: &CancelSlot,
    op: F,
) -> RunningJob
where
    F: FnOnce(Arc<JobControl>) -> JobOutcome + Send + 'static,
{
    let big = total_bytes >= BIG_JOB_THRESHOLD_BYTES;
    let control = Arc::new(JobControl::default());
    if big {
        if let Ok(mut guard) = cancel_slot.lock() {
            *guard = Some(control.clone());
        }
    }

    let (tx, rx) = mpsc::channel();
    let control_for_thread = control.clone();
    std::thread::spawn(move || {
        let _ = tx.send(op(control_for_thread));
    });

    RunningJob {
        label,
        verb,
        control,
        total_bytes,
        started_at: Instant::now(),
        big,
        receiver: rx,
    }
}

pub fn start_encrypt(
    vault: Vault,
    key: VaultKey,
    input: EncryptInput,
    total_bytes: u64,
    label: String,
    cancel_slot: &CancelSlot,
) -> RunningJob {
    spawn_job(
        "Encrypting",
        label,
        total_bytes,
        cancel_slot,
        move |control| {
            let mut vault = vault;
            let result = match input {
                EncryptInput::Files {
                    paths,
                    dest_folder_id,
                } => vault.encrypt_files(&paths, dest_folder_id.as_deref(), &key, &control),
                EncryptInput::Folder {
                    path,
                    dest_folder_id,
                } => vault.encrypt_folder(&path, dest_folder_id.as_deref(), &key, &control),
            };
            JobOutcome::Encrypt(vault, result)
        },
    )
}

pub fn start_export(
    vault: Vault,
    key: VaultKey,
    target: ExportTarget,
    total_bytes: u64,
    label: String,
    cancel_slot: &CancelSlot,
) -> RunningJob {
    spawn_job(
        "Exporting",
        label,
        total_bytes,
        cancel_slot,
        move |control| {
            let result = match target {
                ExportTarget::File { file_id, dest_dir } => {
                    vault.export_file(&file_id, &dest_dir, &key, &control)
                }
                ExportTarget::Folder {
                    folder_id,
                    dest_dir,
                } => vault.export_folder(&folder_id, &dest_dir, &key, &control),
            };
            JobOutcome::Export(result)
        },
    )
}

/// Clears a finished job's `JobControl` registration from `cancel_slot`
/// (only ever set for "big" jobs — see `spawn_job`).
pub fn finish_job(job: RunningJob, cancel_slot: &CancelSlot) {
    if job.big {
        if let Ok(mut guard) = cancel_slot.lock() {
            *guard = None;
        }
    }
}
