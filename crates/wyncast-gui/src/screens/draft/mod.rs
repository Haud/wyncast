mod budget_bar;
mod help_bar;
mod layout;
mod nomination_banner;
mod status_bar;
pub mod sidebar;
pub mod tabs;

use iced::widget::pane_grid;
use iced::{Element, Length, Padding, Task};
use twui::{Colors, StackGap, StackStyle, v_stack};
use wyncast_app::protocol::{
    AppSnapshot, ConnectionStatus, LlmStreamUpdate, NominationInfo, ScrollDirection, TabId,
    UserCommand,
};
use wyncast_baseball::draft::roster::RosterSlot;
use wyncast_baseball::valuation::scarcity::ScarcityEntry;

use crate::focus::FocusTarget;
use crate::modals::{ModalKind, ModalStack};
use crate::modals::position_filter::position_filter_modal;
use crate::modals::quit_confirm::quit_confirm_modal;
use crate::widgets::{SplitPaneState, focus_ring, with_overlay};
use sidebar::{Sidebar, SidebarMessage};
use sidebar::nomination_plan::PlanMessage;
use tabs::analysis::{AnalysisMessage, AnalysisPanel};
use tabs::available::{AvailableMessage, AvailablePanel};
use tabs::draft_log::{DraftLogMessage, DraftLogPanel};
use tabs::teams::{TeamsMessage, TeamsPanel};

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
    PaneResized(pane_grid::ResizeEvent),
    Analysis(AnalysisMessage),
    Available(AvailableMessage),
    DraftLog(DraftLogMessage),
    Teams(TeamsMessage),
    Sidebar(SidebarMessage),
    LlmUpdate { request_id: u64, update: LlmStreamUpdate },
    /// New nomination arrived — carries full info for banner display.
    Nominated { analysis_request_id: Option<u64>, info: Box<NominationInfo> },
    /// Bid updated on the active nomination (same player, new bid/bidder).
    BidUpdated(Box<NominationInfo>),
    NominationCleared,
    PlanStarted { request_id: u64 },
    StateSnapshot(Box<AppSnapshot>),
    /// Retry button pressed while disconnected — sends RequestKeyframe to backend.
    RetryConnection,
}

#[derive(Debug, Clone)]
pub enum DraftEffect {
    SendCommand(UserCommand),
    CycleFocus(Direction),
    Exit,
    /// Main/sidebar divider was dragged to a new ratio.  App handles persistence.
    PaneResized(pane_grid::ResizeEvent),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct DraftScreen {
    active_tab: TabId,
    /// Active modal stack — replaces per-component modal flags.
    pub modal_stack: ModalStack,
    analysis: AnalysisPanel,
    pub available: AvailablePanel,
    pub draft_log: DraftLogPanel,
    pub teams: TeamsPanel,
    sidebar: Sidebar,
    pub my_roster: Vec<RosterSlot>,
    pub positional_scarcity: Vec<ScarcityEntry>,
    /// Active nomination — drives the nomination banner.
    pub current_nomination: Option<NominationInfo>,
    /// Position string from the active nomination (e.g. "1B", "SP").
    nominated_position: Option<String>,
    /// Request ID for the active plan stream, used to route LlmUpdates.
    plan_request_id: Option<u64>,
    // Budget fields from the most recent StateSnapshot.
    pub budget_spent: u32,
    pub budget_remaining: u32,
    pub salary_cap: u32,
    pub inflation_rate: f64,
    pub max_bid: u32,
    pub avg_per_slot: f64,
}

impl DraftScreen {
    pub fn new() -> Self {
        Self {
            active_tab: TabId::Analysis,
            modal_stack: ModalStack::new(),
            analysis: AnalysisPanel::new(),
            available: AvailablePanel::new(),
            draft_log: DraftLogPanel::new(),
            teams: TeamsPanel::new(),
            sidebar: Sidebar::new(),
            my_roster: Vec::new(),
            positional_scarcity: Vec::new(),
            current_nomination: None,
            nominated_position: None,
            plan_request_id: None,
            budget_spent: 0,
            budget_remaining: 260,
            salary_cap: 260,
            inflation_rate: 1.0,
            max_bid: 260,
            avg_per_slot: 0.0,
        }
    }

    pub fn active_tab(&self) -> TabId {
        self.active_tab
    }

