use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};

use iced::{window, Size, Task};
use rfd::{AsyncFileDialog, AsyncMessageDialog, MessageButtons};

use pocketvault_core::{Vault, VaultKey};

use crate::message::Message;
use crate::state::{
    build_meta_cache, vault_folder_total_size, AuthMode, AuthState, JobKind, Modal, PocketVault,
    PreviewData, Screen, Session, VaultJob, BIG_JOB_THRESHOLD_BYTES,
};

fn auth_mut(app: &mut PocketVault) -> Option<&mut AuthState> {
    match &mut app.screen {
        Screen::Auth(auth) => Some(auth),
        Screen::Vault => None,
    }
}

pub fn update(app: &mut PocketVault, message: Message) -> Task<Message> {
    match message {
        Message::PasswordChanged(s) => {
            if let Some(auth) = auth_mut(app) {
                auth.password = s;
            }
            Task::none()
        }
        Message::ConfirmPasswordChanged(s) => {
            if let Some(auth) = auth_mut(app) {
                auth.confirm = s;
            }
            Task::none()
        }
        Message::OldPasswordChanged(s) => {
            if let Some(auth) = auth_mut(app) {
                auth.old_password = s;
            }
            Task::none()
        }

        Message::SubmitAuth => {
            let mode = match auth_mut(app) {
                Some(auth) => auth.mode,
                None => return Task::none(),
            };
            match mode {
                AuthMode::ChangePassword => submit_change_password(app),
                AuthMode::Create => submit_create(app),
                AuthMode::Unlock => submit_unlock(app),
            }
            Task::none()
        }

        Message::ShowChangePasswordScreen => {
            if app.session.is_some() {
                app.screen = Screen::Auth(AuthState::new(AuthMode::ChangePassword));
            }
            Task::none()
        }
        Message::CancelChangePassword => {
            if app.session.is_some() {
                app.screen = Screen::Vault;
            }
            Task::none()
        }

        Message::NavigateFolder(folder_id) => {
            app.current_folder_id = folder_id;
            Task::none()
        }

        Message::ToggleFolderExpanded(folder_id) => {
            if !app.expanded_folders.remove(&folder_id) {
                app.expanded_folders.insert(folder_id);
            }
            Task::none()
        }

        Message::LockVault => {
            app.session = None;
            app.current_folder_id = None;
            app.selected_id.clear();
            app.expanded_folders.clear();
            app.screen = Screen::Auth(AuthState::new(AuthMode::Unlock));
            Task::none()
        }

        Message::SelectRow(id) => {
            app.selected_id = id;
            Task::none()
        }

        Message::EncryptFilesClicked => {
            if app.session.is_none() || app.active_job.is_some() {
                return Task::none();
            }
            window::run(app.main_window, |w| {
                let handle = w.window_handle().expect("window handle");
                AsyncFileDialog::new()
                    .set_title("Choose files to encrypt")
                    .set_parent(&handle)
            })
            .then(|dialog| {
                Task::perform(dialog.pick_files(), |result| {
                    Message::FilesPicked(
                        result.map(|handles| handles.iter().map(|h| h.path().to_path_buf()).collect()),
                    )
                })
            })
        }
        Message::FilesPicked(paths_opt) => start_encrypt_files(app, paths_opt),

        Message::EncryptFolderClicked => {
            if app.session.is_none() || app.active_job.is_some() {
                return Task::none();
            }
            window::run(app.main_window, |w| {
                let handle = w.window_handle().expect("window handle");
                AsyncFileDialog::new()
                    .set_title("Choose a folder to encrypt")
                    .set_parent(&handle)
            })
            .then(|dialog| {
                Task::perform(dialog.pick_folder(), |result| {
                    Message::FolderPicked(result.map(|h| h.path().to_path_buf()))
                })
            })
        }
        Message::FolderPicked(path_opt) => start_encrypt_folder(app, path_opt),

        Message::ExportFile(file_id) => {
            if app.active_job.is_some() {
                return Task::none();
            }
            window::run(app.main_window, |w| {
                let handle = w.window_handle().expect("window handle");
                AsyncFileDialog::new().set_title("Choose export folder").set_parent(&handle)
            })
            .then(move |dialog| {
                let file_id = file_id.clone();
                Task::perform(dialog.pick_folder(), move |result| {
                    Message::ExportDestPicked(file_id, result.map(|h| h.path().to_path_buf()))
                })
            })
        }
        Message::ExportDestPicked(file_id, dest_dir) => start_export_file(app, &file_id, dest_dir),

        Message::ExportFolder(folder_id) => {
            if app.active_job.is_some() {
                return Task::none();
            }
            window::run(app.main_window, |w| {
                let handle = w.window_handle().expect("window handle");
                AsyncFileDialog::new().set_title("Choose export destination").set_parent(&handle)
            })
            .then(move |dialog| {
                let folder_id = folder_id.clone();
                Task::perform(dialog.pick_folder(), move |result| {
                    Message::ExportFolderDestPicked(folder_id.clone(), result.map(|h| h.path().to_path_buf()))
                })
            })
        }
        Message::ExportFolderDestPicked(folder_id, dest_dir) => start_export_folder(app, &folder_id, dest_dir),

        Message::PreviewFile(file_id) => preview_file(app, &file_id),
        Message::DialogDismissed => Task::none(),

        Message::OpenNewFolderDialog => {
            app.modal = Some(Modal::NewFolder { name: String::new() });
            Task::none()
        }
        Message::NewFolderNameChanged(s) => {
            if let Some(Modal::NewFolder { name }) = &mut app.modal {
                *name = s;
            }
            Task::none()
        }
        Message::ConfirmNewFolder => {
            if let Some(Modal::NewFolder { name }) = app.modal.take() {
                let trimmed = name.trim().to_string();
                if !trimmed.is_empty() {
                    if let Some(session) = app.session.as_mut() {
                        let _ = session
                            .vault
                            .create_folder(&trimmed, app.current_folder_id.as_deref());
                    }
                }
            }
            Task::none()
        }

        Message::OpenRenameDialog(folder_id, current_name) => {
            app.modal = Some(Modal::Rename {
                folder_id,
                text: current_name,
            });
            Task::none()
        }
        Message::RenameTextChanged(s) => {
            if let Some(Modal::Rename { text, .. }) = &mut app.modal {
                *text = s;
            }
            Task::none()
        }
        Message::ConfirmRename => {
            if let Some(Modal::Rename { folder_id, text }) = app.modal.take() {
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    if let Some(session) = app.session.as_mut() {
                        if let Err(e) = session.vault.rename_folder(&folder_id, &trimmed) {
                            eprintln!("Rename folder error: {e}");
                        }
                    }
                }
            }
            Task::none()
        }

        Message::RequestDeleteFile(id) => {
            app.modal = Some(Modal::DeleteConfirm {
                file_id: Some(id),
                folder_id: None,
            });
            Task::none()
        }
        Message::RequestDeleteFolder(id) => {
            app.modal = Some(Modal::DeleteConfirm {
                file_id: None,
                folder_id: Some(id),
            });
            Task::none()
        }
        Message::ConfirmDelete => {
            if let Some(Modal::DeleteConfirm { file_id, folder_id }) = app.modal.take() {
                if let Some(fid) = file_id {
                    return start_delete_file(app, fid);
                } else if let Some(folder_id) = folder_id {
                    return start_delete_folder(app, folder_id);
                }
            }
            Task::none()
        }

        Message::CancelModal => {
            app.modal = None;
            Task::none()
        }

        Message::RequestCancelJob => {
            if app.active_job.as_ref().is_some_and(|j| j.cancel.is_some()) {
                app.modal = Some(Modal::ConfirmCancelJob);
            }
            Task::none()
        }
        Message::ConfirmCancelJob => {
            app.modal = None;
            if let Some(job) = app.active_job.as_mut() {
                if let Some(cancel) = &job.cancel {
                    cancel.store(true, Ordering::Relaxed);
                }
                job.cancelling = true;
            }
            Task::none()
        }
        Message::Ignore => Task::none(),

        Message::MutatingJobFinished(outcome) => {
            let kind = app.active_job.take().map(|j| j.kind);
            match outcome {
                None => {}
                Some((vault, Ok(ids))) => {
                    if let Some(session) = app.session.as_mut() {
                        session.vault = vault;
                        match &kind {
                            Some(JobKind::DeleteFile) | Some(JobKind::DeleteFolder(_)) => {
                                for id in &ids {
                                    session.meta_cache.remove(id);
                                }
                            }
                            _ => {
                                for id in ids {
                                    if let Ok(meta) = session.vault.read_metadata(&id, &session.key) {
                                        session.meta_cache.insert(id, meta);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(JobKind::DeleteFolder(fid)) = &kind {
                        if app.current_folder_id.as_deref() == Some(fid.as_str()) {
                            app.current_folder_id = None;
                        }
                    }
                }
                Some((vault, Err(e))) => {
                    if let Some(session) = app.session.as_mut() {
                        session.vault = vault;
                    }
                    eprintln!("Job failed: {e}");
                }
            }
            Task::none()
        }
        Message::ExportJobFinished(outcome) => {
            app.active_job = None;
            let (title, description) = match outcome {
                None => return Task::none(),
                Some(Ok(path)) => ("Exported".to_string(), format!("Saved to:\n{}", path.display())),
                Some(Err(e)) => ("Export Failed".to_string(), e),
            };
            completion_dialog(app.main_window, title, description)
        }

        Message::PreviewWindowClosed(id) => window::close(id),

        Message::WindowClosed(id) => {
            if id == app.main_window {
                iced::exit()
            } else {
                app.previews.remove(&id);
                Task::none()
            }
        }
    }
}

fn submit_change_password(app: &mut PocketVault) {
    let (old_pw, new_pw, confirm) = match auth_mut(app) {
        Some(auth) => (
            auth.old_password.clone(),
            auth.password.clone(),
            auth.confirm.clone(),
        ),
        None => return,
    };

    if new_pw.len() < 8 {
        if let Some(auth) = auth_mut(app) {
            auth.error = "New password must be at least 8 characters.".into();
        }
        return;
    }
    if new_pw != confirm {
        if let Some(auth) = auth_mut(app) {
            auth.error = "Passwords do not match.".into();
        }
        return;
    }
    if let Some(auth) = auth_mut(app) {
        auth.error.clear();
        auth.busy = true;
    }

    if let Some(session) = app.session.as_mut() {
        match session.vault.meta.change_password(&old_pw, &new_pw) {
            Ok(()) => {
                let _ = session.vault.save();
                app.screen = Screen::Vault;
            }
            Err(e) => {
                if let Some(auth) = auth_mut(app) {
                    auth.error = e.to_string();
                    auth.busy = false;
                }
            }
        }
    }
}

fn submit_create(app: &mut PocketVault) {
    let (pw, confirm) = match auth_mut(app) {
        Some(auth) => (auth.password.clone(), auth.confirm.clone()),
        None => return,
    };

    if pw.len() < 8 {
        if let Some(auth) = auth_mut(app) {
            auth.error = "Password must be at least 8 characters.".into();
        }
        return;
    }
    if pw != confirm {
        if let Some(auth) = auth_mut(app) {
            auth.error = "Passwords do not match.".into();
        }
        return;
    }
    if let Some(auth) = auth_mut(app) {
        auth.error.clear();
        auth.busy = true;
    }

    match Vault::create(&app.base_dir, &pw) {
        Ok(vault) => match vault.meta.unlock(&pw) {
            Ok(key) => {
                let meta_cache = build_meta_cache(&vault, &key);
                app.session = Some(Session {
                    vault,
                    key,
                    meta_cache,
                });
                app.screen = Screen::Vault;
                app.current_folder_id = None;
                app.selected_id.clear();
            }
            Err(e) => {
                if let Some(auth) = auth_mut(app) {
                    auth.error = e.to_string();
                    auth.busy = false;
                }
            }
        },
        Err(e) => {
            if let Some(auth) = auth_mut(app) {
                auth.error = e.to_string();
                auth.busy = false;
            }
        }
    }
}

fn submit_unlock(app: &mut PocketVault) {
    let pw = match auth_mut(app) {
        Some(auth) => auth.password.clone(),
        None => return,
    };
    if let Some(auth) = auth_mut(app) {
        auth.error.clear();
        auth.busy = true;
    }

    match Vault::open(&app.base_dir) {
        Ok(vault) => match vault.meta.unlock(&pw) {
            Ok(key) => {
                let meta_cache = build_meta_cache(&vault, &key);
                app.session = Some(Session {
                    vault,
                    key,
                    meta_cache,
                });
                app.screen = Screen::Vault;
                app.current_folder_id = None;
                app.selected_id.clear();
            }
            Err(_) => {
                if let Some(auth) = auth_mut(app) {
                    auth.error = "Invalid password.".into();
                    auth.busy = false;
                }
            }
        },
        Err(e) => {
            if let Some(auth) = auth_mut(app) {
                auth.error = e.to_string();
                auth.busy = false;
            }
        }
    }
}

/// Spawns `work` on a background OS thread with an owned clone of the vault
/// (and key), bridging its result back into a `Message` via a oneshot channel
/// without blocking the UI thread. The original `session.vault` is untouched
/// until `MutatingJobFinished` swaps in whatever the thread returns.
fn spawn_mutating_job(
    app: &mut PocketVault,
    label: String,
    kind: JobKind,
    cancellable: bool,
    work: impl FnOnce(Vault, VaultKey, Arc<AtomicBool>) -> (Vault, Result<Vec<String>, String>)
        + Send
        + 'static,
) -> Task<Message> {
    let session = match app.session.as_ref() {
        Some(s) => s,
        None => return Task::none(),
    };
    let vault = session.vault.clone();
    let key = session.key.clone();
    let cancel = Arc::new(AtomicBool::new(false));

    app.active_job = Some(VaultJob {
        label,
        kind,
        cancel: if cancellable { Some(cancel.clone()) } else { None },
        cancelling: false,
    });

    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work(vault, key, cancel));
    });

    Task::perform(async move { rx.await.ok() }, Message::MutatingJobFinished)
}

/// Same idea as `spawn_mutating_job`, but for read-only export jobs — nothing
/// needs to be swapped back into the session when it finishes.
fn spawn_export_job(
    app: &mut PocketVault,
    label: String,
    kind: JobKind,
    work: impl FnOnce(Vault, VaultKey, Arc<AtomicBool>) -> Result<PathBuf, String> + Send + 'static,
) -> Task<Message> {
    let session = match app.session.as_ref() {
        Some(s) => s,
        None => return Task::none(),
    };
    let vault = session.vault.clone();
    let key = session.key.clone();
    let cancel = Arc::new(AtomicBool::new(false));

    app.active_job = Some(VaultJob {
        label,
        kind,
        cancel: Some(cancel.clone()),
        cancelling: false,
    });

    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work(vault, key, cancel));
    });

    Task::perform(async move { rx.await.ok() }, Message::ExportJobFinished)
}

