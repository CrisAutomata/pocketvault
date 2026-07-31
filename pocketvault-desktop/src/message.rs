use std::path::PathBuf;

use iced::window;
use pocketvault_core::Vault;

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

    // Background vault jobs (big encrypt/export/delete)
    RequestCancelJob,
    ConfirmCancelJob,
    /// No-op — used by the busy-scrim to swallow clicks while a job runs.
    Ignore,
    /// Encrypt/delete jobs mutate the vault, so the (possibly rolled-back)
    /// clone comes back and gets swapped into the session.
    MutatingJobFinished(Option<(Vault, Result<Vec<String>, String>)>),
    /// Export jobs are read-only — nothing to swap back, just the outcome.
    ExportJobFinished(Option<Result<PathBuf, String>>),

    // Preview window
    PreviewWindowClosed(window::Id),
    WindowClosed(window::Id),
}
