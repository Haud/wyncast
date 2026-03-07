pub mod draft_log;
pub mod main_panel;
pub mod modal;
pub mod sidebar;
pub mod teams;

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;

use crate::draft::pick::{DraftPick, Position};
use crate::draft::roster::RosterSlot;
use crate::protocol::{
    ConnectionStatus, InstantAnalysis, NominationInfo, TabFeature, TabId, UserCommand,
};
use crate::tui::layout::build_layout;
use crate::tui::scroll::ScrollDirection;
use crate::tui::widgets;
use crate::tui::{BudgetStatus, FocusPanel, KeybindHint, TeamSummary};
use crate::valuation::scarcity::ScarcityEntry;
use crate::valuation::zscore::PlayerValuation;

use crate::tui::action::Action;

use draft_log::DraftLogMessage;
use main_panel::analysis::AnalysisPanelMessage;
use main_panel::available::AvailablePanelMessage;
use main_panel::{MainPanel, MainPanelMessage};
use modal::ModalLayer;
use modal::position_filter::{PositionFilterModalAction, PositionFilterModalMessage};
use modal::{ModalLayerAction, ModalLayerMessage};
use sidebar::plan::PlanPanelMessage;
use sidebar::roster::RosterMessage;
use sidebar::scarcity::ScarcityPanelMessage;
use sidebar::{Sidebar, SidebarMessage};
use teams::TeamsMessage;

// ---------------------------------------------------------------------------
// DraftScreen
// ---------------------------------------------------------------------------

/// Top-level component for the draft mode dashboard.
///
/// Composes MainPanel, Sidebar, ModalLayer, and the stateless status bar,
/// nomination banner, and help bar widgets. Owns all draft-related state
/// that was previously scattered across `ViewState`.
pub struct DraftScreen {
    /// Main panel component: owns the four tab panels and active tab state.
    pub main_panel: MainPanel,
    /// Sidebar component: roster, scarcity, plan panels (budget is stateless).
    pub sidebar: Sidebar,
    /// Draft-mode modal overlays (position filter + quit confirmation).
    pub modal_layer: ModalLayer,
    /// Which panel currently has keyboard focus for scroll routing.
    /// `None` means no panel is focused (scroll goes to active tab by default).
    pub focused_panel: Option<FocusPanel>,
    /// WebSocket connection status.
    pub connection_status: ConnectionStatus,
    /// Number of picks completed.
    pub pick_number: usize,
    /// Total picks in the draft.
    pub total_picks: usize,
    /// Current active nomination, if any.
    pub current_nomination: Option<NominationInfo>,
    /// Instant analysis for the current nomination.
    pub instant_analysis: Option<InstantAnalysis>,
    /// User's team budget status.
    pub budget: BudgetStatus,
    /// Current inflation rate.
    pub inflation: f64,
    /// All available (undrafted) players sorted by value.
    pub available_players: Vec<PlayerValuation>,
    /// Chronological list of completed draft picks.
    pub draft_log: Vec<DraftPick>,
    /// Summary of each team's draft state.
    pub team_summaries: Vec<TeamSummary>,
    /// User's roster slots (position + optional player).
    pub my_roster: Vec<RosterSlot>,
    /// Positional scarcity entries.
    pub positional_scarcity: Vec<ScarcityEntry>,
    /// Whether the LLM client is configured (has a valid API key).
    /// Used by the status bar to show a "No LLM configured" hint.
    pub llm_configured: bool,
    /// Per-widget scroll offsets (keyed by widget name).
    pub scroll_offset: HashMap<String, usize>,
}

impl DraftScreen {
    pub fn new() -> Self {
        Self {
            main_panel: MainPanel::new(),
            sidebar: Sidebar::new(),
            modal_layer: ModalLayer::new(),
            focused_panel: None,
            connection_status: ConnectionStatus::Disconnected,
            pick_number: 0,
            total_picks: 0,
            current_nomination: None,
            instant_analysis: None,
            budget: BudgetStatus::default(),
            inflation: 1.0,
            available_players: Vec::new(),
            draft_log: Vec::new(),
            team_summaries: Vec::new(),
            my_roster: Vec::new(),
            positional_scarcity: Vec::new(),
            llm_configured: true,
            scroll_offset: HashMap::new(),
        }
    }