fn completion_dialog(main_window: window::Id, title: String, description: String) -> Task<Message> {
    window::run(main_window, move |w| {
        let handle = w.window_handle().expect("window handle");
        AsyncMessageDialog::new()
            .set_title(title)
            .set_description(description)
            .set_buttons(MessageButtons::Ok)
            .set_parent(&handle)
    })
    .then(|dialog| Task::perform(dialog.show(), |_| Message::DialogDismissed))
}

fn start_encrypt_files(app: &mut PocketVault, paths_opt: Option<Vec<PathBuf>>) -> Task<Message> {
    let paths = match paths_opt {
        Some(paths) if !paths.is_empty() => paths,
        _ => return Task::none(),
    };
    if app.session.is_none() || app.active_job.is_some() {
        return Task::none();
    }
    let folder_id = app.current_folder_id.clone();
    let total: u64 = paths.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();

    if total < BIG_JOB_THRESHOLD_BYTES {
        let session = app.session.as_mut().unwrap();
        match session.vault.encrypt_files(&paths, folder_id.as_deref(), &session.key, &AtomicBool::new(false)) {
            Ok(ids) => {
                for id in ids {
                    if let Ok(meta) = session.vault.read_metadata(&id, &session.key) {
                        session.meta_cache.insert(id, meta);
                    }
                }
            }
            Err(e) => eprintln!("Encrypt error: {e}"),
        }
        return Task::none();
    }

    let label = if paths.len() == 1 {
        paths[0].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into())
    } else {
        format!("{} files", paths.len())
    };
    spawn_mutating_job(app, label, JobKind::EncryptFiles, true, move |mut vault, key, cancel| {
        let result = vault
            .encrypt_files(&paths, folder_id.as_deref(), &key, &cancel)
            .map_err(|e| e.to_string());
        (vault, result)
    })
}