    /// Returns true when any modal is open (used to gate global key handling).
    pub fn has_modal(&self) -> bool {
        !self.modal_stack.is_empty()
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
            DraftMessage::ScrollRequested(dir) => match self.active_tab {
                TabId::Analysis => {
                    let task = self
                        .analysis
                        .update(AnalysisMessage::ScrollBy(dir))
                        .map(DraftMessage::Analysis);
                    (task, vec![])
                }
                TabId::Available => {
                    let task = self
                        .available
                        .update(AvailableMessage::ScrollBy(dir))
                        .map(DraftMessage::Available);
                    (task, vec![])
                }
                TabId::DraftLog => {
                    let task = self
                        .draft_log
                        .update(DraftLogMessage::ScrollBy(dir))
                        .map(DraftMessage::DraftLog);
                    (task, vec![])
                }
                TabId::Teams => {
                    let task = self
                        .teams
                        .update(TeamsMessage::ScrollBy(dir))
                        .map(DraftMessage::Teams);
                    (task, vec![])
                }
            },
            DraftMessage::QuitRequested => {
                self.modal_stack.push(ModalKind::QuitConfirm);
                (Task::none(), vec![])
            }
            DraftMessage::QuitConfirmed => {
                self.modal_stack.pop();
                (
                    Task::none(),
                    vec![
                        DraftEffect::SendCommand(UserCommand::Quit),
                        DraftEffect::Exit,
                    ],
                )
            }
            DraftMessage::QuitCancelled => {
                self.modal_stack.pop();
                (Task::none(), vec![])
            }
            DraftMessage::Analysis(msg) => {
                let task = self.analysis.update(msg).map(DraftMessage::Analysis);
                (task, vec![])
            }
            DraftMessage::Available(msg) => {
                // Intercept position-filter messages to drive the modal stack.
                match &msg {
                    AvailableMessage::PositionFilterOpened => {
                        self.modal_stack.push(ModalKind::PositionFilter);
                    }
                    AvailableMessage::PositionFilterClosed
                    | AvailableMessage::PositionSelected(_) => {
                        self.modal_stack.pop();
                    }
                    _ => {}
                }
                let task = self.available.update(msg).map(DraftMessage::Available);
                (task, vec![])
            }
            DraftMessage::DraftLog(msg) => {
                let task = self.draft_log.update(msg).map(DraftMessage::DraftLog);
                (task, vec![])
            }
            DraftMessage::Teams(msg) => {
                let task = self.teams.update(msg).map(DraftMessage::Teams);
                (task, vec![])
            }
            DraftMessage::Sidebar(msg) => {
                let task = self.sidebar.update(msg).map(DraftMessage::Sidebar);
                (task, vec![])
            }
            DraftMessage::LlmUpdate { request_id, update } => {
                // Route to analysis panel (keyed on its request ID).
                let task1 = self
                    .analysis
                    .update(AnalysisMessage::LlmUpdate { request_id, update: update.clone() })
                    .map(DraftMessage::Analysis);

                // Also route to plan panel (keyed on its request ID).
                let task2 = self
                    .sidebar
                    .update(SidebarMessage::Plan(PlanMessage::LlmUpdate { request_id, update }))
                    .map(DraftMessage::Sidebar);

                (Task::batch([task1, task2]), vec![])
            }
            DraftMessage::Nominated { analysis_request_id, info } => {
                self.nominated_position = Some(info.position.clone());
                let player_name = info.player_name.clone();
                self.current_nomination = Some(*info);
                let task1 = self
                    .analysis
                    .update(AnalysisMessage::Nominated { analysis_request_id })
                    .map(DraftMessage::Analysis);
                let task2 = self
                    .available
                    .update(AvailableMessage::NominationActive(player_name))
                    .map(DraftMessage::Available);
                (Task::batch([task1, task2]), vec![])
            }
            DraftMessage::BidUpdated(info) => {
                self.current_nomination = Some(*info);
                (Task::none(), vec![])
            }
            DraftMessage::NominationCleared => {
                self.current_nomination = None;
                self.nominated_position = None;
                let task1 = self
                    .analysis
                    .update(AnalysisMessage::NominationCleared)
                    .map(DraftMessage::Analysis);
                let task2 = self
                    .available
                    .update(AvailableMessage::NominationCleared)
                    .map(DraftMessage::Available);
                let task3 = self
                    .sidebar
                    .update(SidebarMessage::Plan(PlanMessage::NominationCleared))
                    .map(DraftMessage::Sidebar);
                (Task::batch([task1, task2, task3]), vec![])
            }
            DraftMessage::PlanStarted { request_id } => {
                self.plan_request_id = Some(request_id);
                let task = self
                    .sidebar
                    .update(SidebarMessage::Plan(PlanMessage::PlanStarted { request_id }))
                    .map(DraftMessage::Sidebar);
                (task, vec![])
            }
            DraftMessage::PaneResized(event) => {
                (Task::none(), vec![DraftEffect::PaneResized(event)])
            }
            DraftMessage::RetryConnection => {
                (
                    Task::none(),
                    vec![DraftEffect::SendCommand(UserCommand::RequestKeyframe)],
                )
            }
            DraftMessage::StateSnapshot(snapshot) => {
                self.available.available_players = snapshot.available_players.clone();
                self.draft_log.draft_log = snapshot.draft_log;
                self.draft_log.available_players = snapshot.available_players;
                self.teams.team_snapshots = snapshot.team_snapshots;
                self.teams.salary_cap = snapshot.salary_cap;
                self.my_roster = snapshot.my_roster;
                self.positional_scarcity = snapshot.positional_scarcity;
                self.budget_spent = snapshot.budget_spent;
                self.budget_remaining = snapshot.budget_remaining;
                self.salary_cap = snapshot.salary_cap;
                self.inflation_rate = snapshot.inflation_rate;
                self.max_bid = snapshot.max_bid;
                self.avg_per_slot = snapshot.avg_per_slot;
                (Task::none(), vec![])
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
    pane_state: &'a SplitPaneState,
) -> Element<'a, DraftMessage> {
    let content = view_content(screen, focus, pane_state);
    let modal = view_modal(screen);
    with_overlay(content, modal)
}

fn view_content<'a>(
    screen: &'a DraftScreen,
    focus: FocusTarget,
    pane_state: &'a SplitPaneState,
) -> Element<'a, DraftMessage> {
    let status_bar = status_bar::view(ConnectionStatus::Connected);
    let help_bar = help_bar::view();

