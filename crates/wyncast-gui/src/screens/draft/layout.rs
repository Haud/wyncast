use iced::widget::pane_grid;
use iced::{Element, Length, Padding};
use twui::{Colors, v_stack, BoxStyle, StackGap, StackStyle, frame};

use crate::widgets::{SplitOrientation, SplitPaneState, split_pane};

/// Layout for the disconnected state: status bar + disabled tab bar + centered card + help bar.
pub fn disconnected_layout<'a, Message: Clone + 'a>(
    status_bar: Element<'a, Message>,
    tab_bar: Element<'a, Message>,
    card: Element<'a, Message>,
    help_bar: Element<'a, Message>,
) -> Element<'a, Message> {
    let card_container: Element<Message> = frame(
        card,
        BoxStyle {
            width: Length::Fill,
            height: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    v_stack(
        vec![status_bar, tab_bar, card_container, help_bar],
        StackStyle {
            gap: StackGap::None,
            width: Length::Fill,
            height: Length::Fill,
            padding: Padding::ZERO,
            background: Some(Colors::BgApp),
            ..Default::default()
        },
    )
    .into()
}

/// Composes the full draft screen layout from its chrome and content pieces.
///
/// `pane_state` holds the draggable split ratio for the main/sidebar divider.
/// `on_resize` is called when the user drags the divider.
pub fn draft_layout<'a, Message: Clone + 'a>(
    status_bar: Element<'a, Message>,
    nomination_banner: Element<'a, Message>,
    main_panel: Element<'a, Message>,
    sidebar: Element<'a, Message>,
    help_bar: Element<'a, Message>,
    pane_state: &'a SplitPaneState,
    on_resize: impl Fn(pane_grid::ResizeEvent) -> Message + 'a,
) -> Element<'a, Message> {
    let split = split_pane(
        pane_state,
        main_panel,
        sidebar,
        SplitOrientation::Horizontal,
        on_resize,
    );

    let split_container: Element<Message> = frame(
        split,
        BoxStyle {
            width: Length::Fill,
            height: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    v_stack(
        vec![status_bar, nomination_banner, split_container, help_bar],
        StackStyle {
            gap: StackGap::None,
            width: Length::Fill,
            height: Length::Fill,
            padding: Padding::ZERO,
            background: Some(Colors::BgApp),
            ..Default::default()
        },
    )
    .into()
}