fn start_encrypt_folder(app: &mut PocketVault, path_opt: Option<PathBuf>) -> Task<Message> {
    let path = match path_opt {
        Some(p) => p,
        None => return Task::none(),
    };
    if app.session.is_none() || app.active_job.is_some() {
        return Task::none();
    }
    let folder_id = app.current_folder_id.clone();
    let total = pocketvault_core::dir_total_size(&path).unwrap_or(0);

    if total < BIG_JOB_THRESHOLD_BYTES {
        let session = app.session.as_mut().unwrap();
        match session.vault.encrypt_folder(&path, folder_id.as_deref(), &session.key, &AtomicBool::new(false)) {
            Ok(ids) => {
                for id in ids {
                    if let Ok(meta) = session.vault.read_metadata(&id, &session.key) {
                        session.meta_cache.insert(id, meta);
                    }
                }
            }
            Err(e) => eprintln!("Encrypt folder error {path:?}: {e}"),
        }
        return Task::none();
    }

    let label = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "folder".into());
    spawn_mutating_job(app, label, JobKind::EncryptFolder, true, move |mut vault, key, cancel| {
        let result = vault
            .encrypt_folder(&path, folder_id.as_deref(), &key, &cancel)
            .map_err(|e| e.to_string());
        (vault, result)
    })
}

