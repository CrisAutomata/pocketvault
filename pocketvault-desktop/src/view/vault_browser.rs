use iced::alignment::{Horizontal, Vertical};
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Element, Length};

use crate::message::Message;
use crate::state::{FileItem, FolderItem, PocketVault, Session};
use crate::theme;
use pocketvault_core::is_previewable;

use super::rows::{file_row, folder_row, section_label, sidebar_item};

fn sidebar_folders(session: &Session) -> Vec<FolderItem> {
    session
        .vault
        .all_folders()
        .iter()
        .filter(|f| f.parent_id.is_none())
        .map(|f| FolderItem {
            id: f.id.clone(),
            name: f.name.clone(),
            item_count: session.vault.meta.folder_file_count(&f.id),
        })
        .collect()
}

fn content_folders(session: &Session, folder_id: Option<&str>) -> Vec<FolderItem> {
    session
        .vault
        .folders(folder_id)
        .iter()
        .map(|f| FolderItem {
            id: f.id.clone(),
            name: f.name.clone(),
            item_count: session.vault.meta.folder_file_count(&f.id),
        })
        .collect()
}

fn content_files(session: &Session, folder_id: Option<&str>) -> Vec<FileItem> {
    session
        .vault
        .files_in_folder(folder_id)
        .iter()
        .map(|f| {
            if let Some(meta) = session.meta_cache.get(&f.id) {
                FileItem {
                    id: f.id.clone(),
                    display_name: meta.original_name.clone(),
                    modified_str: crate::state::format_ts(meta.modified_ts),
                    size_str: crate::state::format_size(meta.original_size),
                    kind: ".pv File".into(),
                    can_preview: is_previewable(&meta.mime_type),
                }
            } else {
                FileItem {
                    id: f.id.clone(),
                    display_name: f.pv_filename.clone(),
                    modified_str: "—".into(),
                    size_str: "—".into(),
                    kind: ".pv File".into(),
                    can_preview: false,
                }
            }
        })
        .collect()
}

fn divider<'a>() -> Element<'a, Message> {
    container(column![])
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(theme::container_with_bg(theme::divider()))
        .into()
}

fn vdivider<'a>() -> Element<'a, Message> {
    container(column![])
        .width(Length::Fixed(1.0))
        .height(Length::Fill)
        .style(theme::container_with_bg(theme::divider()))
        .into()
}

