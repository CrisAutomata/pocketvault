use std::path::PathBuf;

use iced::{window, Size, Task};
use rfd::{AsyncFileDialog, AsyncMessageDialog, MessageButtons, MessageDialogResult};

use pocketvault_core::Vault;

use crate::message::Message;
use crate::state::{build_meta_cache, AuthMode, AuthState, Modal, PocketVault, PreviewData, Screen, Session};

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

        Message::LockVault => {
            app.session = None;
            app.current_folder_id = None;
            app.selected_id.clear();
            app.screen = Screen::Auth(AuthState::new(AuthMode::Unlock));
            Task::none()
        }

        Message::SelectRow(id) => {
            app.selected_id = id;
            Task::none()
        }

        Message::EncryptFilesClicked => {
            if app.session.is_none() {
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
        Message::FilesPicked(paths_opt) => encrypt_files(app, paths_opt),
        Message::DeleteOriginalsDecision(paths, delete) => {
            if delete {
                for path in &paths {
                    let _ = std::fs::remove_file(path);
                }
            }
            Task::none()
        }

        Message::ExportFile(file_id) => window::run(app.main_window, |w| {
            let handle = w.window_handle().expect("window handle");
            AsyncFileDialog::new().set_title("Choose export folder").set_parent(&handle)
        })
        .then(move |dialog| {
            let file_id = file_id.clone();
            Task::perform(dialog.pick_folder(), move |result| {
                Message::ExportDestPicked(file_id, result.map(|h| h.path().to_path_buf()))
            })
        }),
        Message::ExportDestPicked(file_id, dest_dir) => export_file(app, &file_id, dest_dir),

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
                    if let Some(session) = app.session.as_mut() {
                        if let Err(e) = session.vault.delete_file(&fid) {
                            eprintln!("Delete file error: {e}");
                        }
                        session.meta_cache.remove(&fid);
                    }
                } else if let Some(folder_id) = folder_id {
                    if let Some(session) = app.session.as_mut() {
                        if let Err(e) = session.vault.delete_folder(&folder_id) {
                            eprintln!("Delete folder error: {e}");
                        }
                    }
                    if app.current_folder_id.as_deref() == Some(folder_id.as_str()) {
                        app.current_folder_id = None;
                    }
                }
            }
            Task::none()
        }

        Message::CancelModal => {
            app.modal = None;
            Task::none()
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

fn encrypt_files(app: &mut PocketVault, paths_opt: Option<Vec<PathBuf>>) -> Task<Message> {
    let paths = match paths_opt {
        Some(paths) if !paths.is_empty() => paths,
        _ => return Task::none(),
    };
    let folder_id = app.current_folder_id.clone();
    let main_window = app.main_window;
    let session = match app.session.as_mut() {
        Some(s) => s,
        None => return Task::none(),
    };

    for path in &paths {
        match session
            .vault
            .encrypt_file(path, folder_id.as_deref(), &session.key)
        {
            Ok(id) => {
                if let Ok(meta) = session.vault.read_metadata(&id, &session.key) {
                    session.meta_cache.insert(id, meta);
                }
            }
            Err(e) => eprintln!("Encrypt error {path:?}: {e}"),
        }
    }

    window::run(main_window, |w| {
        let handle = w.window_handle().expect("window handle");
        AsyncMessageDialog::new()
            .set_title("Delete original files?")
            .set_description(
                "Files were encrypted successfully.\n\nDelete the original plaintext files?",
            )
            .set_buttons(MessageButtons::YesNo)
            .set_parent(&handle)
    })
    .then(move |dialog| {
        let paths = paths.clone();
        Task::perform(dialog.show(), move |result| {
            Message::DeleteOriginalsDecision(paths, result == MessageDialogResult::Yes)
        })
    })
}

fn export_file(app: &PocketVault, file_id: &str, dest_dir: Option<PathBuf>) -> Task<Message> {
    let dest_dir = match dest_dir {
        Some(d) => d,
        None => return Task::none(),
    };
    let session = match &app.session {
        Some(s) => s,
        None => return Task::none(),
    };

    let (title, description) = match session.vault.export_file(file_id, &dest_dir, &session.key) {
        Ok(path) => ("Exported".to_string(), format!("Saved to:\n{}", path.display())),
        Err(e) => ("Export Failed".to_string(), e.to_string()),
    };
    window::run(app.main_window, move |w| {
        let handle = w.window_handle().expect("window handle");
        AsyncMessageDialog::new()
            .set_title(title)
            .set_description(description)
            .set_buttons(MessageButtons::Ok)
            .set_parent(&handle)
    })
    .then(|dialog| Task::perform(dialog.show(), |_| Message::DialogDismissed))
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