fn start_export_file(app: &mut PocketVault, file_id: &str, dest_dir: Option<PathBuf>) -> Task<Message> {
    let dest_dir = match dest_dir {
        Some(d) => d,
        None => return Task::none(),
    };
    if app.active_job.is_some() {
        return Task::none();
    }
    let session = match app.session.as_ref() {
        Some(s) => s,
        None => return Task::none(),
    };
    let size = session.meta_cache.get(file_id).map(|m| m.original_size).unwrap_or(0);
    let name = session
        .meta_cache
        .get(file_id)
        .map(|m| m.original_name.clone())
        .unwrap_or_else(|| "file".to_string());

    if size < BIG_JOB_THRESHOLD_BYTES {
        let (title, description) = match session.vault.export_file(file_id, &dest_dir, &session.key, &AtomicBool::new(false)) {
            Ok(path) => ("Exported".to_string(), format!("Saved to:\n{}", path.display())),
            Err(e) => ("Export Failed".to_string(), e.to_string()),
        };
        return completion_dialog(app.main_window, title, description);
    }

    let file_id = file_id.to_string();
    spawn_export_job(app, name, JobKind::ExportFile, move |vault, key, cancel| {
        vault.export_file(&file_id, &dest_dir, &key, &cancel).map_err(|e| e.to_string())
    })
}

