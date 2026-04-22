mod help_bar;
mod layout;
mod nomination_banner;
mod status_bar;
pub mod tabs;

use iced::{Element, Length, Padding, Task};
use twui::{
    Colors, ConfirmationModal, TextColor, TextSize, TextStyle,
    frame, text, v_stack,
    BoxStyle, StackGap, StackStyle,
};
use wyncast_app::protocol::{
    ConnectionStatus, LlmStreamUpdate, ScrollDirection, TabId, UserCommand,
};

use crate::focus::FocusTarget;
use crate::widgets::{focus_ring, with_overlay};
use tabs::analysis::{AnalysisMessage, AnalysisPanel};

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
    ScrollRequested(ScrollDirection),
    QuitRequested,
    QuitConfirmed,
    QuitCancelled,
    Analysis(AnalysisMessage),
    LlmUpdate { request_id: u64, update: LlmStreamUpdate },
    Nominated { analysis_request_id: Option<u64> },
    NominationCleared,
}

#[derive(Debug, Clone)]
pub enum DraftEffect {
    SendCommand(UserCommand),
    CycleFocus(Direction),
    Exit,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct DraftScreen {
    active_tab: TabId,
    quit_modal_open: bool,
    analysis: AnalysisPanel,
}

impl DraftScreen {
    pub fn new() -> Self {
        Self {
            active_tab: TabId::Analysis,
            quit_modal_open: false,
            analysis: AnalysisPanel::new(),
        }
    }

    pub fn has_modal(&self) -> bool {
        self.quit_modal_open
    }

    pub fn update(&mut self, msg: DraftMessage) -> (Task<DraftMessage>, Vec<DraftEffect>) {
        match msg {
            DraftMessage::TabSelected(tab) => {
                self.active_tab = tab;
                (
                    Task::none(),
                    vec![DraftEffect::SendCommand(UserCommand::SwitchTab(tab))],
                )
            }
            DraftMessage::FocusCycle(dir) => {
                (Task::none(), vec![DraftEffect::CycleFocus(dir)])
            }
            DraftMessage::ScrollRequested(dir) => {
                if self.active_tab == TabId::Analysis {
                    let task = self
                        .analysis
                        .update(AnalysisMessage::ScrollBy(dir))
                        .map(DraftMessage::Analysis);
                    (task, vec![])
                } else {
                    (Task::none(), vec![])
                }
            }
            DraftMessage::QuitRequested => {
                self.quit_modal_open = true;
                (Task::none(), vec![])
            }
            DraftMessage::QuitConfirmed => (
                Task::none(),
                vec![
                    DraftEffect::SendCommand(UserCommand::Quit),
                    DraftEffect::Exit,
                ],
            ),
            DraftMessage::QuitCancelled => {
                self.quit_modal_open = false;
                (Task::none(), vec![])
            }
            DraftMessage::Analysis(msg) => {
                let task = self.analysis.update(msg).map(DraftMessage::Analysis);
                (task, vec![])
            }
            DraftMessage::LlmUpdate { request_id, update } => {
                let task = self
                    .analysis
                    .update(AnalysisMessage::LlmUpdate { request_id, update })
                    .map(DraftMessage::Analysis);
                (task, vec![])
            }
            DraftMessage::Nominated { analysis_request_id } => {
                let task = self
                    .analysis
                    .update(AnalysisMessage::Nominated { analysis_request_id })
                    .map(DraftMessage::Analysis);
                (task, vec![])
            }
            DraftMessage::NominationCleared => {
                let task = self
                    .analysis
                    .update(AnalysisMessage::NominationCleared)
                    .map(DraftMessage::Analysis);
                (task, vec![])
            }
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
    let tab_content = tab_content(screen);
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

fn tab_content<'a>(screen: &'a DraftScreen) -> Element<'a, DraftMessage> {
    match screen.active_tab {
        TabId::Analysis => screen.analysis.view(),
        TabId::Available => tab_stub("Available Players — stub (Phase 3.3)"),
        TabId::DraftLog => tab_stub("Draft Log — stub (Phase 3.4)"),
        TabId::Teams => tab_stub("Teams — stub (Phase 3.4)"),
    }
}

fn tab_stub<'a>(label: &'static str) -> Element<'a, DraftMessage> {
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
    use iced::alignment;

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wyncast_app::protocol::ScrollDirection;

    #[test]
    fn update_tab_selected_changes_tab_and_emits_command() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::TabSelected(TabId::DraftLog));
        assert_eq!(screen.active_tab, TabId::DraftLog);
        assert!(matches!(
            effects.as_slice(),
            [DraftEffect::SendCommand(UserCommand::SwitchTab(TabId::DraftLog))]
        ));
    }

    #[test]
    fn update_quit_requested_opens_modal() {
        let mut screen = DraftScreen::new();
        assert!(!screen.quit_modal_open);
        let (_, effects) = screen.update(DraftMessage::QuitRequested);
        assert!(screen.quit_modal_open);
        assert!(effects.is_empty());
    }

    #[test]
    fn update_quit_cancelled_closes_modal() {
        let mut screen = DraftScreen::new();
        screen.quit_modal_open = true;
        let (_, effects) = screen.update(DraftMessage::QuitCancelled);
        assert!(!screen.quit_modal_open);
        assert!(effects.is_empty());
    }

    #[test]
    fn update_quit_confirmed_emits_exit_and_command() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::QuitConfirmed);
        assert_eq!(effects.len(), 2);
        assert!(matches!(effects[0], DraftEffect::SendCommand(UserCommand::Quit)));
        assert!(matches!(effects[1], DraftEffect::Exit));
    }

    #[test]
    fn update_focus_cycle_emits_effect() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::FocusCycle(Direction::Forward));
        assert!(matches!(
            effects.as_slice(),
            [DraftEffect::CycleFocus(Direction::Forward)]
        ));
    }

    #[test]
    fn update_scroll_on_non_analysis_tab_is_noop() {
        let mut screen = DraftScreen::new();
        screen.active_tab = TabId::DraftLog;
        let (_, effects) = screen.update(DraftMessage::ScrollRequested(ScrollDirection::Down));
        assert!(effects.is_empty());
    }

    #[test]
    fn update_nomination_cleared_does_not_panic() {
        let mut screen = DraftScreen::new();
        let _ = screen.update(DraftMessage::Nominated { analysis_request_id: Some(1) });
        let (_, effects) = screen.update(DraftMessage::NominationCleared);
        assert!(effects.is_empty());
    }

    #[test]
    fn update_llm_update_does_not_panic() {
        let mut screen = DraftScreen::new();
        let _ = screen.update(DraftMessage::Nominated { analysis_request_id: Some(42) });
        let (_, effects) = screen.update(DraftMessage::LlmUpdate {
            request_id: 42,
            update: LlmStreamUpdate::Token("hello".to_owned()),
        });
        assert!(effects.is_empty());
    }

    #[test]
    fn update_nominated_does_not_panic() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::Nominated { analysis_request_id: Some(7) });
        assert!(effects.is_empty());
    }
}