    /// Handle a key event in draft mode.
    ///
    /// Returns `Some(UserCommand)` when the key press should be forwarded
    /// to the app orchestrator. Returns `None` when the key press was
    /// handled locally by mutating state.
    pub fn handle_key(&mut self, key_event: KeyEvent) -> Option<UserCommand> {
        // Quit confirmation mode: delegate to ConfirmDialog component
        if self.modal_layer.quit_confirm.open {
            use crate::tui::confirm_dialog::ConfirmResult;
            if let Some(msg) = self.modal_layer.quit_confirm.key_to_message(key_event) {
                if let Some(result) = self.modal_layer.quit_confirm.update(msg) {
                    match result {
                        ConfirmResult::Confirmed('n') => return None, // 'n' cancels
                        ConfirmResult::Confirmed(_) => return Some(UserCommand::Quit),
                        ConfirmResult::Cancelled => return None,
                    }
                }
            }
            return None; // block all other keys
        }

        // Filter mode: route keys through the available panel component
        if self.main_panel.available.filter_mode() {
            if let Some(msg) = self.main_panel.available.key_to_message(key_event) {
                self.main_panel.available.update(msg);
            }
            return None;
        }

        // Position filter modal: intercept all keys when the modal is open
        if self.modal_layer.position_filter.open {
            if let Some(msg) = self.modal_layer.position_filter.key_to_message(key_event) {
                if let Some(action) = self.modal_layer.position_filter.update(msg) {
                    if let PositionFilterModalAction::Selected(pos) = action {
                        self.main_panel
                            .available
                            .update(AvailablePanelMessage::SetPositionFilter(pos));
                    }
                }
            }
            return None;
        }

        // Normal mode key dispatch
        match key_event.code {
            // Tab switching
            KeyCode::Char('1') => {
                self.main_panel
                    .update(MainPanelMessage::SwitchTab(TabId::Analysis));
                self.focused_panel = None;
                None
            }
            KeyCode::Char('2') => {
                self.main_panel
                    .update(MainPanelMessage::SwitchTab(TabId::Available));
                self.focused_panel = None;
                None
            }
            KeyCode::Char('3') => {
                self.main_panel
                    .update(MainPanelMessage::SwitchTab(TabId::DraftLog));
                self.focused_panel = None;
                None
            }
            KeyCode::Char('4') => {
                self.main_panel
                    .update(MainPanelMessage::SwitchTab(TabId::Teams));
                self.focused_panel = None;
                None
            }

            // Scrolling: routes to focused panel (or main panel if no focus)
            KeyCode::Up | KeyCode::Char('k') => {
                self.dispatch_scroll_up(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.dispatch_scroll_down(1);
                None
            }
            KeyCode::PageUp => {
                self.dispatch_scroll_up(page_size());
                None
            }
            KeyCode::PageDown => {
                self.dispatch_scroll_down(page_size());
                None
            }

            // Panel focus cycling
            KeyCode::Tab => {
                if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                    self.focused_panel = FocusPanel::prev(self.focused_panel);
                } else {
                    self.focused_panel = FocusPanel::next(self.focused_panel);
                }
                None
            }
            KeyCode::BackTab => {
                self.focused_panel = FocusPanel::prev(self.focused_panel);
                None
            }

            // Filter mode entry: only on tabs that support filtering
            KeyCode::Char('/') => {
                if self.main_panel.active_tab().supports(TabFeature::Filter) {
                    self.main_panel
                        .available
                        .update(AvailablePanelMessage::ToggleFilterMode);
                }
                None
            }

            // Escape: clear focus, filter text, and position filter
            KeyCode::Esc => {
                self.focused_panel = None;
                self.main_panel
                    .available
                    .update(AvailablePanelMessage::ClearFilters);
                None
            }

            // Position filter modal: only on tabs that support it
            KeyCode::Char('p') => {
                if self
                    .main_panel
                    .active_tab()
                    .supports(TabFeature::PositionFilter)
                {
                    self.modal_layer.position_filter.update(
                        PositionFilterModalMessage::Open {
                            current_filter: self.main_panel.available.position_filter(),
                        },
                    );
                }
                None
            }

            // Request a full keyframe (FULL_STATE_SYNC) from the extension
            KeyCode::Char('r') => Some(UserCommand::RequestKeyframe),

            // Open settings screen
            KeyCode::Char(',') => Some(UserCommand::OpenSettings),

            // Quit: enter confirmation mode instead of quitting immediately
            KeyCode::Char('q') => {
                use crate::tui::confirm_dialog::ConfirmMessage;
                self.modal_layer.quit_confirm.update(ConfirmMessage::Open);
                None
            }

            _ => None,
        }
    }

