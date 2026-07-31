use iced::alignment::{Horizontal, Vertical};
use iced::widget::{button, column, container, text, text_input};
use iced::{Element, Length};

use crate::message::Message;
use crate::state::{AuthMode, AuthState};
use crate::theme;

pub fn view(auth: &AuthState) -> Element<'_, Message> {
    let (title, subtitle) = match auth.mode {
        AuthMode::ChangePassword => (
            "Change Master Password",
            "Enter your current password, then choose a new one.",
        ),
        AuthMode::Create => (
            "Create New Vault",
            "Choose a strong master password to protect your files.",
        ),
        AuthMode::Unlock => (
            "Unlock Vault",
            "Enter your master password to unlock the vault.",
        ),
    };

    let mut card = column![]
        .spacing(12)
        .padding(20)
        .width(Length::Fixed(380.0));

    if matches!(auth.mode, AuthMode::ChangePassword) {
        card = card.push(field_label("Current Password"));
        card = card.push(
            text_input("", &auth.old_password)
                .secure(true)
                .size(14)
                .padding([9, 12])
                .style(theme::text_field)
                .on_input(Message::OldPasswordChanged),
        );
    }

    let password_label = if matches!(auth.mode, AuthMode::ChangePassword) {
        "New Password"
    } else {
        "Master Password"
    };
    card = card.push(field_label(password_label));
    card = card.push(
        text_input("", &auth.password)
            .secure(true)
            .size(14)
            .padding([9, 12])
            .style(theme::text_field)
            .on_input(Message::PasswordChanged)
            .on_submit(Message::SubmitAuth),
    );

    if matches!(auth.mode, AuthMode::Create | AuthMode::ChangePassword) {
        card = card.push(field_label("Confirm Password"));
        card = card.push(
            text_input("", &auth.confirm)
                .secure(true)
                .size(14)
                .padding([9, 12])
                .style(theme::text_field)
                .on_input(Message::ConfirmPasswordChanged)
                .on_submit(Message::SubmitAuth),
        );
    }

    if !auth.error.is_empty() {
        card = card.push(text(auth.error.clone()).size(12).color(theme::error_text()));
    }

    let submit_label = if auth.busy {
        "Please wait…"
    } else {
        match auth.mode {
            AuthMode::ChangePassword => "Change Password",
            AuthMode::Create => "Create Vault",
            AuthMode::Unlock => "Unlock",
        }
    };

    let mut submit = button(
        text(submit_label)
            .size(14)
            .width(Length::Fill)
            .align_x(Horizontal::Center),
    )
    .width(Length::Fill)
    .padding(10)
    .style(|_theme, status| theme::primary_button(status));

    if !auth.busy {
        submit = submit.on_press(Message::SubmitAuth);
    }
    card = card.push(submit);

    if matches!(auth.mode, AuthMode::ChangePassword) {
        card = card.push(
            button(
                text("Cancel")
                    .size(13)
                    .width(Length::Fill)
                    .align_x(Horizontal::Center),
            )
            .width(Length::Fill)
            .padding(8)
            .style(|_theme, status| theme::secondary_button(status))
            .on_press(Message::CancelChangePassword),
        );
    }

    let card_container = container(card).style(theme::card_container);

    let content = column![
        text("🔒").size(40),
        text(title).size(22).color(theme::text()),
        text(subtitle)
            .size(13)
            .color(theme::text_secondary())
            .align_x(Horizontal::Center),
        card_container,
    ]
    .spacing(14)
    .align_x(Horizontal::Center)
    .max_width(520.0);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(theme::plain_container)
        .into()
}

fn field_label<'a>(label: &str) -> Element<'a, Message> {
    text(label.to_string())
        .size(12)
        .color(theme::text_secondary())
        .into()
}
