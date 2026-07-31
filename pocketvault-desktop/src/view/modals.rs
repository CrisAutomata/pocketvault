use iced::alignment::{Horizontal, Vertical};
use iced::widget::{button, column, container, row, stack, text, text_input};
use iced::{Element, Length};

use crate::message::Message;
use crate::state::{Modal, VaultJob};
use crate::theme;

pub fn wrap<'a>(
    base: Element<'a, Message>,
    modal: &'a Option<Modal>,
    active_job: &'a Option<VaultJob>,
) -> Element<'a, Message> {
    let card = match modal {
        None => return base,
        Some(Modal::NewFolder { name }) => new_folder_card(name),
        Some(Modal::Rename { text: rename_text, .. }) => rename_card(rename_text),
        Some(Modal::DeleteConfirm { file_id, folder_id }) => {
            delete_confirm_card(file_id.is_some(), folder_id.is_some())
        }
        Some(Modal::ConfirmCancelJob) => {
            let label = active_job.as_ref().map(|j| j.label.as_str()).unwrap_or("this job");
            confirm_cancel_job_card(label)
        }
    };

    let overlay = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(theme::scrim_container);

    stack![base, overlay].into()
}

fn modal_shell<'a>(height: f32, content: iced::widget::Column<'a, Message>) -> Element<'a, Message> {
    container(content.spacing(12).padding(20))
        .width(Length::Fixed(360.0))
        .height(Length::Fixed(height))
        .style(theme::card_container)
        .into()
}

fn modal_buttons<'a>(
    cancel_label: &str,
    confirm_label: &str,
    confirm_msg: Message,
    danger: bool,
) -> Element<'a, Message> {
    row![
        button(text(cancel_label.to_string()).size(13))
            .padding([8, 16])
            .style(|_theme, status| theme::secondary_button(status))
            .on_press(Message::CancelModal),
        button(text(confirm_label.to_string()).size(13))
            .padding([8, 16])
            .style(move |_theme, status| if danger {
                theme::danger_button(status)
            } else {
                theme::primary_button(status)
            })
            .on_press(confirm_msg),
    ]
    .spacing(8)
    .align_y(Vertical::Center)
    .into()
}

fn new_folder_card<'a>(name: &str) -> Element<'a, Message> {
    modal_shell(
        160.0,
        column![
            text("New Folder").size(15).color(theme::text()),
            text_input("", name)
                .size(14)
                .padding([9, 12])
                .style(theme::text_field)
                .on_input(Message::NewFolderNameChanged)
                .on_submit(Message::ConfirmNewFolder),
            container(modal_buttons(
                "Cancel",
                "Create",
                Message::ConfirmNewFolder,
                false,
            ))
            .width(Length::Fill)
            .align_x(Horizontal::Right),
        ],
    )
}

fn rename_card<'a>(text_value: &str) -> Element<'a, Message> {
    modal_shell(
        160.0,
        column![
            text("Rename Folder").size(15).color(theme::text()),
            text_input("", text_value)
                .size(14)
                .padding([9, 12])
                .style(theme::text_field)
                .on_input(Message::RenameTextChanged)
                .on_submit(Message::ConfirmRename),
            container(modal_buttons(
                "Cancel",
                "Rename",
                Message::ConfirmRename,
                false,
            ))
            .width(Length::Fill)
            .align_x(Horizontal::Right),
        ],
    )
}

fn delete_confirm_card<'a>(is_file: bool, is_folder: bool) -> Element<'a, Message> {
    let title = if is_folder {
        "Delete folder?"
    } else if is_file {
        "Delete encrypted file?"
    } else {
        "Delete?"
    };
    let body = if is_folder {
        "This permanently deletes the folder and all its files. This cannot be undone."
    } else {
        "This permanently removes the file from the vault. This cannot be undone."
    };

    modal_shell(
        170.0,
        column![
            text(title).size(15).color(theme::text()),
            text(body).size(13).color(theme::text_secondary()),
            container(modal_buttons(
                "Cancel",
                "Delete",
                Message::ConfirmDelete,
                true,
            ))
            .width(Length::Fill)
            .align_x(Horizontal::Right),
        ],
    )
}

fn confirm_cancel_job_card<'a>(label: &str) -> Element<'a, Message> {
    modal_shell(
        170.0,
        column![
            text("Stop this job?").size(15).color(theme::text()),
            text(format!(
                "Stop encrypting {label}? Anything already encrypted in this job will be removed."
            ))
            .size(13)
            .color(theme::text_secondary()),
            container(modal_buttons(
                "Keep Going",
                "Stop",
                Message::ConfirmCancelJob,
                true,
            ))
            .width(Length::Fill)
            .align_x(Horizontal::Right),
        ],
    )
}