    /// Render the full draft dashboard.
    pub fn view(&self, frame: &mut Frame) {
        let layout = build_layout(frame.area());

        widgets::status_bar::render(
            frame,
            layout.status_bar,
            self.connection_status,
            self.pick_number,
            self.total_picks,
            self.main_panel.active_tab(),
            self.llm_configured,
        );
        widgets::nomination_banner::render(
            frame,
            layout.nomination_banner,
            self.current_nomination.as_ref(),
            self.instant_analysis.as_ref(),
        );

        let main_focused = self.focused_panel == Some(FocusPanel::MainPanel);
        let roster_focused = self.focused_panel == Some(FocusPanel::Roster);
        let scarcity_focused = self.focused_panel == Some(FocusPanel::Scarcity);
        let budget_focused = self.focused_panel == Some(FocusPanel::Budget);
        let nom_plan_focused = self.focused_panel == Some(FocusPanel::NominationPlan);

        // Main panel: delegates to active tab
        let nominated_name = self
            .current_nomination
            .as_ref()
            .map(|n| n.player_name.as_str());
        self.main_panel.view(
            frame,
            layout.main_panel,
            &self.available_players,
            nominated_name,
            &self.draft_log,
            &self.team_summaries,
            main_focused,
        );

        // Sidebar: roster, scarcity, budget, nomination plan
        let nominated_position = self
            .current_nomination
            .as_ref()
            .and_then(|n| Position::from_str_pos(&n.position));
        self.sidebar.view(
            frame,
            layout.roster,
            layout.scarcity,
            layout.budget,
            layout.nomination_plan,
            &self.my_roster,
            &self.positional_scarcity,
            nominated_position.as_ref(),
            &self.budget,
            self.scroll_offset.get("budget").copied().unwrap_or(0),
            roster_focused,
            scarcity_focused,
            budget_focused,
            nom_plan_focused,
        );

        // Help bar: dumb renderer of the pre-synced active keybind hints
        // Note: render_help_bar is called by the parent (ViewState) since it
        // needs access to active_keybinds which lives on ViewState.
        // We render the help bar here using a local keybinds computation.
        let keybinds = self.compute_keybinds();
        crate::tui::render_help_bar_from_draft(frame, layout.help_bar, self, &keybinds);

        // Modal overlay layer (position filter + quit confirm)
        self.modal_layer.view(frame, frame.area());
    }

    /// Compute keybind hints for the help bar.
    pub fn compute_keybinds(&self) -> Vec<KeybindHint> {
        // 1. Quit confirmation overlay: all other input is blocked
        if self.modal_layer.quit_confirm.open {
            return vec![
                KeybindHint::new("y/q", "Confirm quit"),
                KeybindHint::new("n/Esc", "Cancel"),
            ];
        }

        // 2. Position filter modal
        if self.modal_layer.position_filter.open {
            return vec![
                KeybindHint::new("\u{2191}\u{2193}", "Navigate"),
                KeybindHint::new("Enter", "Select"),
                KeybindHint::new("Esc", "Cancel"),
            ];
        }

        // 3. Text filter mode (the inline filter input bar)
        if self.main_panel.available.filter_mode() {
            return vec![
                KeybindHint::new("Enter", "Apply"),
                KeybindHint::new("Esc", "Cancel"),
            ];
        }

        // 4. Normal mode: assemble context-sensitive hints
        let mut hints = vec![
            KeybindHint::new("q", "Quit"),
            KeybindHint::new("1-4", "Tabs"),
        ];

        // Tab-specific: filtering and position-filter only on supported tabs
        if self.main_panel.active_tab().supports(TabFeature::Filter) {
            hints.push(KeybindHint::new("/", "Filter"));
            hints.push(KeybindHint::new("p", "Pos"));
        }

        // Focus cycling, resync, and settings are always available in normal mode
        hints.push(KeybindHint::new("Tab", "Focus"));
        hints.push(KeybindHint::new("r", "Resync"));
        hints.push(KeybindHint::new(",", "Settings"));

        // Scroll hint only appears when a panel is focused (scroll is routed there)
        if self.focused_panel.is_some() {
            hints.push(KeybindHint::new("\u{2191}\u{2193}/j/k/PgUp/PgDn", "Scroll"));
        }

        // Active filter reminder: shown as a trailing hint when the Available tab
        // has a non-empty filter so the user knows results are currently filtered.
        if !self.main_panel.available.filter_text().is_empty()
            && self.main_panel.active_tab() == TabId::Available
        {
            hints.push(KeybindHint::new(
                format!(
                    "filter:\"{}\"",
                    self.main_panel.available.filter_text().value()
                ),
                "active",
            ));
        }

        hints
    }

