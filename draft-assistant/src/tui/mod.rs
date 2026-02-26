// TUI dashboard: layout, input handling, and widget rendering.
//
// The TUI owns a `ViewState` that mirrors relevant parts of the application
// state. The app orchestrator pushes `UiUpdate` messages over an mpsc channel;
// the TUI applies them to `ViewState` and re-renders at ~30 fps.

pub mod input;
pub mod layout;
pub mod widgets;

use std::collections::HashMap;
use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures_util::StreamExt;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::draft::pick::Position;
use crate::protocol::{
    AppSnapshot, ConnectionStatus, InstantAnalysis, LlmStatus, NominationInfo, TabId, UiUpdate,
    UserCommand,
};
use crate::valuation::scarcity::ScarcityEntry;
use crate::valuation::zscore::PlayerValuation;

use layout::{build_layout, AppLayout};

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
// ViewState
// ---------------------------------------------------------------------------

/// TUI-local state that mirrors the application state for rendering.
///
/// Updated incrementally via `UiUpdate` messages from the app orchestrator.
/// The `render_frame` function reads this struct to draw the dashboard.
pub struct ViewState {
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
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
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
        self.pick_number = snapshot.pick_count;
        self.total_picks = snapshot.total_picks;
        if let Some(tab) = snapshot.active_tab {
            self.active_tab = tab;
        }
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
            // Clear previous analysis text when a new nomination arrives
            state.analysis_text.clear();
            state.analysis_status = LlmStatus::Idle;
        }
        UiUpdate::NominationCleared => {
            state.current_nomination = None;
            state.instant_analysis = None;
            state.analysis_text.clear();
            state.analysis_status = LlmStatus::Idle;
        }
        UiUpdate::AnalysisToken(token) => {
            state.analysis_text.push_str(&token);
            state.analysis_status = LlmStatus::Streaming;
        }
        UiUpdate::AnalysisComplete => {
            state.analysis_status = LlmStatus::Complete;
        }
        UiUpdate::PlanToken(token) => {
            state.plan_text.push_str(&token);
            state.plan_status = LlmStatus::Streaming;
        }
        UiUpdate::PlanComplete => {
            state.plan_status = LlmStatus::Complete;
        }
        UiUpdate::ConnectionStatus(status) => {
            state.connection_status = status;
        }
    }
}

// ---------------------------------------------------------------------------
// Render frame (placeholder widgets)
// ---------------------------------------------------------------------------

/// Render the complete dashboard frame.
///
/// Each zone gets a placeholder `Paragraph` widget. Real widget
/// implementations will be added in Task 15.
fn render_frame(frame: &mut Frame, state: &ViewState) {
    let layout = build_layout(frame.area());

    render_status_bar(frame, &layout, state);
    render_nomination_banner(frame, &layout, state);
    render_main_panel(frame, &layout, state);
    render_roster(frame, &layout);
    render_scarcity(frame, &layout);
    render_budget(frame, &layout);
    render_help_bar(frame, &layout);
}

fn render_status_bar(frame: &mut Frame, layout: &AppLayout, state: &ViewState) {
    let conn_str = match state.connection_status {
        ConnectionStatus::Connected => "Connected",
        ConnectionStatus::Disconnected => "Disconnected",
    };
    let text = format!(
        " Pick {}/{} | {} | Tab: {:?}",
        state.pick_number, state.total_picks, conn_str, state.active_tab
    );
    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(text, Style::default().fg(Color::White)),
    ]))
    .style(Style::default().bg(Color::DarkGray));
    frame.render_widget(paragraph, layout.status_bar);
}

fn render_nomination_banner(frame: &mut Frame, layout: &AppLayout, state: &ViewState) {
    let content = if let Some(ref nom) = state.current_nomination {
        let bidder = nom
            .current_bidder
            .as_deref()
            .unwrap_or("--");
        let timer = nom
            .time_remaining
            .map(|t| format!("{}s", t))
            .unwrap_or_else(|| "--".to_string());
        format!(
            "{} ({}) | Bid: ${} by {} | Timer: {} | Nom by: {}",
            nom.player_name, nom.position, nom.current_bid, bidder, timer, nom.nominated_by
        )
    } else {
        "No active nomination".to_string()
    };

    let paragraph = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Nomination"),
    );
    frame.render_widget(paragraph, layout.nomination_banner);
}

