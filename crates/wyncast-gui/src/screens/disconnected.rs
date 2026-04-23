use iced::alignment;
use iced::{Element, Length};
use twui::{
    BoxStyle, ButtonStyle, ButtonVariant, Colors, StackAlign, StackGap, StackStyle, TextColor,
    TextSize, TextStyle, TextWeight, button, frame, section_box, spinner, text, v_stack,
    SpinnerSize, SpinnerStyle,
};

use crate::screens::draft::DraftMessage;

/// Centered "Waiting for ESPN draft extension…" card shown when disconnected.
pub fn view<'a>() -> Element<'a, DraftMessage> {
    let spinner_elem: Element<DraftMessage> =
        spinner(SpinnerStyle::new().size(SpinnerSize::Xl2)).into();

    let heading: Element<DraftMessage> = text(
        "Waiting for ESPN draft extension…",
        TextStyle {
            size: TextSize::Xl3,
            weight: TextWeight::Bold,
            align: twui::TextAlign::Center,
            ..Default::default()
        },
    )
    .into();

    let subtext: Element<DraftMessage> = text(
        "Install the Wyncast browser extension to connect to your ESPN draft.",
        TextStyle {
            color: TextColor::Dimmed,
            align: twui::TextAlign::Center,
            ..Default::default()
        },
    )
    .into();

    let retry_btn: Element<DraftMessage> = button(
        text("Retry", TextStyle::default()),
        DraftMessage::RetryConnection,
        ButtonStyle::new().variant(ButtonVariant::Secondary),
    )
    .into();

    let continue_btn: Element<DraftMessage> = button(
        text(
            "Continue without extension",
            TextStyle {
                color: TextColor::Dimmed,
                ..Default::default()
            },
        ),
        DraftMessage::RetryConnection, // placeholder target — button is disabled
        ButtonStyle::new()
            .variant(ButtonVariant::Ghost)
            .disabled(true),
    )
    .into();

    let card_content: Element<DraftMessage> = v_stack(
        vec![spinner_elem, heading, subtext, retry_btn, continue_btn],
        StackStyle {
            gap: StackGap::Lg,
            align: StackAlign::Center,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let card: Element<DraftMessage> = section_box(card_content).into();

    let card_constrained: Element<DraftMessage> = frame(
        card,
        BoxStyle {
            width: Length::Fixed(520.0),
            ..Default::default()
        },
    )
    .into();

    frame(
        card_constrained,
        BoxStyle {
            width: Length::Fill,
            height: Length::Fill,
            background: Some(Colors::BgApp),
            align_x: alignment::Horizontal::Center,
            align_y: alignment::Vertical::Center,
            ..Default::default()
        },
    )
    .into()
}
