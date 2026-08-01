pub mod auth;
pub mod modals;
pub mod preview;
pub mod rows;
pub mod vault_browser;

use iced::{window, Element};

use crate::message::Message;
use crate::state::{PocketVault, Screen};

pub fn main_window(app: &PocketVault) -> Element<'_, Message> {
    let base = match &app.screen {
        Screen::Auth(auth) => auth::view(auth),
        Screen::Vault => vault_browser::view(app),
    };
    modals::wrap(base, app)
}

pub fn window_view(app: &PocketVault, id: window::Id) -> Element<'_, Message> {
    if id == app.main_window {
        main_window(app)
    } else if let Some(data) = app.previews.get(&id) {
        preview::view(id, data)
    } else {
        iced::widget::column![].into()
    }
}