pub fn view(app: &PocketVault) -> Element<'_, Message> {
    let session = match &app.session {
        Some(s) => s,
        None => return column![].into(),
    };

    let folder_id = app.current_folder_id.as_deref();
    let sidebar_top = sidebar_folders(session);
    let folders = content_folders(session, folder_id);
    let files = content_files(session, folder_id);
    let total_items = (folders.len() + files.len()) as i64;

    let current_path = match folder_id {
        None => "PocketVault › Vault".to_string(),
        Some(id) => {
            let name = session
                .vault
                .all_folders()
                .iter()
                .find(|f| f.id == id)
                .map(|f| f.name.as_str())
                .unwrap_or("Folder");
            format!("PocketVault › Vault › {name}")
        }
    };

    // ── Toolbar ──────────────────────────────────────────────────────
    let toolbar = container(
        row![
            row![text("🔒").size(16), text("Vault").size(16).color(theme::text())]
                .spacing(6)
                .align_y(Vertical::Center)
                .width(Length::Fixed(200.0)),
            row![
                button(row![text("+").size(16), text("Encrypt").size(12)].spacing(4))
                    .padding([6, 12])
                    .style(|_theme, status| theme::primary_button(status))
                    .on_press(Message::EncryptFilesClicked),
                button(row![text("📁").size(13), text("New Folder").size(12)].spacing(4))
                    .padding([6, 12])
                    .style(|_theme, status| theme::secondary_button(status))
                    .on_press(Message::OpenNewFolderDialog),
            ]
            .spacing(8)
            .width(Length::Fill),
            text(format!(
                "{total_items} item{}",
                if total_items == 1 { "" } else { "s" }
            ))
            .size(12)
            .color(theme::text_secondary()),
        ]
        .spacing(12)
        .padding([0, 16])
        .align_y(Vertical::Center),
    )
    .height(Length::Fixed(52.0))
    .width(Length::Fill)
    .align_y(Vertical::Center)
    .style(theme::container_with_bg(theme::sidebar_bg()));

    // ── Sidebar ──────────────────────────────────────────────────────
    let sidebar_list = scrollable(
        column(
            sidebar_top
                .into_iter()
                .map(|f| {
                    let selected = folder_id == Some(f.id.as_str());
                    sidebar_item(
                        f.name,
                        Some(f.item_count as i64),
                        selected,
                        false,
                        true,
                        Message::NavigateFolder(Some(f.id.clone())),
                        Some(Message::RequestDeleteFolder(f.id)),
                    )
                })
                .collect::<Vec<_>>(),
        )
        .spacing(0),
    )
    .height(Length::Fill)
    .width(Length::Fill);

    let sidebar = container(
        column![
            section_label("VAULT"),
            sidebar_item(
                "Vault".to_string(),
                Some(total_items),
                folder_id.is_none(),
                false,
                false,
                Message::NavigateFolder(None),
                None,
            ),
            container(column![]).height(Length::Fixed(8.0)),
            section_label("FOLDERS"),
            sidebar_list,
            container(column![]).height(Length::Fixed(8.0)),
            section_label("SESSION"),
            sidebar_item(
                "Change Password".to_string(),
                None,
                false,
                false,
                false,
                Message::ShowChangePasswordScreen,
                None,
            ),
            sidebar_item(
                "Lock Vault".to_string(),
                None,
                false,
                true,
                false,
                Message::LockVault,
                None,
            ),
        ]
        .spacing(0)
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fixed(220.0))
    .height(Length::Fill)
    .style(theme::container_with_bg(theme::sidebar_bg()));

    // ── Content ──────────────────────────────────────────────────────
    let header = container(
        row![
            container(column![]).width(Length::Fixed(22.0)),
            text("Name")
                .size(11)
                .color(theme::text_secondary())
                .width(Length::Fill),
            text("Modified")
                .size(11)
                .color(theme::text_secondary())
                .width(Length::Fixed(120.0)),
            text("Size")
                .size(11)
                .color(theme::text_secondary())
                .width(Length::Fixed(80.0)),
            text("Kind")
                .size(11)
                .color(theme::text_secondary())
                .width(Length::Fixed(70.0)),
            container(column![]).width(Length::Fixed(180.0)),
        ]
        .spacing(6)
        .padding([0, 16])
        .align_y(Vertical::Center),
    )
    .height(Length::Fixed(28.0))
    .width(Length::Fill)
    .style(theme::container_with_bg(theme::header_bg()));

    let mut list = column![].spacing(0);

    if folders.is_empty() && files.is_empty() {
        let empty_state: Element<Message> = container(
            column![
                text("🔒").size(40),
                text("Vault is empty")
                    .size(15)
                    .color(theme::text_secondary()),
                text("Click \"+ Encrypt\" to add files")
                    .size(13)
                    .color(theme::text_dim()),
            ]
            .spacing(8)
            .align_x(Horizontal::Center)
            .width(Length::Fill),
        )
        .height(Length::Fixed(200.0))
        .width(Length::Fill)
        .align_y(Vertical::Center)
        .into();
        list = list.push(empty_state);
    }

    if !folders.is_empty() {
        list = list.push(
            container(
                text("FOLDERS")
                    .size(10)
                    .color(theme::section_header()),
            )
            .padding([0, 16])
            .align_y(Vertical::Center)
            .height(Length::Fixed(24.0))
            .width(Length::Fill)
            .style(theme::container_with_bg(theme::folder_band_bg())),
        );
        for f in &folders {
            let selected = app.selected_id == f.id;
            list = list.push(folder_row(f, selected));
        }
    }

    if !files.is_empty() {
        list = list.push(
            container(
                text("ENCRYPTED FILES")
                    .size(10)
                    .color(theme::section_header()),
            )
            .padding([0, 16])
            .align_y(Vertical::Center)
            .height(Length::Fixed(24.0))
            .width(Length::Fill)
            .style(theme::container_with_bg(theme::folder_band_bg())),
        );
        for f in &files {
            let selected = app.selected_id == f.id;
            list = list.push(file_row(f, selected));
        }
    }

    let content = container(
        column![header, divider(), scrollable(list).height(Length::Fill)]
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::container_with_bg(theme::content_bg()));

    // ── Path bar ─────────────────────────────────────────────────────
    let path_bar = container(
        row![text("🔒").size(11), text(current_path).size(11).color(theme::text_secondary())]
            .spacing(6)
            .padding([0, 16])
            .align_y(Vertical::Center),
    )
    .height(Length::Fixed(26.0))
    .width(Length::Fill)
    .style(theme::container_with_bg(theme::path_bar_bg()));

    column![
        toolbar,
        divider(),
        row![sidebar, vdivider(), content]
            .width(Length::Fill)
            .height(Length::Fill),
        divider(),
        path_bar,
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