    // -- Private scroll dispatch methods --

    /// Get the widget key for scroll state based on the active tab.
    fn active_widget_key(&self) -> &'static str {
        match self.main_panel.active_tab() {
            TabId::Analysis => "analysis",
            TabId::Available => "available",
            TabId::DraftLog => "draft_log",
            TabId::Teams => "teams",
        }
    }

    /// Return the scroll key for the currently focused panel.
    fn focused_scroll_key(&self) -> &'static str {
        match self.focused_panel {
            Some(FocusPanel::Roster) => "roster",
            Some(FocusPanel::Scarcity) => "scarcity",
            Some(FocusPanel::Budget) => "budget",
            Some(FocusPanel::NominationPlan) => "nom_plan",
            Some(FocusPanel::MainPanel) | None => self.active_widget_key(),
        }
    }

    /// Dispatch a scroll-up event to the appropriate panel based on focus state.
    fn dispatch_scroll_up(&mut self, lines: usize) {
        let key = self.focused_scroll_key();
        if key == "analysis" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageUp
            } else {
                ScrollDirection::Up
            };
            self.main_panel
                .analysis
                .update(AnalysisPanelMessage::Scroll(dir));
            return;
        }
        if key == "draft_log" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageUp
            } else {
                ScrollDirection::Up
            };
            self.main_panel
                .draft_log
                .update(DraftLogMessage::Scroll(dir));
            return;
        }
        if key == "teams" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageUp
            } else {
                ScrollDirection::Up
            };
            self.main_panel.teams.update(TeamsMessage::Scroll(dir));
            return;
        }
        if key == "roster" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageUp
            } else {
                ScrollDirection::Up
            };
            self.sidebar.roster.update(RosterMessage::Scroll(dir));
            return;
        }
        if key == "scarcity" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageUp
            } else {
                ScrollDirection::Up
            };
            self.sidebar
                .scarcity
                .update(ScarcityPanelMessage::Scroll(dir));
            return;
        }
        if key == "nom_plan" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageUp
            } else {
                ScrollDirection::Up
            };
            self.sidebar.plan.update(PlanPanelMessage::Scroll(dir));
            return;
        }
        if key == "available" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageUp
            } else {
                ScrollDirection::Up
            };
            self.main_panel
                .available
                .update(AvailablePanelMessage::Scroll(dir));
            return;
        }
        let offset = self
            .scroll_offset
            .entry(key.to_string())
            .or_insert(0);
        *offset = offset.saturating_sub(lines);
    }

    /// Dispatch a scroll-down event to the appropriate panel based on focus state.
    fn dispatch_scroll_down(&mut self, lines: usize) {
        let key = self.focused_scroll_key();
        if key == "analysis" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageDown
            } else {
                ScrollDirection::Down
            };
            self.main_panel
                .analysis
                .update(AnalysisPanelMessage::Scroll(dir));
            return;
        }
        if key == "draft_log" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageDown
            } else {
                ScrollDirection::Down
            };
            self.main_panel
                .draft_log
                .update(DraftLogMessage::Scroll(dir));
            return;
        }
        if key == "teams" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageDown
            } else {
                ScrollDirection::Down
            };
            self.main_panel.teams.update(TeamsMessage::Scroll(dir));
            return;
        }
        if key == "roster" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageDown
            } else {
                ScrollDirection::Down
            };
            self.sidebar.roster.update(RosterMessage::Scroll(dir));
            return;
        }
        if key == "scarcity" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageDown
            } else {
                ScrollDirection::Down
            };
            self.sidebar
                .scarcity
                .update(ScarcityPanelMessage::Scroll(dir));
            return;
        }
        if key == "nom_plan" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageDown
            } else {
                ScrollDirection::Down
            };
            self.sidebar.plan.update(PlanPanelMessage::Scroll(dir));
            return;
        }
        if key == "available" {
            let dir = if lines >= page_size() {
                ScrollDirection::PageDown
            } else {
                ScrollDirection::Down
            };
            self.main_panel
                .available
                .update(AvailablePanelMessage::Scroll(dir));
            return;
        }
        let offset = self
            .scroll_offset
            .entry(key.to_string())
            .or_insert(0);
        *offset = offset.saturating_add(lines);
    }
}