    let nomination_banner = nomination_banner::view(
        screen.current_nomination.as_ref(),
        screen.inflation_rate,
        &screen.available.available_players,
    );
    let main = main_panel(screen, focus);
    let sidebar = sidebar(screen, focus);

    layout::draft_layout(
        status_bar,
        nomination_banner,
        main,
        sidebar,
        help_bar,
        pane_state,
        DraftMessage::PaneResized,
    )
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
        TabId::Available => screen.available.view().map(DraftMessage::Available),
        TabId::DraftLog => screen.draft_log.view().map(DraftMessage::DraftLog),
        TabId::Teams => screen.teams.view().map(DraftMessage::Teams),
    }
}

fn sidebar<'a>(screen: &'a DraftScreen, focus: FocusTarget) -> Element<'a, DraftMessage> {
    let budget = budget_bar_elem(screen, focus);

    let three_panels = screen
        .sidebar
        .view(
            focus,
            &screen.my_roster,
            &screen.positional_scarcity,
            screen.nominated_position.as_deref(),
        )
        .map(DraftMessage::Sidebar);

    v_stack(
        vec![budget, three_panels],
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

fn budget_bar_elem<'a>(screen: &DraftScreen, focus: FocusTarget) -> Element<'a, DraftMessage> {
    budget_bar::view(
        screen.budget_spent,
        screen.budget_remaining,
        screen.salary_cap,
        screen.inflation_rate,
        screen.max_bid,
        screen.avg_per_slot,
        focus == FocusTarget::Budget,
    )
}

fn view_modal<'a>(screen: &'a DraftScreen) -> Option<Element<'a, DraftMessage>> {
    match screen.modal_stack.top() {
        Some(ModalKind::QuitConfirm) => {
            quit_confirm_modal(DraftMessage::QuitConfirmed, DraftMessage::QuitCancelled)
        }
        Some(ModalKind::PositionFilter) => position_filter_modal(
            true,
            DraftMessage::Available(AvailableMessage::PositionFilterClosed),
            |pos| DraftMessage::Available(AvailableMessage::PositionSelected(pos)),
            screen.available.position_filter,
        ),
        None => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sidebar::roster::RosterMessage;
    use wyncast_app::protocol::{AppMode, ScrollDirection};

    fn empty_snapshot() -> Box<AppSnapshot> {
        Box::new(AppSnapshot {
            app_mode: AppMode::Draft,
            pick_count: 0,
            total_picks: 0,
            active_tab: None,
            available_players: vec![],
            positional_scarcity: vec![],
            draft_log: vec![],
            my_roster: vec![],
            budget_spent: 0,
            budget_remaining: 260,
            salary_cap: 260,
            inflation_rate: 1.0,
            max_bid: 260,
            avg_per_slot: 0.0,
            hitting_spent: 0,
            hitting_target: 182,
            pitching_spent: 0,
            pitching_target: 78,
            team_snapshots: vec![],
            llm_configured: false,
        })
    }

    #[test]
    fn tab_selected_changes_tab_and_emits_command() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::TabSelected(TabId::DraftLog));
        assert_eq!(screen.active_tab, TabId::DraftLog);
        assert!(matches!(
            effects.as_slice(),
            [DraftEffect::SendCommand(UserCommand::SwitchTab(TabId::DraftLog))]
        ));
    }

    #[test]
    fn quit_requested_opens_modal() {
        let mut screen = DraftScreen::new();
        assert!(!screen.has_modal());
        let (_, effects) = screen.update(DraftMessage::QuitRequested);
        assert_eq!(screen.modal_stack.top(), Some(&ModalKind::QuitConfirm));
        assert!(effects.is_empty());
    }

    #[test]
    fn quit_cancelled_closes_modal() {
        let mut screen = DraftScreen::new();
        screen.modal_stack.push(ModalKind::QuitConfirm);
        let (_, effects) = screen.update(DraftMessage::QuitCancelled);
        assert!(screen.modal_stack.is_empty());
        assert!(effects.is_empty());
    }

    #[test]
    fn quit_confirmed_emits_exit_and_command() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::QuitConfirmed);
        assert_eq!(effects.len(), 2);
        assert!(matches!(effects[0], DraftEffect::SendCommand(UserCommand::Quit)));
        assert!(matches!(effects[1], DraftEffect::Exit));
    }

    #[test]
    fn focus_cycle_emits_effect() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::FocusCycle(Direction::Forward));
        assert!(matches!(effects.as_slice(), [DraftEffect::CycleFocus(Direction::Forward)]));
    }

    #[test]
    fn scroll_on_non_scrollable_tab_produces_task() {
        let mut screen = DraftScreen::new();
        screen.active_tab = TabId::DraftLog;
        let (_, effects) = screen.update(DraftMessage::ScrollRequested(ScrollDirection::Down));
        assert!(effects.is_empty());
    }

    #[test]
    fn nominated_sets_position_and_nomination() {
        let mut screen = DraftScreen::new();
        let info = Box::new(NominationInfo {
            player_name: "Test Player".to_string(),
            position: "1B".to_string(),
            nominated_by: "Some Team".to_string(),
            current_bid: 1,
            current_bidder: None,
            time_remaining: None,
            eligible_slots: vec![],
        });
        let (_, effects) = screen.update(DraftMessage::Nominated {
            analysis_request_id: Some(1),
            info,
        });
        assert_eq!(screen.nominated_position, Some("1B".to_string()));
        assert!(screen.current_nomination.is_some());
        assert_eq!(screen.current_nomination.as_ref().unwrap().player_name, "Test Player");
        assert!(effects.is_empty());
    }

    #[test]
    fn bid_updated_updates_nomination() {
        let mut screen = DraftScreen::new();
        let info = Box::new(NominationInfo {
            player_name: "Test Player".to_string(),
            position: "1B".to_string(),
            nominated_by: "Some Team".to_string(),
            current_bid: 50,
            current_bidder: Some("Team B".to_string()),
            time_remaining: None,
            eligible_slots: vec![],
        });
        let (_, effects) = screen.update(DraftMessage::BidUpdated(info));
        assert_eq!(screen.current_nomination.as_ref().unwrap().current_bid, 50);
        assert!(effects.is_empty());
    }

    #[test]
    fn nomination_cleared_clears_position_and_nomination() {
        let mut screen = DraftScreen::new();
        screen.nominated_position = Some("SP".to_string());
        screen.current_nomination = Some(NominationInfo {
            player_name: "P".to_string(),
            position: "SP".to_string(),
            nominated_by: "T".to_string(),
            current_bid: 1,
            current_bidder: None,
            time_remaining: None,
            eligible_slots: vec![],
        });
        let (_, effects) = screen.update(DraftMessage::NominationCleared);
        assert!(screen.nominated_position.is_none());
        assert!(screen.current_nomination.is_none());
        assert!(effects.is_empty());
    }

    #[test]
    fn plan_started_sets_request_id() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::PlanStarted { request_id: 42 });
        assert_eq!(screen.plan_request_id, Some(42));
        assert!(effects.is_empty());
    }

    #[test]
    fn state_snapshot_updates_roster_and_scarcity() {
        let mut screen = DraftScreen::new();
        assert!(screen.my_roster.is_empty());
        assert!(screen.positional_scarcity.is_empty());
        let (_, effects) = screen.update(DraftMessage::StateSnapshot(empty_snapshot()));
        assert!(effects.is_empty());
        // Still empty because the snapshot has empty vecs; just verify no panic.
        assert!(screen.my_roster.is_empty());
    }

    #[test]
    fn state_snapshot_captures_budget_fields() {
        let mut screen = DraftScreen::new();
        let mut snap = *empty_snapshot();
        snap.budget_spent = 100;
        snap.budget_remaining = 160;
        snap.salary_cap = 260;
        snap.inflation_rate = 1.12;
        snap.max_bid = 155;
        snap.avg_per_slot = 8.5;
        let _ = screen.update(DraftMessage::StateSnapshot(Box::new(snap)));
        assert_eq!(screen.budget_spent, 100);
        assert_eq!(screen.budget_remaining, 160);
        assert_eq!(screen.salary_cap, 260);
        assert!((screen.inflation_rate - 1.12).abs() < 1e-10);
        assert_eq!(screen.max_bid, 155);
        assert!((screen.avg_per_slot - 8.5).abs() < 1e-10);
    }

    #[test]
    fn has_modal_false_initially() {
        assert!(!DraftScreen::new().has_modal());
    }

    #[test]
    fn has_modal_true_when_quit_open() {
        let mut screen = DraftScreen::new();
        screen.modal_stack.push(ModalKind::QuitConfirm);
        assert!(screen.has_modal());
    }

    #[test]
    fn position_filter_opened_pushes_stack() {
        let mut screen = DraftScreen::new();
        screen.active_tab = TabId::Available;
        let (_, effects) = screen.update(DraftMessage::Available(AvailableMessage::PositionFilterOpened));
        assert_eq!(screen.modal_stack.top(), Some(&ModalKind::PositionFilter));
        assert!(effects.is_empty());
    }

    #[test]
    fn position_filter_closed_pops_stack() {
        let mut screen = DraftScreen::new();
        screen.modal_stack.push(ModalKind::PositionFilter);
        let (_, effects) = screen.update(DraftMessage::Available(AvailableMessage::PositionFilterClosed));
        assert!(screen.modal_stack.is_empty());
        assert!(effects.is_empty());
    }

    #[test]
    fn position_selected_pops_stack_and_sets_filter() {
        use wyncast_baseball::draft::pick::Position;
        let mut screen = DraftScreen::new();
        screen.modal_stack.push(ModalKind::PositionFilter);
        let (_, effects) = screen.update(DraftMessage::Available(AvailableMessage::PositionSelected(Some(Position::Catcher))));
        assert!(screen.modal_stack.is_empty());
        assert_eq!(screen.available.position_filter, Some(Position::Catcher));
        assert!(effects.is_empty());
    }

    #[test]
    fn llm_update_does_not_panic() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::LlmUpdate {
            request_id: 1,
            update: LlmStreamUpdate::Token("tok".to_string()),
        });
        assert!(effects.is_empty());
    }

    #[test]
    fn sidebar_scroll_message_does_not_panic() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::Sidebar(
            SidebarMessage::Roster(RosterMessage::ScrollBy(ScrollDirection::Down)),
        ));
        assert!(effects.is_empty());
    }

    #[test]
    fn retry_connection_emits_request_keyframe() {
        let mut screen = DraftScreen::new();
        let (_, effects) = screen.update(DraftMessage::RetryConnection);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            DraftEffect::SendCommand(UserCommand::RequestKeyframe)
        ));
    }
}
