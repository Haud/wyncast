use iced::{Element, Length, Padding};
use twui::{
    Colors, TextColor, TextSize, TextStyle, TextWeight,
    h_stack, text,
    StackAlign, StackGap, StackStyle,
    frame, BoxStyle,
};
use wyncast_app::protocol::ConnectionStatus;

pub fn view<'a, Message: Clone + 'a>(connection_status: ConnectionStatus) -> Element<'a, Message> {
    let (dot_char, dot_color, status_text) = match connection_status {
        ConnectionStatus::Connected => ("●", TextColor::Yellow, "Connected"),
        ConnectionStatus::Disconnected => ("○", TextColor::Error, "Waiting for extension"),
    };

    let dot: Element<Message> = text(
        dot_char,
        TextStyle {
            color: dot_color,
            size: TextSize::Sm,
            ..Default::default()
        },
    )
    .into();

    let status: Element<Message> = text(
        status_text,
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let title: Element<Message> = text(
        "Wyncast Draft",
        TextStyle {
            size: TextSize::Sm,
            weight: TextWeight::Semibold,
            ..Default::default()
        },
    )
    .into();

    let spacer: Element<Message> = iced::widget::Space::new().width(Length::Fill).into();

    let row: Element<Message> = h_stack(
        vec![dot, status, spacer, title],
        StackStyle {
            gap: StackGap::Sm,
            align: StackAlign::Center,
            width: Length::Fill,
            padding: Padding::new(4.0).left(8.0).right(8.0),
            ..Default::default()
        },
    )
    .into();

    frame(
        row,
        BoxStyle {
            width: Length::Fill,
            background: Some(Colors::Slate800),
            ..Default::default()
        },
    )
    .into()
}
