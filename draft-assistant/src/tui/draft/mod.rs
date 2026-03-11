pub mod draft_log;
pub mod main_panel;
pub mod modal;
pub mod sidebar;
pub mod teams;

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crossterm::event::KeyCode;
use ratatui::Frame;

use crate::draft::pick::{DraftPick, Position};
use crate::draft::roster::RosterSlot;
use crate::protocol::{
    ConnectionStatus, InstantAnalysis, NominationInfo, TabFeature, TabId, UserCommand,
};
use crate::tui::layout::build_layout;
use crate::tui::scroll::ScrollDirection;
use crate::tui::subscription::{Subscription, SubscriptionId};
use crate::tui::subscription::keybinding::{
    alt, exact, KeyBindingRecipe, KeybindHint as KbHint, KeybindManager, PRIORITY_NORMAL,
};
use crate::tui::widgets;
use crate::tui::{BudgetStatus, FocusPanel, TeamSummary};
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
    /// Active analysis LLM request ID (for routing LlmUpdate events).
    pub analysis_request_id: Option<u64>,
    /// Active plan LLM request ID (for routing LlmUpdate events).
    pub plan_request_id: Option<u64>,
    /// Per-widget scroll offsets (keyed by widget name).
    pub scroll_offset: HashMap<String, usize>,
    /// Stable base ID used to derive state-dependent subscription IDs for
    /// DraftScreen's own keybindings. The actual ID is hashed from this plus
    /// `focused_panel` and `active_tab` so the listener is rebuilt when those
    /// change (updating the binding set).
    sub_id_base: SubscriptionId,
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
            analysis_request_id: None,
            plan_request_id: None,
            scroll_offset: HashMap::new(),
            sub_id_base: SubscriptionId::unique(),
        }
    }

    /// Render the full draft dashboard.
    pub fn view(&self, frame: &mut Frame, keybinds: &[crate::tui::KeybindHint]) {
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

        // Sidebar: roster, scarcity, nomination plan
        let nominated_position = self
            .current_nomination
            .as_ref()
            .and_then(|n| Position::from_str_pos(&n.position));
        self.sidebar.view(
            frame,
            layout.roster,
            layout.scarcity,
            layout.nomination_plan,
            &self.my_roster,
            &self.positional_scarcity,
            nominated_position.as_ref(),
            roster_focused,
            scarcity_focused,
            nom_plan_focused,
        );

        // Budget: bottom of left column
        widgets::budget::render(
            frame,
            layout.budget,
            &self.budget,
            self.scroll_offset.get("budget").copied().unwrap_or(0),
            budget_focused,
        );

        // Help bar: render keybind hints passed in from App (from kb_manager).
        crate::tui::render_help_bar_draft(frame, layout.help_bar, self.main_panel.available.filter_mode(), self.main_panel.available.filter_text(), keybinds);

        // Modal overlay layer (position filter + quit confirm)
        self.modal_layer.view(frame, frame.area());
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

impl DraftScreen {
    /// Declare keybindings for the subscription system.
    ///
    /// Composes:
    /// 1. `modal_layer` — highest precedence (quit confirm + position filter).
    /// 2. `main_panel` — active tab subscription (e.g. Available filter mode).
    /// 3. `sidebar` — child subscriptions (currently none).
    /// 4. DraftScreen's own normal-mode bindings — state-dependent ID so the
    ///    listener is rebuilt when `focused_panel` or `active_tab` changes.
    pub fn subscription(&self, kb: &mut KeybindManager) -> Subscription<DraftScreenMessage> {
        // 1. Modal layer (highest precedence — maps child types to ModalLayerMessage).
        let modal_sub = self
            .modal_layer
            .subscription(kb)
            .map(DraftScreenMessage::Modal);

        // 2. Main panel (active tab).
        let main_sub = self
            .main_panel
            .subscription(kb)
            .map(DraftScreenMessage::MainPanel);

        // 3. Sidebar (no child subscriptions yet).
        let sidebar_sub = self
            .sidebar
            .subscription(kb)
            .map(DraftScreenMessage::Sidebar);

        // 4. DraftScreen's own normal-mode bindings.
        //    State-dependent ID: rebuild listener when focus or tab changes.
        let own_sub = {
            let mut hasher = DefaultHasher::new();
            self.sub_id_base.hash(&mut hasher);
            // Hash the discriminant of focused_panel (as u8 or None=0).
            let fp_disc: u8 = match self.focused_panel {
                None => 0,
                Some(FocusPanel::MainPanel) => 1,
                Some(FocusPanel::Budget) => 2,
                Some(FocusPanel::Roster) => 3,
                Some(FocusPanel::Scarcity) => 4,
                Some(FocusPanel::NominationPlan) => 5,
            };
            fp_disc.hash(&mut hasher);
            // Hash active tab.
            let tab_disc: u8 = match self.main_panel.active_tab() {
                TabId::Analysis => 0,
                TabId::Available => 1,
                TabId::DraftLog => 2,
                TabId::Teams => 3,
            };
            tab_disc.hash(&mut hasher);
            let own_id = SubscriptionId::from_u64(hasher.finish());

            let supports_filter = self.main_panel.active_tab().supports(TabFeature::Filter);
            let supports_pos_filter = self
                .main_panel
                .active_tab()
                .supports(TabFeature::PositionFilter);
            let has_focus = self.focused_panel.is_some();

            let mut recipe = KeyBindingRecipe::<DraftScreenMessage>::new(own_id)
                .priority(PRIORITY_NORMAL)
                // Always-present bindings
                .bind(
                    exact(KeyCode::Char('q')),
                    |_| DraftScreenMessage::RequestQuit,
                    KbHint::new("q", "Quit"),
                )
                .bind(
                    exact(KeyCode::Char('r')),
                    |_| DraftScreenMessage::RequestResync,
                    KbHint::new("r", "Resync"),
                )
                .bind(
                    exact(KeyCode::Char(',')),
                    |_| DraftScreenMessage::OpenSettings,
                    KbHint::new(",", "Settings"),
                )
                .bind(
                    exact(KeyCode::Char('1')),
                    |_| DraftScreenMessage::SwitchTab(TabId::Analysis),
                    KbHint::new("1-4", "Tabs"),
                )
                .bind(
                    exact(KeyCode::Char('2')),
                    |_| DraftScreenMessage::SwitchTab(TabId::Available),
                    None,
                )
                .bind(
                    exact(KeyCode::Char('3')),
                    |_| DraftScreenMessage::SwitchTab(TabId::DraftLog),
                    None,
                )
                .bind(
                    exact(KeyCode::Char('4')),
                    |_| DraftScreenMessage::SwitchTab(TabId::Teams),
                    None,
                )
                .bind(
                    exact(KeyCode::Tab),
                    |_| DraftScreenMessage::FocusNext,
                    KbHint::new("Tab", "Focus"),
                )
                .bind(
                    exact(KeyCode::BackTab),
                    |_| DraftScreenMessage::FocusPrev,
                    None,
                )
                .bind(
                    alt(KeyCode::Tab),
                    |_| DraftScreenMessage::FocusPrev,
                    None,
                );

            // Scroll bindings: only when a panel is focused
            if has_focus {
                recipe = recipe
                    .bind(
                        exact(KeyCode::Up),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::Up),
                        KbHint::new("↑↓/j/k/PgUp/PgDn", "Scroll"),
                    )
                    .bind(
                        exact(KeyCode::Char('k')),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::Up),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Down),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::Down),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Char('j')),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::Down),
                        None,
                    )
                    .bind(
                        exact(KeyCode::PageUp),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::PageUp),
                        None,
                    )
                    .bind(
                        exact(KeyCode::PageDown),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::PageDown),
                        None,
                    );
            } else {
                // No focus: scroll routes to the active tab (still via ScrollFocused)
                recipe = recipe
                    .bind(
                        exact(KeyCode::Up),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::Up),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Char('k')),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::Up),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Down),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::Down),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Char('j')),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::Down),
                        None,
                    )
                    .bind(
                        exact(KeyCode::PageUp),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::PageUp),
                        None,
                    )
                    .bind(
                        exact(KeyCode::PageDown),
                        |_| DraftScreenMessage::ScrollFocused(ScrollDirection::PageDown),
                        None,
                    );
            }

            // Filter bindings: only on tabs that support filtering
            if supports_filter {
                recipe = recipe.bind(
                    exact(KeyCode::Char('/')),
                    |_| DraftScreenMessage::ToggleFilter,
                    KbHint::new("/", "Filter"),
                );
            }
            if supports_pos_filter {
                recipe = recipe.bind(
                    exact(KeyCode::Char('p')),
                    |_| DraftScreenMessage::OpenPositionFilter,
                    KbHint::new("p", "Pos filter"),
                );
            }

            kb.subscribe(recipe)
        };

        Subscription::batch([modal_sub, main_sub, sidebar_sub, own_sub])
    }
}

// ---------------------------------------------------------------------------
// DraftScreenMessage
// ---------------------------------------------------------------------------

/// Messages that can be dispatched to [`DraftScreen`].
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
                        ModalLayerAction::QuitConfirm(crate::tui::confirm_dialog::ConfirmResult::Confirmed('y' | 'q')) => {
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
