mod message;
mod state;
mod theme;
mod update;
mod view;

use std::collections::HashMap;

use iced::{window, Size, Subscription, Task};

use message::Message;
use pocketvault_core::Vault;
use state::{vault_base_dir, AuthMode, AuthState, PocketVault, Screen};

fn boot() -> (PocketVault, Task<Message>) {
    let base_dir = vault_base_dir();
    let _ = std::fs::create_dir_all(&base_dir);

    let mode = if Vault::exists(&base_dir) {
        AuthMode::Unlock
    } else {
        AuthMode::Create
    };

    let (main_window, open_task) = window::open(window::Settings {
        size: Size::new(900.0, 580.0),
        min_size: Some(Size::new(900.0, 580.0)),
        ..Default::default()
    });

    let app = PocketVault {
        base_dir,
        session: None,
        screen: Screen::Auth(AuthState::new(mode)),
        current_folder_id: None,
        selected_id: String::new(),
        expanded_folders: std::collections::HashSet::new(),
        modal: None,
        previews: HashMap::new(),
        main_window,
        running_encrypt: None,
        encrypt_queue: std::collections::VecDeque::new(),
        active_delete_job: None,
        active_export_job: None,
        active_repack_job: None,
        next_job_id: 0,
    };

    (app, open_task.discard())
}

fn title(app: &PocketVault, id: window::Id) -> String {
    if id == app.main_window {
        "PocketVault".to_string()
    } else if let Some(data) = app.previews.get(&id) {
        format!("Preview — {}", data.file_name)
    } else {
        "PocketVault".to_string()
    }
}

fn subscription(app: &PocketVault) -> Subscription<Message> {
    let mut subs = vec![window::close_events().map(Message::WindowClosed)];

    // Periodic redraw while any job is running, so the ETA/progress text
    // (read straight from a shared `JobControl` on each render — see
    // `state::RunningEncryptJob`) keeps visibly counting down.
    let any_job_running = app.running_encrypt.is_some()
        || app.active_delete_job.is_some()
        || app.active_export_job.is_some()
        || app.active_repack_job.is_some();
    if any_job_running {
        subs.push(iced::time::every(std::time::Duration::from_millis(300)).map(|_| Message::Tick));
    }

    Subscription::batch(subs)
}

fn main() -> iced::Result {
    iced::daemon(boot, update::update, view::window_view)
        .title(title)
        .subscription(subscription)
        .run()
}