fn start_export_folder(app: &mut PocketVault, folder_id: &str, dest_dir: Option<PathBuf>) -> Task<Message> {
    let dest_dir = match dest_dir {
        Some(d) => d,
        None => return Task::none(),
    };
    if app.active_job.is_some() {
        return Task::none();
    }
    let session = match app.session.as_ref() {
        Some(s) => s,
        None => return Task::none(),
    };
    let total = vault_folder_total_size(session, folder_id);
    let name = session
        .vault
        .all_folders()
        .iter()
        .find(|f| f.id == folder_id)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "folder".to_string());

    if total < BIG_JOB_THRESHOLD_BYTES {
        let (title, description) = match session.vault.export_folder(folder_id, &dest_dir, &session.key, &AtomicBool::new(false)) {
            Ok(path) => ("Exported".to_string(), format!("Saved to:\n{}", path.display())),
            Err(e) => ("Export Failed".to_string(), e.to_string()),
        };
        return completion_dialog(app.main_window, title, description);
    }

    let folder_id = folder_id.to_string();
    spawn_export_job(app, name, JobKind::ExportFolder, move |vault, key, cancel| {
        vault.export_folder(&folder_id, &dest_dir, &key, &cancel).map_err(|e| e.to_string())
    })
}

fn start_delete_file(app: &mut PocketVault, file_id: String) -> Task<Message> {
    if app.active_job.is_some() {
        return Task::none();
    }
    let session = match app.session.as_mut() {
        Some(s) => s,
        None => return Task::none(),
    };
    let size = session.meta_cache.get(&file_id).map(|m| m.original_size).unwrap_or(0);

    if size < BIG_JOB_THRESHOLD_BYTES {
        if let Err(e) = session.vault.delete_file(&file_id) {
            eprintln!("Delete file error: {e}");
        }
        session.meta_cache.remove(&file_id);
        return Task::none();
    }

    let label = session
        .meta_cache
        .get(&file_id)
        .map(|m| m.original_name.clone())
        .unwrap_or_else(|| "file".to_string());
    spawn_mutating_job(app, label, JobKind::DeleteFile, false, move |mut vault, _key, _cancel| {
        let result = vault.delete_file(&file_id).map(|()| vec![file_id]).map_err(|e| e.to_string());
        (vault, result)
    })
}

