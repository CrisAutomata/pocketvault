use iced::alignment::{Horizontal, Vertical};
use iced::widget::{button, column, container, row, stack, text, text_input};
use iced::{Element, Length};

use crate::message::Message;
use crate::state::{CancelTarget, Modal, PocketVault};
use crate::theme;

pub fn wrap<'a>(base: Element<'a, Message>, app: &'a PocketVault) -> Element<'a, Message> {
    let card = match &app.modal {
        None => return base,
        Some(Modal::NewFolder { name }) => new_folder_card(name),
        Some(Modal::Rename {
            text: rename_text, ..
        }) => rename_card(rename_text),
        Some(Modal::DeleteConfirm { file_id, folder_id }) => {
            delete_confirm_card(file_id.is_some(), folder_id.is_some())
        }
        Some(Modal::ConfirmCancelJob(target)) => {
            let (verb, label) = match target {
                CancelTarget::Encrypt => (
                    "encrypting",
                    app.running_encrypt
                        .as_ref()
                        .map(|j| j.label.as_str())
                        .unwrap_or("this job"),
                ),
                CancelTarget::Export => (
                    "exporting",
                    app.active_export_job
                        .as_ref()
                        .map(|j| j.label.as_str())
                        .unwrap_or("this job"),
                ),
                CancelTarget::Repack => ("reorganizing", "the vault"),
            };
            confirm_cancel_job_card(verb, label, *target)
        }
        Some(Modal::Settings { custom_gib }) => settings_card(app, custom_gib),
    };

    let overlay = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(theme::scrim_container);

    stack![base, overlay].into()
}

fn modal_shell<'a>(
    height: f32,
    content: iced::widget::Column<'a, Message>,
) -> Element<'a, Message> {
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

fn confirm_cancel_job_card<'a>(
    verb: &str,
    label: &str,
    target: CancelTarget,
) -> Element<'a, Message> {
    modal_shell(
        170.0,
        column![
            text("Stop this job?").size(15).color(theme::text()),
            text(format!(
                "Stop {verb} {label}? Anything already done in this job will be removed."
            ))
            .size(13)
            .color(theme::text_secondary()),
            container(modal_buttons(
                "Keep Going",
                "Stop",
                Message::ConfirmCancelJob(target),
                true,
            ))
            .width(Length::Fill)
            .align_x(Horizontal::Right),
        ],
    )
}

/// A segment-size preset button: label plus its estimated segment count for
/// the current vault size, e.g. "5 GiB — ~70 segments".
fn preset_button<'a>(label: &str, vault_size: u64, bytes: u64) -> Element<'a, Message> {
    let estimate = crate::state::estimate_segment_count(vault_size, Some(bytes));
    button(
        text(format!("{label} — {estimate} segments"))
            .size(13)
            .color(theme::text()),
    )
    .width(Length::Fill)
    .padding([8, 12])
    .style(|_theme, status| theme::secondary_button(status))
    .on_press(Message::ConfirmRepack {
        new_target_bytes: Some(bytes),
        reclaim: false,
    })
    .into()
}

/// "Storage → Segment Size": shows the vault's current segment setting and
/// an estimated count, lets the user pick a new size (presets, a custom GiB
/// value, or single-file), or force a reclaim without changing the size.
fn settings_card<'a>(app: &'a PocketVault, custom_gib: &'a str) -> Element<'a, Message> {
    let Some(session) = &app.session else {
        return modal_shell(100.0, column![text("No vault open.").size(13)]);
    };
    let current = session.vault.meta.segment_settings.target_segment_bytes;
    let vault_size = crate::state::vault_total_size(session);

    let custom_apply = custom_gib
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .map(|n| Message::ConfirmRepack {
            new_target_bytes: Some(n * pocketvault_core::GIB),
            reclaim: false,
        });

    modal_shell(
        440.0,
        column![
            text("Storage").size(15).color(theme::text()),
            text(format!("Vault size: {}", crate::state::format_size(vault_size)))
                .size(12)
                .color(theme::text_secondary()),
            text(format!(
                "Current: {} ({} segments)",
                crate::state::describe_segment_size(current),
                crate::state::estimate_segment_count(vault_size, current),
            ))
            .size(12)
            .color(theme::text_secondary()),
            preset_button("1 GiB", vault_size, pocketvault_core::GIB),
            preset_button("5 GiB", vault_size, 5 * pocketvault_core::GIB),
            preset_button("10 GiB", vault_size, 10 * pocketvault_core::GIB),
            preset_button("20 GiB", vault_size, 20 * pocketvault_core::GIB),
            row![
                text_input("Custom GiB", custom_gib)
                    .size(13)
                    .padding([8, 10])
                    .style(theme::text_field)
                    .on_input(Message::CustomSegmentGibChanged),
                button(text("Apply").size(13))
                    .padding([8, 12])
                    .style(|_theme, status| theme::secondary_button(status))
                    .on_press_maybe(custom_apply),
            ]
            .spacing(8)
            .align_y(Vertical::Center),
            button(text("Single File").size(13))
                .width(Length::Fill)
                .padding([8, 12])
                .style(|_theme, status| theme::secondary_button(status))
                .on_press(Message::ConfirmRepack {
                    new_target_bytes: None,
                    reclaim: false,
                }),
            button(text("Reclaim Space (keep current size)").size(13))
                .width(Length::Fill)
                .padding([8, 12])
                .style(|_theme, status| theme::secondary_button(status))
                .on_press(Message::ConfirmRepack {
                    new_target_bytes: current,
                    reclaim: true,
                }),
            container(
                button(text("Close").size(13))
                    .padding([8, 16])
                    .style(|_theme, status| theme::secondary_button(status))
                    .on_press(Message::CancelModal)
            )
            .width(Length::Fill)
            .align_x(Horizontal::Right),
        ],
    )
}
