// TUI dashboard: layout, input handling, and widget rendering.
//
// The TUI owns a `ViewState` that mirrors relevant parts of the application
// state. The app orchestrator pushes `UiUpdate` messages over an mpsc channel;
// the TUI applies them to `ViewState` and re-renders at ~30 fps.

pub mod input;
pub mod layout;
pub mod onboarding;
pub mod widgets;

use std::collections::HashMap;
use std::time::Duration;

use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::draft::pick::{DraftPick, Position};
use crate::draft::roster::RosterSlot;
use crate::protocol::{
    AppMode, AppSnapshot, ConnectionStatus, InstantAnalysis, LlmStatus, NominationInfo,
    TabFeature, TabId, UiUpdate, UserCommand,
};
use crate::valuation::scarcity::ScarcityEntry;
use crate::valuation::zscore::PlayerValuation;

use layout::{build_layout, AppLayout};
pub use onboarding::llm_setup::LlmSetupState;

// ---------------------------------------------------------------------------
// FocusPanel
// ---------------------------------------------------------------------------

/// Identifies which panel currently has keyboard focus for scroll routing.
///
/// When `None`, scroll events go to the active tab's main panel (backward
/// compatible default). When `Some(panel)`, scroll events are dispatched
/// exclusively to the focused panel. Tab cycles through the panels; Esc
/// clears focus back to `None`.
///
/// The cycle order follows left-to-right, then top-to-bottom within columns:
/// `None -> MainPanel -> Roster -> Scarcity -> Budget -> NominationPlan -> None`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel {
    /// The active tab's content area (left side).
    MainPanel,
    /// Sidebar: My Roster panel.
    Roster,
    /// Sidebar: Positional Scarcity panel.
    Scarcity,
    /// Sidebar: Budget panel.
    Budget,
    /// Sidebar: Nomination Plan panel.
    NominationPlan,
}

impl FocusPanel {
    /// Ordered list of panels for cycling.
    const CYCLE: &[FocusPanel] = &[
        FocusPanel::MainPanel,
        FocusPanel::Roster,
        FocusPanel::Scarcity,
        FocusPanel::Budget,
        FocusPanel::NominationPlan,
    ];

    /// Advance focus forward:
    /// None -> MainPanel -> Roster -> Scarcity -> Budget -> NominationPlan -> None
    pub fn next(current: Option<FocusPanel>) -> Option<FocusPanel> {
        match current {
            None => Some(Self::CYCLE[0]),
            Some(panel) => {
                let idx = Self::CYCLE.iter().position(|&p| p == panel);
                match idx {
                    Some(i) if i + 1 < Self::CYCLE.len() => Some(Self::CYCLE[i + 1]),
                    _ => None,
                }
            }
        }
    }

