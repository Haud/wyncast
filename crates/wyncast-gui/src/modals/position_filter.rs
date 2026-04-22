use iced::{Element, Length, Padding};
use twui::{
    ButtonSize, ButtonStyle, ButtonVariant, Modal, ModalHeader,
    TextColor, TextSize, TextStyle, TextWeight,
    BoxStyle, StackGap, StackStyle,
    button, frame, text, v_stack,
};
use wyncast_baseball::draft::pick::Position;

/// Ordered list of positions shown in the filter modal.
///
/// `None` maps to "All Positions" (clear filter).
pub const POSITION_OPTIONS: &[Option<Position>] = &[
    None,
    Some(Position::Catcher),
    Some(Position::FirstBase),
    Some(Position::SecondBase),
    Some(Position::ThirdBase),
    Some(Position::ShortStop),
    Some(Position::LeftField),
    Some(Position::CenterField),
    Some(Position::RightField),
    Some(Position::Outfield),
    Some(Position::StartingPitcher),
    Some(Position::ReliefPitcher),
    Some(Position::DesignatedHitter),
];

/// Display label for a position option.
pub fn position_label(pos: Option<Position>) -> &'static str {
    match pos {
        None => "All Positions",
        Some(p) => p.display_str(),
    }
}

/// Renders a position filter modal overlay.
///
/// Returns `None` when `is_open` is false (no overlay rendered).
pub fn position_filter_modal<'a, Message: Clone + 'a>(
    is_open: bool,
    on_dismiss: Message,
    on_select: impl Fn(Option<Position>) -> Message + 'a,
    current_filter: Option<Position>,
) -> Option<Element<'a, Message>> {
    let buttons: Vec<Element<'a, Message>> = POSITION_OPTIONS
        .iter()
        .map(|&pos| {
            let label = position_label(pos);
            let is_selected = pos == current_filter;
            let variant = if is_selected {
                ButtonVariant::Filled
            } else {
                ButtonVariant::Ghost
            };
            let label_elem: Element<'a, Message> = text(
                label,
                TextStyle {
                    size: TextSize::Sm,
                    weight: TextWeight::Semibold,
                    ..Default::default()
                },
            )
            .into();
            let msg = on_select(pos);
            let btn: Element<'a, Message> = button(
                label_elem,
                msg,
                ButtonStyle {
                    variant,
                    size: ButtonSize::Sm,
                    ..Default::default()
                },
            )
            .into();
            btn
        })
        .collect();

    let header: Element<'a, Message> = ModalHeader::view("Filter by Position", on_dismiss.clone())
        .into();

    let divider: Element<'a, Message> = frame(
        iced::widget::Space::new().width(Length::Fill),
        BoxStyle {
            width: Length::Fill,
            height: Length::Fixed(1.0),
            background: Some(twui::Colors::BorderDefault),
            ..Default::default()
        },
    )
    .into();

    let button_list: Element<'a, Message> = v_stack(
        buttons,
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            padding: Padding::new(8.0),
            ..Default::default()
        },
    )
    .into();

    let help: Element<'a, Message> = text(
        "Press Esc to close",
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let modal_content: Element<'a, Message> = v_stack(
        vec![header, divider, button_list, help],
        StackStyle {
            gap: StackGap::None,
            width: Length::Fixed(240.0),
            ..Default::default()
        },
    )
    .into();

    Modal::new().view(is_open, on_dismiss, modal_content)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_label_none_is_all() {
        assert_eq!(position_label(None), "All Positions");
    }

    #[test]
    fn position_label_catcher() {
        assert_eq!(position_label(Some(Position::Catcher)), "C");
    }

    #[test]
    fn position_options_starts_with_none() {
        assert!(POSITION_OPTIONS[0].is_none());
    }

    #[test]
    fn position_filter_modal_returns_none_when_closed() {
        let result: Option<Element<'_, String>> = position_filter_modal(
            false,
            "dismiss".to_string(),
            |_| "select".to_string(),
            None,
        );
        assert!(result.is_none());
    }

    #[test]
    fn position_filter_modal_returns_some_when_open() {
        let result: Option<Element<'_, String>> = position_filter_modal(
            true,
            "dismiss".to_string(),
            |_| "select".to_string(),
            None,
        );
        assert!(result.is_some());
    }
}
