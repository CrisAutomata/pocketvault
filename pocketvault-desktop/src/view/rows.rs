use iced::alignment::Vertical;
use iced::widget::{button, container, row, text};
use iced::{Alignment, Element, Length};

use crate::message::Message;
use crate::state::{FileItem, FolderItem, FolderTreeRow};
use crate::theme;

pub fn badge<'a>(count: i64) -> Element<'a, Message> {
    container(text(count.to_string()).size(11).color(theme::badge_text()))
        .padding([2, 8])
        .style(theme::container_with_bg(theme::badge_bg()))
        .into()
}

#[allow(clippy::too_many_arguments)]
pub fn sidebar_item<'a>(
    label: String,
    badge_count: Option<i64>,
    selected: bool,
    is_lock: bool,
    can_delete: bool,
    on_click: Message,
    on_delete: Option<Message>,
) -> Element<'a, Message> {
    let icon = if is_lock {
        text("🔒").size(13)
    } else {
        text("📁").size(13).color(theme::folder_icon())
    };
    let label_color = if is_lock {
        theme::danger()
    } else {
        theme::text()
    };

    let mut content = row![
        icon,
        text(label).size(13).color(label_color).width(Length::Fill)
    ]
    .spacing(6)
    .align_y(Vertical::Center);

    if let Some(count) = badge_count {
        content = content.push(badge(count));
    }

    if can_delete {
        if let Some(msg) = on_delete {
            content = content.push(
                button(text("✕").size(10))
                    .padding(3)
                    .style(|_theme, status| theme::icon_button(status))
                    .on_press(msg),
            );
        }
    }

    button(content)
        .width(Length::Fill)
        .padding([6, 8])
        .style(move |_theme, status| theme::row_button(selected)(_theme, status))
        .on_press(on_click)
        .into()
}

/// A row in the sidebar's expandable folder tree (VS Code Explorer style):
/// indented by depth, with a chevron toggle only on rows that have children.
pub fn folder_tree_row<'a>(
    item: &FolderTreeRow,
    selected: bool,
    expanded: bool,
) -> Element<'a, Message> {
    let folder_id = item.id.clone();
    let folder_id_for_toggle = item.id.clone();
    let name = item.name.clone();
    let count = item.item_count as i64;
    let indent = item.depth as f32 * 16.0;

    let mut content = row![iced::widget::Space::new().width(Length::Fixed(indent))]
        .spacing(6)
        .align_y(Vertical::Center);

    content = content.push(if item.has_children {
        let chevron = if expanded { "▾" } else { "▸" };
        button(text(chevron).size(22).color(theme::text_secondary()))
            .padding(2)
            .style(|_theme, status| theme::chevron_button(status))
            .on_press(Message::ToggleFolderExpanded(folder_id_for_toggle))
            .into()
    } else {
        Element::from(iced::widget::Space::new().width(Length::Fixed(26.0)))
    });

    content = content
        .push(text("📁").size(13).color(theme::folder_icon()))
        .push(text(name).size(13).color(theme::text()).width(Length::Fill))
        .push(badge(count));

    button(content)
        .width(Length::Fill)
        .padding([6, 8])
        .style(move |_theme, status| theme::row_button(selected)(_theme, status))
        .on_press(Message::NavigateFolder(Some(folder_id)))
        .into()
}

pub fn folder_row<'a>(item: &FolderItem, selected: bool) -> Element<'a, Message> {
    let name = item.name.clone();
    let folder_id = item.id.clone();
    let folder_id_for_export = item.id.clone();
    let folder_id_for_delete = item.id.clone();
    let folder_id_for_rename = item.id.clone();
    let folder_name_for_rename = item.name.clone();
    let count = item.item_count;

    let content = row![
        container(text("📁").size(13).color(theme::folder_icon())).width(Length::Fixed(22.0)),
        text(name).size(13).width(Length::Fill),
        text("—")
            .size(13)
            .color(theme::text_secondary())
            .width(Length::Fixed(120.0)),
        text(format!("{count} item{}", if count == 1 { "" } else { "s" }))
            .size(13)
            .color(theme::text_secondary())
            .width(Length::Fixed(80.0)),
        text("Folder")
            .size(13)
            .color(theme::text_secondary())
            .width(Length::Fixed(70.0)),
        row![
            button(text("Export").size(11))
                .padding([4, 8])
                .style(|_theme, status| theme::secondary_button(status))
                .on_press(Message::ExportFolder(folder_id_for_export)),
            button(text("Rename").size(11))
                .padding([4, 8])
                .style(|_theme, status| theme::secondary_button(status))
                .on_press(Message::OpenRenameDialog(
                    folder_id_for_rename,
                    folder_name_for_rename
                )),
            button(text("Delete").size(11))
                .padding([4, 8])
                .style(|_theme, status| theme::danger_button(status))
                .on_press(Message::RequestDeleteFolder(folder_id_for_delete)),
        ]
        .spacing(4)
        .width(Length::Fixed(180.0)),
    ]
    .spacing(6)
    .align_y(Vertical::Center)
    .padding([0, 16]);

    button(content)
        .width(Length::Fill)
        .height(Length::Fixed(36.0))
        .padding(0)
        .style(move |_theme, status| theme::row_button(selected)(_theme, status))
        .on_press(Message::NavigateFolder(Some(folder_id)))
        .into()
}

pub fn file_row<'a>(item: &FileItem, selected: bool) -> Element<'a, Message> {
    let display_name = item.display_name.clone();
    let modified = item.modified_str.clone();
    let size = item.size_str.clone();
    let kind = item.kind.clone();
    let file_id = item.id.clone();
    let file_id_for_preview = item.id.clone();
    let file_id_for_export = item.id.clone();
    let file_id_for_delete = item.id.clone();
    let can_preview = item.can_preview;

    let mut actions = row![].spacing(4).width(Length::Fixed(180.0));

    if can_preview {
        actions = actions.push(
            button(text("Preview").size(11))
                .padding([4, 8])
                .style(|_theme, status| theme::primary_button(status))
                .on_press(Message::PreviewFile(file_id_for_preview)),
        );
    }

    actions = actions.push(
        button(text("Export").size(11))
            .padding([4, 8])
            .style(|_theme, status| theme::secondary_button(status))
            .on_press(Message::ExportFile(file_id_for_export)),
    );

    actions = actions.push(
        button(text("Delete").size(11))
            .padding([4, 8])
            .style(|_theme, status| theme::danger_button(status))
            .on_press(Message::RequestDeleteFile(file_id_for_delete)),
    );

    let content = row![
        container(text("🔐").size(14)).width(Length::Fixed(22.0)),
        text(display_name).size(13).width(Length::Fill),
        text(modified)
            .size(12)
            .color(theme::text_secondary())
            .width(Length::Fixed(120.0)),
        text(size)
            .size(12)
            .color(theme::text_secondary())
            .width(Length::Fixed(80.0)),
        text(kind)
            .size(12)
            .color(theme::text_secondary())
            .width(Length::Fixed(70.0)),
        actions,
    ]
    .spacing(6)
    .align_y(Vertical::Center)
    .padding([0, 16]);

    button(content)
        .width(Length::Fill)
        .height(Length::Fixed(36.0))
        .padding(0)
        .style(move |_theme, status| theme::row_button(selected)(_theme, status))
        .on_press(Message::SelectRow(file_id))
        .into()
}

pub fn section_label<'a>(label: &str) -> Element<'a, Message> {
    container(
        text(label.to_string())
            .size(10)
            .color(theme::section_header()),
    )
    .padding([4, 8])
    .align_y(Alignment::Center)
    .into()
}