fn start_delete_folder(app: &mut PocketVault, folder_id: String) -> Task<Message> {
    if app.active_job.is_some() {
        return Task::none();
    }
    let session = match app.session.as_mut() {
        Some(s) => s,
        None => return Task::none(),
    };
    let total = vault_folder_total_size(session, &folder_id);

    if total < BIG_JOB_THRESHOLD_BYTES {
        match session.vault.delete_folder(&folder_id) {
            Ok(ids) => {
                for id in ids {
                    session.meta_cache.remove(&id);
                }
            }
            Err(e) => eprintln!("Delete folder error: {e}"),
        }
        if app.current_folder_id.as_deref() == Some(folder_id.as_str()) {
            app.current_folder_id = None;
        }
        return Task::none();
    }

    let label = session
        .vault
        .all_folders()
        .iter()
        .find(|f| f.id == folder_id)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "folder".to_string());
    spawn_mutating_job(app, label, JobKind::DeleteFolder(folder_id.clone()), false, move |mut vault, _key, _cancel| {
        let result = vault.delete_folder(&folder_id).map_err(|e| e.to_string());
        (vault, result)
    })
}

fn preview_error_dialog(main_window: window::Id, description: String) -> Task<Message> {
    window::run(main_window, move |w| {
        let handle = w.window_handle().expect("window handle");
        AsyncMessageDialog::new()
            .set_title("Preview Error")
            .set_description(description)
            .set_buttons(MessageButtons::Ok)
            .set_parent(&handle)
    })
    .then(|dialog| Task::perform(dialog.show(), |_| Message::DialogDismissed))
}

fn preview_file(app: &mut PocketVault, file_id: &str) -> Task<Message> {
    let main_window = app.main_window;
    let session = match &app.session {
        Some(s) => s,
        None => return Task::none(),
    };

    match session.vault.read_to_memory(file_id, &session.key) {
        Ok((meta, data)) => {
            let is_image = meta.mime_type.starts_with("image/");
            let is_text = meta.mime_type == "text/plain";

            if !is_image && !is_text {
                return Task::none();
            }

            let (is_image, image_rgba, text) = if is_image {
                let fmt = match meta.mime_type.as_str() {
                    "image/jpeg" => Some(image::ImageFormat::Jpeg),
                    "image/png" => Some(image::ImageFormat::Png),
                    "image/gif" => Some(image::ImageFormat::Gif),
                    "image/webp" => Some(image::ImageFormat::WebP),
                    "image/bmp" => Some(image::ImageFormat::Bmp),
                    _ => None,
                };
                let decoded = fmt.and_then(|f| image::load_from_memory_with_format(&data, f).ok());
                let image_rgba = decoded.map(|img| {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    (w, h, rgba.into_raw())
                });
                if image_rgba.is_none() {
                    return preview_error_dialog(main_window, "Could not decode image.".to_string());
                }
                (true, image_rgba, String::new())
            } else {
                (false, None, String::from_utf8_lossy(&data).to_string())
            };

            let (id, task) = window::open(window::Settings {
                size: Size::new(720.0, 540.0),
                min_size: Some(Size::new(640.0, 480.0)),
                ..Default::default()
            });
            app.previews.insert(
                id,
                PreviewData {
                    file_name: meta.original_name,
                    is_image,
                    image_rgba,
                    text,
                },
            );
            task.discard()
        }
        Err(e) => preview_error_dialog(main_window, e.to_string()),
    }
}
