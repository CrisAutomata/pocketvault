use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Shadow};

pub fn bg() -> Color {
    Color::from_rgb8(0x1c, 0x1c, 0x1e)
}
pub fn sidebar_bg() -> Color {
    Color::from_rgb8(0x24, 0x24, 0x26)
}
pub fn content_bg() -> Color {
    bg()
}
pub fn header_bg() -> Color {
    bg()
}
pub fn selected() -> Color {
    Color::from_rgb8(0x20, 0x60, 0xc8)
}
pub fn hover() -> Color {
    Color::from_rgb8(0x2a, 0x2a, 0x2e)
}
pub fn divider() -> Color {
    Color::from_rgb8(0x38, 0x38, 0x3a)
}
pub fn text() -> Color {
    Color::from_rgb8(0xe8, 0xe8, 0xea)
}
pub fn text_secondary() -> Color {
    Color::from_rgb8(0x8e, 0x8e, 0x93)
}
pub fn text_dim() -> Color {
    Color::from_rgb8(0x5a, 0x5a, 0x5e)
}
pub fn folder_icon() -> Color {
    Color::from_rgb8(0xf0, 0xa8, 0x30)
}
pub fn badge_bg() -> Color {
    Color::from_rgb8(0x3a, 0x3a, 0x3e)
}
pub fn badge_text() -> Color {
    text_secondary()
}
pub fn section_header() -> Color {
    Color::from_rgb8(0x5a, 0x5a, 0x5e)
}
pub fn input_bg() -> Color {
    Color::from_rgb8(0x2a, 0x2a, 0x2e)
}
pub fn button_primary() -> Color {
    Color::from_rgb8(0x20, 0x60, 0xc8)
}
pub fn button_primary_hover() -> Color {
    Color::from_rgb8(0x28, 0x70, 0xd8)
}
pub fn danger() -> Color {
    Color::from_rgb8(0xc8, 0x40, 0x40)
}
pub fn danger_hover() -> Color {
    Color::from_rgb8(0xd8, 0x50, 0x50)
}
pub fn path_bar_bg() -> Color {
    Color::from_rgb8(0x14, 0x14, 0x16)
}
pub fn error_text() -> Color {
    Color::from_rgb8(0xe0, 0x50, 0x50)
}
pub fn scrim() -> Color {
    Color::from_rgba8(0x00, 0x00, 0x00, 0.53)
}
pub fn folder_band_bg() -> Color {
    Color::from_rgb8(0x21, 0x21, 0x24)
}
pub fn preview_backdrop() -> Color {
    Color::from_rgb8(0x0a, 0x0a, 0x0c)
}

fn no_border() -> Border {
    Border {
        color: Color::TRANSPARENT,
        width: 0.0,
        radius: 0.0.into(),
    }
}

fn no_shadow() -> Shadow {
    Shadow::default()
}

/// Solid-fill button (Encrypt, Preview, dialog submit, Create/Rename confirm).
pub fn primary_button(status: button::Status) -> button::Style {
    let bg_color = match status {
        button::Status::Hovered => button_primary_hover(),
        button::Status::Disabled => divider(),
        _ => button_primary(),
    };
    button::Style {
        background: Some(Background::Color(bg_color)),
        text_color: text(),
        border: Border {
            radius: 6.0.into(),
            ..no_border()
        },
        shadow: no_shadow(),
        snap: false,
    }
}

/// Neutral/secondary button (Cancel, Export, New Folder, Rename).
pub fn secondary_button(status: button::Status) -> button::Style {
    let (bg_color, text_color) = match status {
        button::Status::Hovered => (hover(), text()),
        button::Status::Disabled => (badge_bg(), text_dim()),
        _ => (badge_bg(), text()),
    };
    button::Style {
        background: Some(Background::Color(bg_color)),
        text_color,
        border: Border {
            radius: 6.0.into(),
            ..no_border()
        },
        shadow: no_shadow(),
        snap: false,
    }
}

/// Outlined danger button (Delete) — fills solid on hover.
pub fn danger_button(status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(danger_hover())),
            text_color: text(),
            border: Border {
                radius: 6.0.into(),
                ..no_border()
            },
            shadow: no_shadow(),
            snap: false,
        },
        button::Status::Disabled => button::Style {
            background: None,
            text_color: text_dim(),
            border: Border {
                color: divider(),
                width: 1.0,
                radius: 6.0.into(),
            },
            shadow: no_shadow(),
            snap: false,
        },
        _ => button::Style {
            background: None,
            text_color: danger(),
            border: Border {
                color: danger(),
                width: 1.0,
                radius: 6.0.into(),
            },
            shadow: no_shadow(),
            snap: false,
        },
    }
}

/// A row (sidebar item, folder/file row) — highlighted when selected, else hover tint.
pub fn row_button(selected_row: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let bg_color = if selected_row {
            Some(selected())
        } else {
            match status {
                button::Status::Hovered => Some(hover()),
                _ => None,
            }
        };
        button::Style {
            background: bg_color.map(Background::Color),
            text_color: text(),
            border: no_border(),
            shadow: no_shadow(),
            snap: false,
        }
    }
}

/// Small neutral toggle button (folder-tree expand/collapse chevron).
pub fn chevron_button(status: button::Status) -> button::Style {
    let bg_color = match status {
        button::Status::Hovered => Some(hover()),
        _ => None,
    };
    button::Style {
        background: bg_color.map(Background::Color),
        text_color: text_secondary(),
        border: Border {
            radius: 4.0.into(),
            ..no_border()
        },
        shadow: no_shadow(),
        snap: false,
    }
}

/// Small icon-only button (row delete "✕", modal close "✕").
pub fn icon_button(status: button::Status) -> button::Style {
    let bg_color = match status {
        button::Status::Hovered => Some(danger()),
        _ => None,
    };
    button::Style {
        background: bg_color.map(Background::Color),
        text_color: text_dim(),
        border: Border {
            radius: 4.0.into(),
            ..no_border()
        },
        shadow: no_shadow(),
        snap: false,
    }
}

pub fn plain_container(_theme: &iced::Theme) -> container::Style {
    container::Style {
        text_color: Some(text()),
        background: Some(Background::Color(bg())),
        border: no_border(),
        shadow: no_shadow(),
        snap: false,
    }
}

pub fn container_with_bg(color: Color) -> impl Fn(&iced::Theme) -> container::Style {
    move |_theme| container::Style {
        text_color: Some(text()),
        background: Some(Background::Color(color)),
        border: no_border(),
        shadow: no_shadow(),
        snap: false,
    }
}

/// The rounded "Card" panel used for the password form and modals.
pub fn card_container(_theme: &iced::Theme) -> container::Style {
    container::Style {
        text_color: Some(text()),
        background: Some(Background::Color(sidebar_bg())),
        border: Border {
            color: divider(),
            width: 1.0,
            radius: 12.0.into(),
        },
        shadow: no_shadow(),
        snap: false,
    }
}

pub fn scrim_container(_theme: &iced::Theme) -> container::Style {
    container::Style {
        text_color: None,
        background: Some(Background::Color(scrim())),
        border: no_border(),
        shadow: no_shadow(),
        snap: false,
    }
}

pub fn text_field(_theme: &iced::Theme, _status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(input_bg()),
        border: Border {
            color: divider(),
            width: 1.0,
            radius: 6.0.into(),
        },
        icon: text_secondary(),
        placeholder: text_dim(),
        value: text(),
        selection: selected(),
    }
}