    /// Advance focus backward:
    /// None -> NominationPlan -> Budget -> Scarcity -> Roster -> MainPanel -> None
    pub fn prev(current: Option<FocusPanel>) -> Option<FocusPanel> {
        match current {
            None => Some(*Self::CYCLE.last().unwrap()),
            Some(panel) => {
                let idx = Self::CYCLE.iter().position(|&p| p == panel);
                match idx {
                    Some(0) => None,
                    Some(i) => Some(Self::CYCLE[i - 1]),
                    None => None,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// KeybindHint
// ---------------------------------------------------------------------------

/// A single keyboard shortcut hint displayed in the help bar.
///
/// Each hint pairs a key label (e.g. `"q"`, `"Tab"`, `"↑↓"`) with a short
/// human-readable description (e.g. `"Quit"`, `"Focus"`, `"Scroll"`).
///
/// The active set of hints is stored in [`ViewState::active_keybinds`],
/// recomputed on every render frame by [`compute_keybinds`]. The help bar is
/// a dumb renderer that displays whatever hints are present there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindHint {
    /// Short key label shown in the help bar (e.g. `"q"`, `"Tab"`, `"↑↓/j/k"`).
    pub key: String,
    /// Human-readable description of the action (e.g. `"Quit"`, `"Focus"`).
    pub description: String,
}

impl KeybindHint {
    /// Construct a new hint from string-like values.
    pub fn new(key: impl Into<String>, description: impl Into<String>) -> Self {
        KeybindHint {
            key: key.into(),
            description: description.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// BudgetStatus
// ---------------------------------------------------------------------------

/// Snapshot of the user's team budget state for display.
#[derive(Debug, Clone)]
pub struct BudgetStatus {
    /// Total salary spent so far.
    pub spent: u32,
    /// Remaining salary cap.
    pub remaining: u32,
    /// Per-team salary cap.
    pub cap: u32,
    /// Current league-wide inflation rate.
    pub inflation_rate: f64,
    /// Maximum bid the user can make right now.
    pub max_bid: u32,
    /// Average dollars remaining per empty roster slot.
    pub avg_per_slot: f64,
}

impl Default for BudgetStatus {
    fn default() -> Self {
        BudgetStatus {
            spent: 0,
            remaining: 260,
            cap: 260,
            inflation_rate: 1.0,
            max_bid: 0,
            avg_per_slot: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// TeamSummary
// ---------------------------------------------------------------------------

/// Lightweight summary of a team's draft state for the Teams widget.
#[derive(Debug, Clone)]
pub struct TeamSummary {
    /// Team display name.
    pub name: String,
    /// Remaining salary cap.
    pub budget_remaining: u32,
    /// Number of filled roster slots.
    pub slots_filled: usize,
    /// Total draftable roster slots.
    pub total_slots: usize,
}

// ---------------------------------------------------------------------------
// ViewState
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// PositionFilterModal
// ---------------------------------------------------------------------------

/// State for the position filter modal overlay.
///
/// When `open` is true the modal is shown on the Available Players tab.
/// The user can navigate with arrow keys, type to narrow the list, press
/// Enter to apply the highlighted selection, or press Escape to cancel.
#[derive(Debug, Clone, Default)]
pub struct PositionFilterModal {
    /// Whether the modal is currently visible.
    pub open: bool,
    /// Incremental search text typed by the user while the modal is open.
    pub search_text: String,
    /// Index into the *filtered* list of options that is currently highlighted.
    pub selected_index: usize,
}

impl PositionFilterModal {
    /// The full ordered list of selectable options (None = "ALL").
    ///
    /// `None` represents "no filter" (show all positions).
    pub const OPTIONS: &'static [Option<Position>] = &[
        None,
        Some(Position::Catcher),
        Some(Position::FirstBase),
        Some(Position::SecondBase),
        Some(Position::ThirdBase),
        Some(Position::ShortStop),
        Some(Position::LeftField),
        Some(Position::CenterField),
        Some(Position::RightField),
        Some(Position::Utility),
        Some(Position::StartingPitcher),
        Some(Position::ReliefPitcher),
    ];

    /// Return the display label for an option.
    pub fn option_label(opt: Option<Position>) -> &'static str {
        match opt {
            None => "ALL",
            Some(p) => p.display_str(),
        }
    }

    /// Return the subset of options that match the current search text
    /// (case-insensitive prefix or substring match).
    pub fn filtered_options(&self) -> Vec<Option<Position>> {
        let search = self.search_text.to_uppercase();
        Self::OPTIONS
            .iter()
            .copied()
            .filter(|opt| {
                let label = Self::option_label(*opt);
                label.contains(search.as_str())
            })
            .collect()
    }
}

/// TUI-local state that mirrors the application state for rendering.
///
/// Updated incrementally via `UiUpdate` messages from the app orchestrator.
/// The `render_frame` function reads this struct to draw the dashboard.
pub struct ViewState {
    /// Current app mode (Onboarding, Draft, or Settings).
    pub app_mode: AppMode,
    /// Current active nomination, if any.
    pub current_nomination: Option<NominationInfo>,
    /// Instant analysis for the current nomination.
    pub instant_analysis: Option<InstantAnalysis>,
    /// All available (undrafted) players sorted by value.
    pub available_players: Vec<PlayerValuation>,
    /// Positional scarcity entries.
    pub positional_scarcity: Vec<ScarcityEntry>,
    /// User's team budget status.
    pub budget: BudgetStatus,
    /// Current inflation rate.
    pub inflation: f64,
    /// Accumulated LLM analysis text (streamed tokens).
    pub analysis_text: String,
    /// Status of the LLM analysis stream.
    pub analysis_status: LlmStatus,
    /// Accumulated LLM nomination plan text (streamed tokens).
    pub plan_text: String,
    /// Status of the LLM plan stream.
    pub plan_status: LlmStatus,
    /// WebSocket connection status.
    pub connection_status: ConnectionStatus,
    /// Number of picks completed.
    pub pick_number: usize,
    /// Total picks in the draft.
    pub total_picks: usize,
    /// Which tab is active in the main panel.
    pub active_tab: TabId,
    /// Per-widget scroll offsets (keyed by widget name).
    pub scroll_offset: HashMap<String, usize>,
    /// Current filter/search text.
    pub filter_text: String,
    /// Whether the filter input is active.
    pub filter_mode: bool,
    /// Position filter for the available players table.
    pub position_filter: Option<Position>,
    /// Whether the quit confirmation dialog is showing.
    pub confirm_quit: bool,
    /// Chronological list of completed draft picks.
    pub draft_log: Vec<DraftPick>,
    /// Summary of each team's draft state.
    pub team_summaries: Vec<TeamSummary>,
    /// User's roster slots (position + optional player).
    pub my_roster: Vec<RosterSlot>,
    /// Which panel currently has keyboard focus for scroll routing.
    /// `None` means no panel is focused (scroll goes to active tab by default).
    pub focused_panel: Option<FocusPanel>,
    /// Position filter modal state.
    pub position_filter_modal: PositionFilterModal,
    /// Active keybind hints displayed in the help bar.
    ///
    /// Recomputed on every render frame by [`compute_keybinds`] based on the
    /// current UI mode and active tab. The help bar renders these directly
    /// without any additional logic.
    pub active_keybinds: Vec<KeybindHint>,
    /// State for the LLM setup onboarding screen.
    pub llm_setup: LlmSetupState,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            app_mode: AppMode::Draft,
            current_nomination: None,
            instant_analysis: None,
            available_players: Vec::new(),
            positional_scarcity: Vec::new(),
            budget: BudgetStatus::default(),
            inflation: 1.0,
            analysis_text: String::new(),
            analysis_status: LlmStatus::Idle,
            plan_text: String::new(),
            plan_status: LlmStatus::Idle,
            connection_status: ConnectionStatus::Disconnected,
            pick_number: 0,
            total_picks: 0,
            active_tab: TabId::Analysis,
            scroll_offset: HashMap::new(),
            filter_text: String::new(),
            filter_mode: false,
            position_filter: None,
            confirm_quit: false,
            draft_log: Vec::new(),
            team_summaries: Vec::new(),
            my_roster: Vec::new(),
            focused_panel: None,
            position_filter_modal: PositionFilterModal::default(),
            active_keybinds: Vec::new(),
            llm_setup: LlmSetupState::default(),
        }
    }
}

impl ViewState {
    /// Apply a full state snapshot from the app orchestrator.
    ///
    /// This updates all fields that the snapshot provides. Fields not
    /// covered by the snapshot (e.g. LLM text, scroll offsets) are left
    /// unchanged.
    pub fn apply_snapshot(&mut self, snapshot: AppSnapshot) {
        self.app_mode = snapshot.app_mode;
        self.pick_number = snapshot.pick_count;
        self.total_picks = snapshot.total_picks;
        if let Some(tab) = snapshot.active_tab {
            if self.active_tab != tab {
                self.focused_panel = None;
            }
            self.active_tab = tab;
        }

        // Recalculated data from the valuation pipeline
        self.available_players = snapshot.available_players;
        self.positional_scarcity = snapshot.positional_scarcity;
        self.draft_log = snapshot.draft_log;
        self.my_roster = snapshot.my_roster;

        // Budget status
        self.budget = BudgetStatus {
            spent: snapshot.budget_spent,
            remaining: snapshot.budget_remaining,
            cap: snapshot.salary_cap,
            inflation_rate: snapshot.inflation_rate,
            max_bid: snapshot.max_bid,
            avg_per_slot: snapshot.avg_per_slot,
        };

        // Inflation rate
        self.inflation = snapshot.inflation_rate;

        // Team summaries
        self.team_summaries = snapshot
            .team_snapshots
            .into_iter()
            .map(|ts| TeamSummary {
                name: ts.name,
                budget_remaining: ts.budget_remaining,
                slots_filled: ts.slots_filled,
                total_slots: ts.total_slots,
            })
            .collect();
    }
}

// ---------------------------------------------------------------------------
// UiUpdate processing
// ---------------------------------------------------------------------------

/// Apply a single UiUpdate to the ViewState.
fn apply_ui_update(state: &mut ViewState, update: UiUpdate) {
    match update {
        UiUpdate::StateSnapshot(snapshot) => {
            state.apply_snapshot(*snapshot);
        }
        UiUpdate::NominationUpdate(nomination) => {
            state.current_nomination = Some(*nomination);
            // Clear previous analysis text and instant analysis when a new nomination arrives
            state.analysis_text.clear();
            state.analysis_status = LlmStatus::Idle;
            state.instant_analysis = None;
            // Clear focused panel to avoid a stale cyan border on the new nomination
            state.focused_panel = None;
            // Reset main panel scroll offsets so the new nomination context is visible from the top.
            // This ensures the nominated player highlight in the Available tab is not scrolled off screen.
            state.scroll_offset.insert("available".to_string(), 0);
            state.scroll_offset.insert("analysis".to_string(), 0);
        }
        UiUpdate::BidUpdate(nomination) => {
            // Update nomination info (new bid) but preserve LLM streaming text
            state.current_nomination = Some(*nomination);
        }
        UiUpdate::NominationCleared => {
            state.current_nomination = None;
            state.instant_analysis = None;
            state.analysis_text.clear();
            state.analysis_status = LlmStatus::Idle;
            state.focused_panel = None;
        }
        UiUpdate::AnalysisToken(token) => {
            state.analysis_text.push_str(&token);
            state.analysis_status = LlmStatus::Streaming;
        }
        UiUpdate::AnalysisComplete(final_text) => {
            state.analysis_text = final_text;
            state.analysis_status = LlmStatus::Complete;
        }
        UiUpdate::AnalysisError(msg) => {
            state.analysis_text.clear();
            state.analysis_text.push_str(&format!("[Error: {}]", msg));
            state.analysis_status = LlmStatus::Error;
        }
        UiUpdate::PlanStarted => {
            state.plan_text.clear();
            state.plan_status = LlmStatus::Streaming;
        }
        UiUpdate::PlanToken(token) => {
            state.plan_text.push_str(&token);
            state.plan_status = LlmStatus::Streaming;
        }
        UiUpdate::PlanComplete(final_text) => {
            state.plan_text = final_text;
            state.plan_status = LlmStatus::Complete;
        }
        UiUpdate::PlanError(msg) => {
            state.plan_text.clear();
            state.plan_text.push_str(&format!("[Error: {}]", msg));
            state.plan_status = LlmStatus::Error;
        }
        UiUpdate::ConnectionStatus(status) => {
            state.connection_status = status;
        }
        UiUpdate::OnboardingUpdate(update) => {
            use crate::protocol::OnboardingUpdate;
            use crate::llm::provider::models_for_provider;
            use onboarding::llm_setup::LlmConnectionStatus;

            match update {
                OnboardingUpdate::ConnectionTestResult { success, message } => {
                    state.llm_setup.connection_status = if success {
                        LlmConnectionStatus::Success(message)
                    } else {
                        LlmConnectionStatus::Failed(message)
                    };
                }
                OnboardingUpdate::ProgressSync { provider, model } => {
                    // Rebuild LlmSetupState indices from the saved progress.
                    if let Some(ref p) = provider {
                        if let Some(idx) = LlmSetupState::PROVIDERS.iter().position(|pp| pp == p) {
                            state.llm_setup.selected_provider_idx = idx;
                        }
                        if let Some(ref model_id) = model {
                            let models = models_for_provider(p);
                            if let Some(midx) = models.iter().position(|m| m.model_id == model_id.as_str()) {
                                state.llm_setup.selected_model_idx = midx;
                            }
                        }
                    }
                }
            }
        }
        UiUpdate::ModeChanged(mode) => {
            state.app_mode = mode;
        }
    }
}

// ---------------------------------------------------------------------------
// Keybind computation
// ---------------------------------------------------------------------------

/// Compute the set of active keybind hints for the current UI state.
///
/// This is the single declarative source of truth for what appears in the
/// help bar. It is called once per render frame and the result is stored in
/// [`ViewState::active_keybinds`] so that the help bar widget is a dumb
/// renderer with no conditional logic of its own.
///
/// Dispatches to mode-specific hint builders first. Draft mode then uses the
/// priority order:
/// 1. Quit confirmation dialog
/// 2. Position filter modal
/// 3. Text filter mode (inline input bar)
/// 4. Normal mode with tab-specific and focus-specific hints
pub fn compute_keybinds(state: &ViewState) -> Vec<KeybindHint> {
    match &state.app_mode {
        AppMode::Onboarding(step) => compute_onboarding_keybinds(state, step),
        AppMode::Settings(_) => {
            vec![
                KeybindHint::new("Esc", "Back to Draft"),
                KeybindHint::new("q", "Quit"),
            ]
        }
        AppMode::Draft => compute_draft_keybinds(state),
    }
}

/// Compute keybind hints for onboarding mode.
fn compute_onboarding_keybinds(state: &ViewState, step: &crate::onboarding::OnboardingStep) -> Vec<KeybindHint> {
    use crate::onboarding::OnboardingStep;
    use onboarding::llm_setup::LlmSetupSection;

    match step {
        OnboardingStep::LlmSetup => {
            if state.llm_setup.api_key_editing {
                vec![
                    KeybindHint::new("type", "Input key"),
                    KeybindHint::new("Enter", "Confirm"),
                    KeybindHint::new("Esc", "Cancel"),
                ]
            } else {
                let mut hints = vec![
                    KeybindHint::new("^v", "Select"),
                    KeybindHint::new("Tab", "Section"),
                ];
                if state.llm_setup.active_section == LlmSetupSection::ApiKey {
                    hints.push(KeybindHint::new("Enter", "Edit key"));
                } else if state.llm_setup.active_section == LlmSetupSection::TestButton {
                    hints.push(KeybindHint::new("Enter", "Test"));
                }
                hints.push(KeybindHint::new("n", "Next"));
                hints.push(KeybindHint::new("Esc", "Back"));
                hints
            }
        }
        OnboardingStep::StrategySetup | OnboardingStep::Complete => {
            vec![
                KeybindHint::new("n", "Next"),
                KeybindHint::new("Esc", "Back"),
                KeybindHint::new("q", "Quit"),
            ]
        }
    }
}

/// Compute keybind hints for draft mode.
fn compute_draft_keybinds(state: &ViewState) -> Vec<KeybindHint> {
    // 1. Quit confirmation overlay: all other input is blocked
    if state.confirm_quit {
        return vec![
            KeybindHint::new("y/q", "Confirm quit"),
            KeybindHint::new("n/Esc", "Cancel"),
        ];
    }

    // 2. Position filter modal
    if state.position_filter_modal.open {
        return vec![
            KeybindHint::new("↑↓", "Navigate"),
            KeybindHint::new("Enter", "Select"),
            KeybindHint::new("Esc", "Cancel"),
        ];
    }

    // 3. Text filter mode (the inline filter input bar)
    if state.filter_mode {
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
    if state.active_tab.supports(TabFeature::Filter) {
        hints.push(KeybindHint::new("/", "Filter"));
        hints.push(KeybindHint::new("p", "Pos"));
    }

    // Focus cycling and resync are always available in normal mode
    hints.push(KeybindHint::new("Tab", "Focus"));
    hints.push(KeybindHint::new("r", "Resync"));

    // Scroll hint only appears when a panel is focused (scroll is routed there)
    if state.focused_panel.is_some() {
        hints.push(KeybindHint::new("↑↓/j/k/PgUp/PgDn", "Scroll"));
    }

    // Active filter reminder: shown as a trailing hint when the Available tab
    // has a non-empty filter so the user knows results are currently filtered.
    if !state.filter_text.is_empty() && state.active_tab == TabId::Available {
        hints.push(KeybindHint::new(
            format!("filter:\"{}\"", state.filter_text),
            "active",
        ));
    }

    hints
}

// ---------------------------------------------------------------------------
// Render frame
// ---------------------------------------------------------------------------

/// Render the complete dashboard frame.
///
/// Dispatches to different render paths based on the current app mode:
/// - `Draft` renders the full draft dashboard (tabs, sidebar, help bar)
/// - `Onboarding` renders a placeholder screen (real UI in Task 4)
/// - `Settings` renders a placeholder screen (real UI in Task 6)
///
/// Note: active keybind hints are read from `state.active_keybinds`, which is
/// pre-synced by the run loop before each draw call. This avoids recomputing
/// keybinds inside the render path.
fn render_frame(frame: &mut Frame, state: &ViewState) {
    match &state.app_mode {
        AppMode::Onboarding(step) => {
            onboarding::render(frame, step, state);
        }
        AppMode::Settings(_section) => {
            render_placeholder(frame, "Settings", &state.active_keybinds);
        }
        AppMode::Draft => {
            render_draft_frame(frame, state);
        }
    }
}

/// Render the full draft dashboard (the main operational view).
fn render_draft_frame(frame: &mut Frame, state: &ViewState) {
    let layout = build_layout(frame.area());

    widgets::status_bar::render(frame, layout.status_bar, state);
    widgets::nomination_banner::render(frame, layout.nomination_banner, state);

    let main_focused = state.focused_panel == Some(FocusPanel::MainPanel);
    let roster_focused = state.focused_panel == Some(FocusPanel::Roster);
    let scarcity_focused = state.focused_panel == Some(FocusPanel::Scarcity);
    let budget_focused = state.focused_panel == Some(FocusPanel::Budget);
    let nom_plan_focused = state.focused_panel == Some(FocusPanel::NominationPlan);

    // Main panel: tab-dependent content
    match state.active_tab {
        TabId::Analysis => widgets::llm_analysis::render(frame, layout.main_panel, state, main_focused),
        TabId::Available => widgets::available::render(frame, layout.main_panel, state, main_focused),
        TabId::DraftLog => widgets::draft_log::render(frame, layout.main_panel, state, main_focused),
        TabId::Teams => widgets::teams::render(frame, layout.main_panel, state, main_focused),
    }

    // Sidebar widgets (each with individual focus)
    widgets::roster::render(frame, layout.roster, state, roster_focused);
    widgets::scarcity::render(frame, layout.scarcity, state, scarcity_focused);
    widgets::budget::render(frame, layout.budget, state, budget_focused);
    widgets::nomination_plan::render(frame, layout.nomination_plan, state, nom_plan_focused);

    // Help bar: dumb renderer of the pre-synced active keybind hints
    render_help_bar(frame, &layout, state, &state.active_keybinds);

    // Position filter modal overlay
    if state.position_filter_modal.open {
        widgets::position_filter_modal::render(frame, frame.area(), state);
    }

    // Quit confirm dialog rendered last so it appears on top of everything
    if state.confirm_quit {
        widgets::quit_confirm::render(frame, frame.area(), state);
    }
}

/// Render a centered placeholder screen with the given message and a help bar.
///
/// Used for Onboarding and Settings modes until their real UIs are implemented.
fn render_placeholder(frame: &mut Frame, message: &str, keybinds: &[KeybindHint]) {
    use ratatui::layout::{Alignment, Constraint, Layout};

    let area = frame.area();

    // Split into main content area and a 1-line help bar at the bottom
    let outer = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    // Vertically center the text block within the main content area
    let vertical = Layout::vertical([
        Constraint::Percentage(40),
        Constraint::Length(3),
        Constraint::Percentage(40),
    ])
    .split(outer[0]);

    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(
            message,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(Color::Black));

    frame.render_widget(paragraph, vertical[1]);

    // Help bar: render keybind hints at the bottom
    let mut spans: Vec<Span> = Vec::new();
    for (i, hint) in keybinds.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(
            format!(" {}:{}", hint.key, hint.description),
            Style::default().fg(Color::Gray),
        ));
    }
    let help_bar = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Black));
    frame.render_widget(help_bar, outer[1]);
}

/// Render the help bar using the pre-computed keybind hints.
///
/// This function is a dumb renderer: it knows nothing about modes, tabs, or
/// focus. All context-sensitivity lives in [`compute_keybinds`]. The special
/// case for filter mode (showing an inline input bar) is still handled here
/// because it requires displaying live `ViewState` data (the current filter
/// text and cursor), not just static hint labels.
fn render_help_bar(
    frame: &mut Frame,
    layout: &AppLayout,
    state: &ViewState,
    keybinds: &[KeybindHint],
) {
    // Filter mode: show a dedicated inline filter input bar instead of hints.
    // This is handled here (not in compute_keybinds) because the input bar
    // embeds the live filter_text content, which is display state rather than
    // a keybind description.
    if state.filter_mode {
        let spans = vec![
            Span::styled(
                " FILTER ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(
                &state.filter_text,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("▎", Style::default().fg(Color::Cyan)),
            Span::styled(
                "  (Enter:apply | Esc:cancel)",
                Style::default().fg(Color::DarkGray),
            ),
        ];
        let paragraph = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, layout.help_bar);
        return;
    }

    // Normal / modal modes: render the precomputed hint list.
    // Format: " key:desc | key:desc | ..."
    let mut spans: Vec<Span> = Vec::new();
    for (i, hint) in keybinds.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(
            format!(" {}:{}", hint.key, hint.description),
            Style::default().fg(Color::Gray),
        ));
    }

    let paragraph = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, layout.help_bar);
}