fn render_main_panel(frame: &mut Frame, layout: &AppLayout, state: &ViewState) {
    let title = match state.active_tab {
        TabId::Analysis => "Analysis",
        TabId::NomPlan => "Nomination Plan",
        TabId::Available => "Available Players",
        TabId::DraftLog => "Draft Log",
        TabId::Teams => "Teams",
    };

    let content = match state.active_tab {
        TabId::Analysis => {
            if state.analysis_text.is_empty() {
                match state.analysis_status {
                    LlmStatus::Idle => "Waiting for nomination...".to_string(),
                    LlmStatus::Streaming => "Streaming...".to_string(),
                    LlmStatus::Complete => "Analysis complete (empty).".to_string(),
                    LlmStatus::Error => "Analysis error.".to_string(),
                }
            } else {
                state.analysis_text.clone()
            }
        }
        TabId::NomPlan => {
            if state.plan_text.is_empty() {
                "No nomination plan yet.".to_string()
            } else {
                state.plan_text.clone()
            }
        }
        _ => format!("{} (placeholder)", title),
    };

    let paragraph = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title),
    );
    frame.render_widget(paragraph, layout.main_panel);
}

fn render_roster(frame: &mut Frame, layout: &AppLayout) {
    let paragraph = Paragraph::new("Roster (placeholder)").block(
        Block::default()
            .borders(Borders::ALL)
            .title("My Roster"),
    );
    frame.render_widget(paragraph, layout.roster);
}

fn render_scarcity(frame: &mut Frame, layout: &AppLayout) {
    let paragraph = Paragraph::new("Scarcity (placeholder)").block(
        Block::default()
            .borders(Borders::ALL)
            .title("Scarcity"),
    );
    frame.render_widget(paragraph, layout.scarcity);
}

fn render_budget(frame: &mut Frame, layout: &AppLayout) {
    let paragraph = Paragraph::new("Budget (placeholder)").block(
        Block::default()
            .borders(Borders::ALL)
            .title("Budget"),
    );
    frame.render_widget(paragraph, layout.budget);
}

fn render_help_bar(frame: &mut Frame, layout: &AppLayout) {
    let text = " q:Quit | 1-5:Tabs | /:Filter | r:Refresh | ?:Help";
    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(
            text,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::DIM),
        ),
    ]))
    .style(Style::default().bg(Color::DarkGray));
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

    // 3. Create ViewState
    let mut view_state = ViewState::default();

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
                        // Quit on 'q' (when not in filter mode) or Ctrl+C
                        if key_event.code == KeyCode::Char('c')
                            && key_event.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            let _ = cmd_tx.send(UserCommand::Quit).await;
                            break;
                        }
                        if key_event.code == KeyCode::Char('q') && !view_state.filter_mode {
                            let _ = cmd_tx.send(UserCommand::Quit).await;
                            break;
                        }
                        // Delegate to input handler (stub for now, Task 17)
                        input::handle_key(key_event, &mut view_state, &cmd_tx).await;
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

    #[test]
    fn view_state_default_is_sensible() {
        let state = ViewState::default();
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

    #[test]
    fn apply_snapshot_updates_fields() {
        let mut state = ViewState::default();
        let snapshot = AppSnapshot {
            pick_count: 42,
            total_picks: 260,
            active_tab: Some(TabId::DraftLog),
        };
        state.apply_snapshot(snapshot);
        assert_eq!(state.pick_number, 42);
        assert_eq!(state.total_picks, 260);
        assert_eq!(state.active_tab, TabId::DraftLog);
    }

    #[test]
    fn apply_snapshot_preserves_tab_when_none() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        let snapshot = AppSnapshot {
            pick_count: 10,
            total_picks: 260,
            active_tab: None,
        };
        state.apply_snapshot(snapshot);
        assert_eq!(state.pick_number, 10);
        assert_eq!(state.active_tab, TabId::Available);
    }

    #[test]
    fn apply_ui_update_state_snapshot() {
        let mut state = ViewState::default();
        let snapshot = AppSnapshot {
            pick_count: 5,
            total_picks: 100,
            active_tab: Some(TabId::Teams),
        };
        apply_ui_update(&mut state, UiUpdate::StateSnapshot(Box::new(snapshot)));
        assert_eq!(state.pick_number, 5);
        assert_eq!(state.total_picks, 100);
        assert_eq!(state.active_tab, TabId::Teams);
    }

    #[test]
    fn apply_ui_update_nomination_update() {
        let mut state = ViewState::default();
        state.analysis_text = "old analysis".to_string();
        state.analysis_status = LlmStatus::Complete;

        let nom = NominationInfo {
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            nominated_by: "Team Alpha".to_string(),
            current_bid: 45,
            current_bidder: Some("Team Beta".to_string()),
            time_remaining: Some(30),
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
        apply_ui_update(&mut state, UiUpdate::AnalysisComplete);
        assert_eq!(state.analysis_status, LlmStatus::Complete);
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
        apply_ui_update(&mut state, UiUpdate::PlanComplete);
        assert_eq!(state.plan_status, LlmStatus::Complete);
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
}
