use std::path::PathBuf;

use iced::window;
use pocketvault_core::Vault;
use crate::state::CancelTarget;

#[derive(Debug, Clone)]
pub enum Message {
    // Auth screen
    PasswordChanged(String),
    ConfirmPasswordChanged(String),
    OldPasswordChanged(String),
    SubmitAuth,
    ShowChangePasswordScreen,
    CancelChangePassword,

    // Vault browser
    NavigateFolder(Option<String>),
    ToggleFolderExpanded(String),
    LockVault,
    SelectRow(String),
    EncryptFilesClicked,
    FilesPicked(Option<Vec<PathBuf>>),
    EncryptFolderClicked,
    FolderPicked(Option<PathBuf>),
    ExportFile(String),
    ExportDestPicked(String, Option<PathBuf>),
    ExportFolder(String),
    ExportFolderDestPicked(String, Option<PathBuf>),
    PreviewFile(String),
    // Fire-and-forget acknowledgment dialogs (Exported / Export Failed / Preview
    // Error) — we don't care about the result, just that the async task completed.
    DialogDismissed,

    // New folder modal
    OpenNewFolderDialog,
    NewFolderNameChanged(String),
    ConfirmNewFolder,

    // Rename modal
    OpenRenameDialog(String, String),
    RenameTextChanged(String),
    ConfirmRename,

    // Delete confirm modal
    RequestDeleteFile(String),
    RequestDeleteFolder(String),
    ConfirmDelete,

    CancelModal,

    // Background vault jobs: one encrypt job runs at a time (more just queue
    // up — see `state::PocketVault::encrypt_queue`), one delete job, one
    // export job (independent of the other two, always allowed since it's
    // read-only).
    RequestCancelJob(CancelTarget),
    ConfirmCancelJob(CancelTarget),
    /// Periodic redraw while any job is running, so the ETA text (computed
    /// straight from a shared `JobControl` in `view()`) keeps counting down.
    Tick,
    /// The running encrypt job finished — same shape as before: the (possibly
    /// rolled-back) vault clone comes back and gets swapped into the session.
    EncryptJobFinished(Option<(Vault, Result<Vec<String>, String>)>),
    DeleteJobFinished(Option<(Vault, Result<Vec<String>, String>)>),
    /// Export jobs are read-only — nothing to swap back, just the outcome.
    ExportJobFinished(Option<Result<PathBuf, String>>),

    // Preview window
    PreviewWindowClosed(window::Id),
    WindowClosed(window::Id),
}