impl Default for DraftScreen {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// DraftScreenMessage
// ---------------------------------------------------------------------------

/// Messages that can be dispatched to [`DraftScreen`].
///
/// This enum mirrors the match arms in [`DraftScreen::handle_key`] but uses a
/// message-based dispatch instead of direct key events. Both systems coexist —
/// `handle_key` is untouched and remains the primary input path. `update`
/// is the new message-based path that will be used by the subscription system
/// in later phases.
#[derive(Debug, Clone)]
pub enum DraftScreenMessage {
    /// Delegate to the main panel component.
    MainPanel(MainPanelMessage),
    /// Delegate to the sidebar component.
    Sidebar(SidebarMessage),
    /// Delegate to the modal layer component.
    Modal(ModalLayerMessage),
    /// Switch the active tab.
    SwitchTab(TabId),
    /// Cycle focus forward to the next panel.
    FocusNext,
    /// Cycle focus backward to the previous panel.
    FocusPrev,
    /// Scroll the currently focused panel.
    ScrollFocused(ScrollDirection),
    /// Toggle the text filter input on the Available tab (mirrors `/` key).
    ToggleFilter,
    /// Open the position filter modal on the Available tab (mirrors `p` key).
    OpenPositionFilter,
    /// Enter the quit-confirmation dialog.
    RequestQuit,
    /// Request a full keyframe sync from the extension.
    RequestResync,
    /// Open the settings screen.
    OpenSettings,
}

impl DraftScreen {
    /// Process a [`DraftScreenMessage`] and return an optional [`Action`].
    ///
    /// This mirrors the logic in [`DraftScreen::handle_key`] but driven by
    /// message variants instead of raw key events. The existing `handle_key`
    /// method is untouched — both paths coexist.
    pub fn update(&mut self, msg: DraftScreenMessage) -> Option<Action> {
        use crate::tui::confirm_dialog::ConfirmMessage;
        use crate::protocol::TabFeature;

        match msg {
            DraftScreenMessage::MainPanel(m) => {
                self.main_panel.update(m)
            }
            DraftScreenMessage::Sidebar(m) => {
                self.sidebar.update(m)
            }
            DraftScreenMessage::Modal(m) => {
                if let Some(action) = self.modal_layer.update(m) {
                    match action {
                        ModalLayerAction::QuitConfirm(crate::tui::confirm_dialog::ConfirmResult::Confirmed(_)) => {
                            return Some(Action::Command(UserCommand::Quit));
                        }
                        ModalLayerAction::PositionFilter(PositionFilterModalAction::Selected(pos)) => {
                            self.main_panel
                                .available
                                .update(AvailablePanelMessage::SetPositionFilter(pos));
                        }
                        _ => {}
                    }
                }
                None
            }
            DraftScreenMessage::SwitchTab(tab) => {
                self.main_panel.update(MainPanelMessage::SwitchTab(tab));
                self.focused_panel = None;
                None
            }
            DraftScreenMessage::FocusNext => {
                self.focused_panel = FocusPanel::next(self.focused_panel);
                None
            }
            DraftScreenMessage::FocusPrev => {
                self.focused_panel = FocusPanel::prev(self.focused_panel);
                None
            }
            DraftScreenMessage::ScrollFocused(dir) => {
                let lines = match dir {
                    ScrollDirection::PageUp | ScrollDirection::PageDown => page_size(),
                    _ => 1,
                };
                match dir {
                    ScrollDirection::Up | ScrollDirection::PageUp => {
                        self.dispatch_scroll_up(lines);
                    }
                    ScrollDirection::Down | ScrollDirection::PageDown => {
                        self.dispatch_scroll_down(lines);
                    }
                    _ => {}
                }
                None
            }
            DraftScreenMessage::ToggleFilter => {
                if self.main_panel.active_tab().supports(TabFeature::Filter) {
                    self.main_panel
                        .available
                        .update(AvailablePanelMessage::ToggleFilterMode);
                }
                None
            }
            DraftScreenMessage::OpenPositionFilter => {
                if self
                    .main_panel
                    .active_tab()
                    .supports(TabFeature::PositionFilter)
                {
                    self.modal_layer.position_filter.update(
                        PositionFilterModalMessage::Open {
                            current_filter: self.main_panel.available.position_filter(),
                        },
                    );
                }
                None
            }
            DraftScreenMessage::RequestQuit => {
                self.modal_layer.quit_confirm.update(ConfirmMessage::Open);
                None
            }
            DraftScreenMessage::RequestResync => {
                Some(Action::Command(UserCommand::RequestKeyframe))
            }
            DraftScreenMessage::OpenSettings => {
                Some(Action::Command(UserCommand::OpenSettings))
            }
        }
    }
}

/// Page size for PageUp/PageDown scrolling.
fn page_size() -> usize {
    20
}
