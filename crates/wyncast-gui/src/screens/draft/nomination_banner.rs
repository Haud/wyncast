use iced::{Element, Length, Padding};
use twui::{Colors, TextColor, TextSize, TextStyle, frame, text, BoxStyle};

/// Stub nomination banner. Phase 3.6 will populate this with live nomination data.
pub fn view<'a, Message: Clone + 'a>() -> Element<'a, Message> {
    let placeholder: Element<Message> = text(
        "— no active nomination —",
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    frame(
        placeholder,
        BoxStyle {
            width: Length::Fill,
            background: Some(Colors::BgElevated),
            padding: Padding::new(8.0).left(12.0).right(12.0),
            ..Default::default()
        },
    )
    .into()
}
