use iced::alignment::Vertical;
use iced::widget::{button, column, container, image, row, scrollable, text};
use iced::{window, Element, Length};

use crate::message::Message;
use crate::state::PreviewData;
use crate::theme;

pub fn view(id: window::Id, data: &PreviewData) -> Element<'_, Message> {
    let header = container(
        row![
            text(data.file_name.clone())
                .size(14)
                .color(theme::text())
                .width(Length::Fill),
            button(text("✕").size(12))
                .padding(6)
                .style(|_theme, status| theme::icon_button(status))
                .on_press(Message::PreviewWindowClosed(id)),
        ]
        .spacing(8)
        .padding([0, 16])
        .align_y(Vertical::Center),
    )
    .height(Length::Fixed(44.0))
    .width(Length::Fill)
    .style(theme::container_with_bg(theme::sidebar_bg()));

    let body: Element<Message> = if data.is_image {
        if let Some((w, h, rgba)) = &data.image_rgba {
            container(
                image(image::Handle::from_rgba(*w, *h, rgba.clone()))
                    .content_fit(iced::ContentFit::Contain)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::container_with_bg(theme::preview_backdrop()))
            .into()
        } else {
            container(text("Unable to decode image.").color(theme::text()))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    } else {
        scrollable(
            container(text(data.text.clone()).size(13).color(theme::text()))
                .padding(20)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    };

    column![header, body]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