// ---------------------------------------------------------------------------
// Main TUI loop
// ---------------------------------------------------------------------------

/// Run the TUI event loop.
///
/// This is the main entry point for the terminal UI. It:
/// 1. Initializes the terminal (enters raw mode, enables alternate screen).
/// 2. Installs a panic hook to restore the terminal on crash.
/// 3. Runs an async select loop: UI updates, keyboard input, render ticks.
/// 4. Restores the terminal on clean exit.
pub async fn run(
    mut ui_rx: mpsc::Receiver<UiUpdate>,
    cmd_tx: mpsc::Sender<UserCommand>,
    initial_mode: AppMode,
) -> anyhow::Result<()> {
    // 1. Initialize terminal
    let mut terminal = ratatui::init();

    // 2. Set panic hook to restore terminal on crash.
    //    We capture the original hook and chain ours before it.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Best-effort terminal restoration
        let _ = ratatui::restore();
        original_hook(panic_info);
    }));

    // 3. Create ViewState with the initial app mode so the first frame
    //    renders the correct screen (avoids a flash of the draft UI when
    //    the app starts in onboarding mode).
    let mut view_state = ViewState {
        app_mode: initial_mode,
        ..ViewState::default()
    };

    // 4. Create crossterm EventStream for async keyboard input
    let mut event_stream = EventStream::new();

    // 5. Create render interval (~30fps)
    let mut render_tick = tokio::time::interval(Duration::from_millis(33));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // 6. Main loop
    loop {
        tokio::select! {
            // UI updates from the app orchestrator
            update = ui_rx.recv() => {
                match update {
                    Some(ui_update) => {
                        apply_ui_update(&mut view_state, ui_update);
                    }
                    None => {
                        // Channel closed: app is shutting down
                        break;
                    }
                }
            }

            // Keyboard input
            maybe_event = event_stream.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key_event))) => {
                        // Delegate to input handler
                        if let Some(cmd) = input::handle_key(key_event, &mut view_state) {
                            let is_quit = cmd == UserCommand::Quit;
                            let _ = cmd_tx.send(cmd).await;
                            if is_quit {
                                break;
                            }
                        }
                    }
                    Some(Ok(_)) => {
                        // Mouse events, resize events, etc. -- ignore for now
                    }
                    Some(Err(_)) => {
                        // Input error -- break out
                        break;
                    }
                    None => {
                        // Stream ended
                        break;
                    }
                }
            }

            // Render tick
            _ = render_tick.tick() => {
                // Sync active keybinds into ViewState before rendering so the
                // field reflects the current hints (useful for testing and
                // any future consumers of ViewState outside the render path).
                view_state.active_keybinds = compute_keybinds(&view_state);
                terminal.draw(|frame| render_frame(frame, &view_state))?;
            }
        }
    }

    // 7. Restore terminal
    ratatui::restore();

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AppMode, TeamSnapshot};

    // -- FocusPanel cycling --

    #[test]
    fn focus_next_cycles_forward() {
        assert_eq!(FocusPanel::next(None), Some(FocusPanel::MainPanel));
        assert_eq!(FocusPanel::next(Some(FocusPanel::MainPanel)), Some(FocusPanel::Roster));
        assert_eq!(FocusPanel::next(Some(FocusPanel::Roster)), Some(FocusPanel::Scarcity));
        assert_eq!(FocusPanel::next(Some(FocusPanel::Scarcity)), Some(FocusPanel::Budget));
        assert_eq!(FocusPanel::next(Some(FocusPanel::Budget)), Some(FocusPanel::NominationPlan));
        assert_eq!(FocusPanel::next(Some(FocusPanel::NominationPlan)), None);
    }

    #[test]
    fn focus_prev_cycles_backward() {
        assert_eq!(FocusPanel::prev(None), Some(FocusPanel::NominationPlan));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::NominationPlan)), Some(FocusPanel::Budget));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::Budget)), Some(FocusPanel::Scarcity));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::Scarcity)), Some(FocusPanel::Roster));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::Roster)), Some(FocusPanel::MainPanel));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::MainPanel)), None);
    }

    #[test]
    fn focus_next_then_prev_is_identity() {
        // Starting from None, next then prev should return to None
        let step1 = FocusPanel::next(None);
        let step2 = FocusPanel::prev(step1);
        assert_eq!(step2, None);
    }

    #[test]
    fn view_state_default_is_sensible() {
        let state = ViewState::default();
        assert_eq!(state.app_mode, AppMode::Draft);
        assert!(state.current_nomination.is_none());
        assert!(state.instant_analysis.is_none());
        assert!(state.available_players.is_empty());
        assert!(state.positional_scarcity.is_empty());
        assert_eq!(state.pick_number, 0);
        assert_eq!(state.total_picks, 0);
        assert_eq!(state.active_tab, TabId::Analysis);
        assert_eq!(state.connection_status, ConnectionStatus::Disconnected);
        assert_eq!(state.analysis_status, LlmStatus::Idle);
        assert_eq!(state.plan_status, LlmStatus::Idle);
        assert!(state.analysis_text.is_empty());
        assert!(state.plan_text.is_empty());
        assert!(state.scroll_offset.is_empty());
        assert!(!state.filter_mode);
        assert!(state.filter_text.is_empty());
        assert!(state.position_filter.is_none());
        assert!(!state.confirm_quit);
        assert!(state.draft_log.is_empty());
        assert!(state.team_summaries.is_empty());
        assert!(state.my_roster.is_empty());
        assert!(state.focused_panel.is_none());
        assert!(!state.position_filter_modal.open);
        assert!(state.position_filter_modal.search_text.is_empty());
        assert_eq!(state.position_filter_modal.selected_index, 0);
    }

    #[test]
    fn budget_status_default() {
        let budget = BudgetStatus::default();
        assert_eq!(budget.spent, 0);
        assert_eq!(budget.remaining, 260);
        assert_eq!(budget.cap, 260);
        assert!((budget.inflation_rate - 1.0).abs() < f64::EPSILON);
        assert_eq!(budget.max_bid, 0);
        assert!((budget.avg_per_slot - 0.0).abs() < f64::EPSILON);
    }

    /// Helper to build a test AppSnapshot with sensible defaults.
    fn test_snapshot(pick_count: usize, total_picks: usize, active_tab: Option<TabId>) -> AppSnapshot {
        AppSnapshot {
            app_mode: AppMode::Draft,
            pick_count,
            total_picks,
            active_tab,
            available_players: vec![],
            positional_scarcity: vec![],
            draft_log: vec![],
            my_roster: vec![],
            budget_spent: 0,
            budget_remaining: 260,
            salary_cap: 260,
            inflation_rate: 1.0,
            max_bid: 0,
            avg_per_slot: 0.0,
            team_snapshots: vec![],
        }
    }

    #[test]
    fn apply_snapshot_updates_fields() {
        let mut state = ViewState::default();
        let snapshot = test_snapshot(42, 260, Some(TabId::Teams));
        state.apply_snapshot(snapshot);
        assert_eq!(state.pick_number, 42);
        assert_eq!(state.total_picks, 260);
        assert_eq!(state.active_tab, TabId::Teams);
    }

    #[test]
    fn apply_snapshot_preserves_tab_when_none() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        let snapshot = test_snapshot(10, 260, None);
        state.apply_snapshot(snapshot);
        assert_eq!(state.pick_number, 10);
        assert_eq!(state.active_tab, TabId::Available);
    }

    #[test]
    fn apply_ui_update_state_snapshot() {
        let mut state = ViewState::default();
        let snapshot = test_snapshot(5, 100, Some(TabId::DraftLog));
        apply_ui_update(&mut state, UiUpdate::StateSnapshot(Box::new(snapshot)));
        assert_eq!(state.pick_number, 5);
        assert_eq!(state.total_picks, 100);
        assert_eq!(state.active_tab, TabId::DraftLog);
    }

    #[test]
    fn apply_snapshot_updates_budget_and_teams() {
        let mut state = ViewState::default();
        let mut snapshot = test_snapshot(10, 260, None);
        snapshot.budget_spent = 100;
        snapshot.budget_remaining = 160;
        snapshot.inflation_rate = 1.15;
        snapshot.max_bid = 140;
        snapshot.avg_per_slot = 10.0;
        snapshot.team_snapshots = vec![
            TeamSnapshot {
                name: "Team 1".into(),
                budget_remaining: 160,
                slots_filled: 5,
                total_slots: 26,
            },
            TeamSnapshot {
                name: "Team 2".into(),
                budget_remaining: 200,
                slots_filled: 3,
                total_slots: 26,
            },
        ];

        state.apply_snapshot(snapshot);

        assert_eq!(state.budget.spent, 100);
        assert_eq!(state.budget.remaining, 160);
        assert!((state.budget.inflation_rate - 1.15).abs() < f64::EPSILON);
        assert_eq!(state.budget.max_bid, 140);
        assert!((state.inflation - 1.15).abs() < f64::EPSILON);
        assert_eq!(state.team_summaries.len(), 2);
        assert_eq!(state.team_summaries[0].name, "Team 1");
        assert_eq!(state.team_summaries[0].budget_remaining, 160);
        assert_eq!(state.team_summaries[0].slots_filled, 5);
        assert_eq!(state.team_summaries[1].name, "Team 2");
        assert_eq!(state.team_summaries[1].budget_remaining, 200);
    }

    #[test]
    fn apply_ui_update_nomination_update() {
        use crate::protocol::{InstantAnalysis, InstantVerdict};

        let mut state = ViewState::default();
        state.analysis_text = "old analysis".to_string();
        state.analysis_status = LlmStatus::Complete;
        state.instant_analysis = Some(InstantAnalysis {
            player_name: "Old Player".to_string(),
            dollar_value: 30.0,
            adjusted_value: 28.0,
            verdict: InstantVerdict::Pass,
        });

        let nom = NominationInfo {
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            nominated_by: "Team Alpha".to_string(),
            current_bid: 45,
            current_bidder: Some("Team Beta".to_string()),
            time_remaining: Some(30),
            eligible_slots: vec![],
        };
        apply_ui_update(&mut state, UiUpdate::NominationUpdate(Box::new(nom)));

        assert!(state.current_nomination.is_some());
        assert_eq!(
            state.current_nomination.as_ref().unwrap().player_name,
            "Mike Trout"
        );
        // Analysis text should be cleared for new nomination
        assert!(state.analysis_text.is_empty());
        assert_eq!(state.analysis_status, LlmStatus::Idle);
        // instant_analysis should also be cleared to avoid stale data from previous nomination
        assert!(state.instant_analysis.is_none());
        // Scroll offsets for available and analysis panels should be reset so the nominated
        // player highlight is visible from the top of the list.
        assert_eq!(state.scroll_offset.get("available").copied(), Some(0));
        assert_eq!(state.scroll_offset.get("analysis").copied(), Some(0));
    }

    #[test]
    fn apply_ui_update_bid_update_preserves_analysis_text() {
        let mut state = ViewState::default();
        // Simulate an active nomination with streaming analysis
        state.current_nomination = Some(NominationInfo {
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            nominated_by: "Team Alpha".to_string(),
            current_bid: 45,
            current_bidder: Some("Team Beta".to_string()),
            time_remaining: Some(30),
            eligible_slots: vec![],
        });
        state.analysis_text = "Trout is a strong target because...".to_string();
        state.analysis_status = LlmStatus::Streaming;

        // A bid update comes in (same player, higher bid)
        let updated_nom = NominationInfo {
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            nominated_by: "Team Alpha".to_string(),
            current_bid: 50,
            current_bidder: Some("Team Gamma".to_string()),
            time_remaining: Some(25),
            eligible_slots: vec![],
        };
        apply_ui_update(&mut state, UiUpdate::BidUpdate(Box::new(updated_nom)));

        // Nomination info should be updated
        let nom = state.current_nomination.as_ref().unwrap();
        assert_eq!(nom.current_bid, 50);
        assert_eq!(nom.current_bidder, Some("Team Gamma".to_string()));
        // Analysis text and status should be preserved
        assert_eq!(state.analysis_text, "Trout is a strong target because...");
        assert_eq!(state.analysis_status, LlmStatus::Streaming);
    }

    #[test]
    fn apply_ui_update_nomination_cleared() {
        let mut state = ViewState::default();
        state.current_nomination = Some(NominationInfo {
            player_name: "Test".to_string(),
            position: "SP".to_string(),
            nominated_by: "Team".to_string(),
            current_bid: 10,
            current_bidder: None,
            time_remaining: None,
            eligible_slots: vec![],
        });
        state.analysis_text = "some analysis".to_string();

        apply_ui_update(&mut state, UiUpdate::NominationCleared);

        assert!(state.current_nomination.is_none());
        assert!(state.instant_analysis.is_none());
        assert!(state.analysis_text.is_empty());
        assert_eq!(state.analysis_status, LlmStatus::Idle);
    }

    #[test]
    fn apply_ui_update_analysis_token() {
        let mut state = ViewState::default();
        apply_ui_update(
            &mut state,
            UiUpdate::AnalysisToken("Hello ".to_string()),
        );
        apply_ui_update(
            &mut state,
            UiUpdate::AnalysisToken("World".to_string()),
        );
        assert_eq!(state.analysis_text, "Hello World");
        assert_eq!(state.analysis_status, LlmStatus::Streaming);
    }

    #[test]
    fn apply_ui_update_analysis_complete() {
        let mut state = ViewState::default();
        state.analysis_status = LlmStatus::Streaming;
        state.analysis_text = "partial token".to_string();
        apply_ui_update(
            &mut state,
            UiUpdate::AnalysisComplete("Full analysis text.".to_string()),
        );
        assert_eq!(state.analysis_status, LlmStatus::Complete);
        // AnalysisComplete carries the final text, which may include a truncation note
        assert_eq!(state.analysis_text, "Full analysis text.");
    }

    #[test]
    fn apply_ui_update_plan_started_clears_previous_text() {
        let mut state = ViewState::default();
        // Simulate old plan text from a previous invocation
        state.plan_text = "Old plan from last pick cycle.".to_string();
        state.plan_status = LlmStatus::Complete;

        apply_ui_update(&mut state, UiUpdate::PlanStarted);

        // PlanStarted must clear plan_text so new tokens don't append to stale content
        assert!(state.plan_text.is_empty(), "plan_text should be cleared on PlanStarted");
        assert_eq!(state.plan_status, LlmStatus::Streaming);
    }

    #[test]
    fn apply_ui_update_plan_started_then_tokens_replace_not_append() {
        let mut state = ViewState::default();
        state.plan_text = "Stale plan text.".to_string();
        state.plan_status = LlmStatus::Complete;

        // A new planning cycle begins
        apply_ui_update(&mut state, UiUpdate::PlanStarted);
        apply_ui_update(&mut state, UiUpdate::PlanToken("New plan: ".to_string()));
        apply_ui_update(&mut state, UiUpdate::PlanToken("nominate X".to_string()));

        // Result must be only the new tokens, not stale text + new tokens
        assert_eq!(state.plan_text, "New plan: nominate X");
        assert_eq!(state.plan_status, LlmStatus::Streaming);
    }

    #[test]
    fn apply_ui_update_plan_token() {
        let mut state = ViewState::default();
        apply_ui_update(&mut state, UiUpdate::PlanToken("Plan: ".to_string()));
        apply_ui_update(&mut state, UiUpdate::PlanToken("nominate X".to_string()));
        assert_eq!(state.plan_text, "Plan: nominate X");
        assert_eq!(state.plan_status, LlmStatus::Streaming);
    }

    #[test]
    fn apply_ui_update_plan_complete() {
        let mut state = ViewState::default();
        state.plan_status = LlmStatus::Streaming;
        state.plan_text = "partial token".to_string();
        apply_ui_update(
            &mut state,
            UiUpdate::PlanComplete("Full plan text.".to_string()),
        );
        assert_eq!(state.plan_status, LlmStatus::Complete);
        // PlanComplete carries the final text, which may include a truncation note
        assert_eq!(state.plan_text, "Full plan text.");
    }

    #[test]
    fn apply_ui_update_connection_status() {
        let mut state = ViewState::default();
        assert_eq!(state.connection_status, ConnectionStatus::Disconnected);
        apply_ui_update(
            &mut state,
            UiUpdate::ConnectionStatus(ConnectionStatus::Connected),
        );
        assert_eq!(state.connection_status, ConnectionStatus::Connected);
    }

    // -- KeybindHint --

    #[test]
    fn keybind_hint_new_stores_fields() {
        let hint = KeybindHint::new("q", "Quit");
        assert_eq!(hint.key, "q");
        assert_eq!(hint.description, "Quit");
    }

    #[test]
    fn keybind_hint_accepts_string_types() {
        let hint = KeybindHint::new(String::from("Tab"), "Focus");
        assert_eq!(hint.key, "Tab");
        assert_eq!(hint.description, "Focus");
    }

    // -- compute_keybinds --

    /// Helper: extract all key labels from a hint list.
    fn keys(hints: &[KeybindHint]) -> Vec<&str> {
        hints.iter().map(|h| h.key.as_str()).collect()
    }

    #[test]
    fn compute_keybinds_normal_mode_base_hints_present() {
        let state = ViewState::default();
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"q"), "should contain quit hint");
        assert!(ks.contains(&"1-4"), "should contain tab-switch hint");
        assert!(ks.contains(&"Tab"), "should contain focus hint");
        assert!(ks.contains(&"r"), "should contain resync hint");
    }

    #[test]
    fn compute_keybinds_no_scroll_hint_without_focus() {
        let mut state = ViewState::default();
        state.focused_panel = None;
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        // Scroll hint should only appear when a panel is focused
        assert!(
            !ks.contains(&"↑↓/j/k/PgUp/PgDn"),
            "scroll hint should not appear without focus"
        );
    }

    #[test]
    fn compute_keybinds_scroll_hint_with_focus() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::MainPanel);
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(
            ks.contains(&"↑↓/j/k/PgUp/PgDn"),
            "scroll hint should appear when a panel is focused"
        );
    }

    #[test]
    fn compute_keybinds_filter_hints_on_available_tab() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"/"), "filter hint should appear on Available tab");
        assert!(ks.contains(&"p"), "pos filter hint should appear on Available tab");
    }

    #[test]
    fn compute_keybinds_no_filter_hints_on_analysis_tab() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Analysis;
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(
            !ks.contains(&"/"),
            "filter hint should not appear on Analysis tab"
        );
    }

    #[test]
    fn compute_keybinds_filter_mode() {
        let mut state = ViewState::default();
        state.filter_mode = true;
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        // Filter mode shows only Enter/Esc
        assert!(ks.contains(&"Enter"), "filter mode should show Enter hint");
        assert!(ks.contains(&"Esc"), "filter mode should show Esc hint");
        // Normal mode hints should not appear
        assert!(!ks.contains(&"q"), "normal quit hint should not appear in filter mode");
        assert!(!ks.contains(&"1-4"), "tab hint should not appear in filter mode");
    }

    #[test]
    fn compute_keybinds_position_modal_open() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"↑↓"), "modal should show navigate hint");
        assert!(ks.contains(&"Enter"), "modal should show select hint");
        assert!(ks.contains(&"Esc"), "modal should show cancel hint");
        // Normal hints suppressed
        assert!(!ks.contains(&"q"), "quit hint should not appear when modal is open");
    }

    #[test]
    fn compute_keybinds_quit_confirm_mode() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"y/q"), "confirm quit hint should appear");
        assert!(ks.contains(&"n/Esc"), "cancel hint should appear");
        // Normal hints suppressed
        assert!(!ks.contains(&"1-4"), "tab hint should not appear in confirm mode");
    }

    #[test]
    fn compute_keybinds_active_filter_reminder_on_available_tab() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        state.filter_text = "trout".to_string();
        let hints = compute_keybinds(&state);
        // There should be a hint whose key contains the filter text
        let has_reminder = hints.iter().any(|h| h.key.contains("trout"));
        assert!(has_reminder, "should show filter reminder hint with filter text");
    }

    #[test]
    fn compute_keybinds_no_filter_reminder_on_analysis_tab() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Analysis;
        state.filter_text = "trout".to_string();
        let hints = compute_keybinds(&state);
        // Filter reminder should only appear on Available tab
        let has_reminder = hints.iter().any(|h| h.key.contains("trout"));
        assert!(
            !has_reminder,
            "filter reminder should not appear on Analysis tab"
        );
    }

    #[test]
    fn view_state_default_active_keybinds_empty() {
        let state = ViewState::default();
        assert!(
            state.active_keybinds.is_empty(),
            "active_keybinds should start empty before first render"
        );
    }

    #[test]
    fn quit_confirm_takes_priority_over_modal_and_filter_mode() {
        // If somehow both confirm_quit and modal are set, confirm_quit wins
        let mut state = ViewState::default();
        state.confirm_quit = true;
        state.position_filter_modal.open = true;
        state.filter_mode = true;
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"y/q"), "quit confirm should take highest priority");
        assert!(!ks.contains(&"↑↓"), "modal nav hint should not appear");
        // Only the quit-confirm hints should be present, not filter-mode Enter
        assert_eq!(hints.len(), 2, "only 2 quit-confirm hints should be present");
    }

    // -- AppMode-aware keybind computation --

    #[test]
    fn compute_keybinds_llm_setup_normal_mode() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"^v"), "LLM setup should show select hint");
        assert!(ks.contains(&"Tab"), "LLM setup should show Tab hint");
        assert!(ks.contains(&"n"), "LLM setup should show Next hint");
        assert!(ks.contains(&"Esc"), "LLM setup should show Back hint");
        // Draft-specific hints should NOT appear
        assert!(!ks.contains(&"1-4"), "tab hints should not appear in onboarding");
    }

    #[test]
    fn compute_keybinds_llm_setup_editing_mode() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.llm_setup.api_key_editing = true;
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"Enter"), "editing should show confirm hint");
        assert!(ks.contains(&"Esc"), "editing should show cancel hint");
        assert!(!ks.contains(&"n"), "editing should not show Next hint");
    }

    #[test]
    fn compute_keybinds_strategy_setup_placeholder() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"n"), "strategy setup should show Next hint");
        assert!(ks.contains(&"Esc"), "strategy setup should show Back hint");
        assert!(ks.contains(&"q"), "strategy setup should show Quit hint");
    }

    #[test]
    fn compute_keybinds_settings_mode() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"Esc"), "settings should show Back hint");
        assert!(ks.contains(&"q"), "settings should show Quit hint");
        // Draft-specific hints should NOT appear
        assert!(!ks.contains(&"1-4"), "tab hints should not appear in settings");
        assert!(!ks.contains(&"Tab"), "focus hint should not appear in settings");
    }

    #[test]
    fn compute_keybinds_draft_mode_unchanged() {
        // Verify that draft mode keybinds are the same as before (no regression)
        let mut state = ViewState::default();
        state.app_mode = AppMode::Draft;
        let hints = compute_keybinds(&state);
        let ks = keys(&hints);
        assert!(ks.contains(&"q"), "draft mode should contain quit hint");
        assert!(ks.contains(&"1-4"), "draft mode should contain tab-switch hint");
        assert!(ks.contains(&"Tab"), "draft mode should contain focus hint");
        assert!(ks.contains(&"r"), "draft mode should contain resync hint");
    }

    // -- AppMode in ViewState --

    #[test]
    fn apply_snapshot_updates_app_mode() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        assert_eq!(state.app_mode, AppMode::Draft);

        let mut snapshot = test_snapshot(0, 0, None);
        snapshot.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        state.apply_snapshot(snapshot);
        assert_eq!(state.app_mode, AppMode::Onboarding(OnboardingStep::StrategySetup));
    }

    #[test]
    fn apply_ui_update_mode_changed() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        assert_eq!(state.app_mode, AppMode::Draft);

        apply_ui_update(
            &mut state,
            UiUpdate::ModeChanged(AppMode::Onboarding(OnboardingStep::LlmSetup)),
        );
        assert_eq!(state.app_mode, AppMode::Onboarding(OnboardingStep::LlmSetup));
    }

    #[test]
    fn apply_ui_update_mode_changed_to_draft() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);

        apply_ui_update(&mut state, UiUpdate::ModeChanged(AppMode::Draft));
        assert_eq!(state.app_mode, AppMode::Draft);
    }
}
