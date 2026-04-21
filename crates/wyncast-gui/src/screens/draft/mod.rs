mod help_bar;
mod layout;
mod nomination_banner;
mod status_bar;
mod tabs;

use iced::alignment;
use iced::{Element, Length, Padding};
use twui::{
    Colors, ConfirmationModal, TextColor, TextSize, TextStyle,
    frame, text, v_stack,
    BoxStyle, StackGap, StackStyle,
};
use wyncast_app::protocol::{ConnectionStatus, ScrollDirection, TabId};

use crate::focus::FocusTarget;
use crate::widgets::{focus_ring, with_overlay};

// ---------------------------------------------------------------------------
// Messages & types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Direction {
    Forward,
    Reverse,
}

#[derive(Debug, Clone)]
pub enum DraftMessage {
    TabSelected(TabId),
    FocusCycle(Direction),
    #[allow(dead_code)]
    ScrollRequested(ScrollDirection),
    QuitRequested,
    QuitConfirmed,
    QuitCancelled,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct DraftScreen {
    pub active_tab: TabId,
    pub quit_modal_open: bool,
}

impl DraftScreen {
    pub fn new() -> Self {
        Self {
            active_tab: TabId::Analysis,
            quit_modal_open: false,
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view<'a>(
    screen: &'a DraftScreen,
    focus: FocusTarget,
    connection_status: ConnectionStatus,
) -> Element<'a, DraftMessage> {
    let content = view_content(screen, focus, connection_status);
    let modal = quit_modal(screen);
    with_overlay(content, modal)
}

fn view_content<'a>(
    screen: &'a DraftScreen,
    focus: FocusTarget,
    connection_status: ConnectionStatus,
) -> Element<'a, DraftMessage> {
    let status_bar = status_bar::view(connection_status);
    let nomination_banner = nomination_banner::view();
    let main_panel = main_panel(screen, focus);
    let sidebar = sidebar(focus);
    let help_bar = help_bar::view();

    layout::draft_layout(status_bar, nomination_banner, main_panel, sidebar, help_bar)
}

fn main_panel<'a>(screen: &'a DraftScreen, focus: FocusTarget) -> Element<'a, DraftMessage> {
    let tab_bar_elem = tabs::view(screen.active_tab);
    let tab_content = tab_content_stub(screen.active_tab);
    let tab_content_with_ring = focus_ring(tab_content, focus == FocusTarget::MainPanel);

    v_stack(
        vec![tab_bar_elem, tab_content_with_ring],
        StackStyle {
            gap: StackGap::None,
            width: Length::Fill,
            height: Length::Fill,
            ..Default::default()
        },
    )
    .into()
}

fn tab_content_stub<'a>(active_tab: TabId) -> Element<'a, DraftMessage> {
    let label = match active_tab {
        TabId::Analysis => "Analysis — stub (Phase 3.2)",
        TabId::Available => "Available Players — stub (Phase 3.3)",
        TabId::DraftLog => "Draft Log — stub (Phase 3.4)",
        TabId::Teams => "Teams — stub (Phase 3.4)",
    };

    let placeholder: Element<DraftMessage> = text(
        label,
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
            height: Length::Fill,
            padding: Padding::new(12.0),
            ..Default::default()
        },
    )
    .into()
}

fn sidebar<'a>(focus: FocusTarget) -> Element<'a, DraftMessage> {
    let budget = sidebar_panel_stub("Budget", focus == FocusTarget::Budget);
    let roster = sidebar_panel_stub("Roster", focus == FocusTarget::Roster);
    let scarcity = sidebar_panel_stub("Scarcity", focus == FocusTarget::Scarcity);
    let plan = sidebar_panel_stub("Nomination Plan", focus == FocusTarget::NominationPlan);

    v_stack(
        vec![budget, roster, scarcity, plan],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            height: Length::Fill,
            padding: Padding::new(4.0),
            background: Some(Colors::BgSidebar),
            ..Default::default()
        },
    )
    .into()
}

fn sidebar_panel_stub<'a>(label: &'static str, focused: bool) -> Element<'a, DraftMessage> {
    let label_elem: Element<DraftMessage> = text(
        label,
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let content: Element<DraftMessage> = frame(
        label_elem,
        BoxStyle {
            width: Length::Fill,
            height: Length::FillPortion(1),
            padding: Padding::new(8.0),
            background: Some(Colors::BgElevated),
            align_x: alignment::Horizontal::Left,
            align_y: alignment::Vertical::Top,
            ..Default::default()
        },
    )
    .into();

    focus_ring(content, focused)
}

fn quit_modal<'a>(screen: &DraftScreen) -> Option<Element<'a, DraftMessage>> {
    ConfirmationModal::view(
        screen.quit_modal_open,
        "Quit Wyncast?",
        "Are you sure you want to quit?",
        "Quit",
        "Cancel",
        DraftMessage::QuitConfirmed,
        DraftMessage::QuitCancelled,
    )
}
