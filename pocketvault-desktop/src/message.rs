use std::path::PathBuf;

use iced::window;

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
    LockVault,
    SelectRow(String),
    EncryptFilesClicked,
    FilesPicked(Option<Vec<PathBuf>>),
    DeleteOriginalsDecision(Vec<PathBuf>, bool),
    ExportFile(String),
    ExportDestPicked(String, Option<PathBuf>),
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

    // Preview window
    PreviewWindowClosed(window::Id),
    WindowClosed(window::Id),
}
