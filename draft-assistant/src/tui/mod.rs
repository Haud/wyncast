// TUI dashboard: layout, input handling, and widget rendering.
//
// The TUI owns an `App` root component that holds all TUI state. The app
// orchestrator pushes `UiUpdate` messages over an mpsc channel; the App
// applies them and re-renders at ~30 fps.

pub mod action;
pub mod app;
pub mod confirm_dialog;
pub mod draft;
pub mod layout;
pub mod llm_stream;
pub mod onboarding;
pub mod scroll;
pub mod settings;
pub mod subscription;
pub mod text_input;
pub mod widgets;

use std::time::Duration;

use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::protocol::{AppMode, UiUpdate, UserCommand};
use crate::tui::action::Action;
use crate::tui::app::AppMessage;
use crate::tui::subscription::{AppEvent, SubscriptionManager};
use crate::tui::subscription::keybinding::KeybindManager;

pub use onboarding::llm_setup::LlmSetupState;
pub use onboarding::strategy_setup::StrategySetupState;
pub use text_input::{TextInput, TextInputMessage};

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
/// `None -> MainPanel -> Budget -> Roster -> Scarcity -> NominationPlan -> None`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel {
    /// The active tab's content area (left side).
    MainPanel,
    /// Sidebar: My Roster panel.
    Roster,
    /// Sidebar: Positional Scarcity panel.
    Scarcity,
    /// Left column bottom: Budget panel.
    Budget,
    /// Sidebar: Nomination Plan panel.
    NominationPlan,
}

impl FocusPanel {
    /// Ordered list of panels for cycling.
    const CYCLE: &[FocusPanel] = &[
        FocusPanel::MainPanel,
        FocusPanel::Budget,
        FocusPanel::Roster,
        FocusPanel::Scarcity,
        FocusPanel::NominationPlan,
    ];

    /// Advance focus forward:
    /// None -> MainPanel -> Budget -> Roster -> Scarcity -> NominationPlan -> None
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
    /// None -> NominationPlan -> Scarcity -> Roster -> Budget -> MainPanel -> None
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

// Re-export the canonical KeybindHint from the subscription system.
pub use subscription::keybinding::KeybindHint;

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

// Re-exports from draft modal layer.
pub use draft::modal::ModalLayer;
pub use draft::modal::position_filter::PositionFilterModal;

// ---------------------------------------------------------------------------
// Help bar rendering
// ---------------------------------------------------------------------------

/// Render the help bar using the pre-computed keybind hints.
///
/// This function is a dumb renderer: it knows nothing about modes, tabs, or
/// focus. All context-sensitivity lives in [`app::App::compute_keybinds`].
/// The special case for filter mode (showing an inline input bar) is still
/// handled here because it requires displaying live state data (the current
/// filter text and cursor), not just static hint labels.
///
/// For onboarding/settings modes, `filter_text` and `filter_mode` are
/// `None`/`false` since the filter UI only exists in draft mode.
pub(crate) fn render_help_bar(
    frame: &mut Frame,
    area: Rect,
    app: &app::App,
    keybinds: &[KeybindHint],
) {
    if app.draft_screen.main_panel.available.filter_mode() {
        let filter = app.draft_screen.main_panel.available.filter_text();
        let text_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
        let cursor_style = Style::default().fg(Color::Cyan);
        let selection_style = Style::default().fg(Color::White).bg(Color::DarkGray).add_modifier(Modifier::BOLD);

        let mut spans = vec![
            Span::styled(
                " FILTER ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
        ];
        spans.extend(filter.styled_spans(text_style, cursor_style, selection_style));
        spans.push(Span::styled(
            "  (Enter:apply | Esc:cancel)",
            Style::default().fg(Color::DarkGray),
        ));
        let paragraph = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
        return;
    }

    render_keybind_hints(frame, area, keybinds);
}

/// Render the help bar for draft mode.
///
/// When filter mode is active, shows an inline filter input bar with the
/// current filter text. Otherwise, renders the standard keybind hint row.
pub(crate) fn render_help_bar_draft(
    frame: &mut Frame,
    area: Rect,
    filter_mode: bool,
    filter_input: &TextInput,
    keybinds: &[KeybindHint],
) {
    if filter_mode {
        let text_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
        let cursor_style = Style::default().fg(Color::Cyan);
        let selection_style = Style::default().fg(Color::White).bg(Color::DarkGray).add_modifier(Modifier::BOLD);

        let mut spans = vec![
            Span::styled(
                " FILTER ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
        ];
        spans.extend(filter_input.styled_spans(text_style, cursor_style, selection_style));
        spans.push(Span::styled(
            "  (Enter:apply | Esc:cancel)",
            Style::default().fg(Color::DarkGray),
        ));
        let paragraph = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
        return;
    }

    render_keybind_hints(frame, area, keybinds);
}

/// Render a list of keybind hints into the help bar area.
fn render_keybind_hints(frame: &mut Frame, area: Rect, keybinds: &[KeybindHint]) {
    let mut spans: Vec<Span> = Vec::new();
    for (i, hint) in keybinds.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        }
        let text = if hint.key.is_empty() {
            format!(" {}", hint.description)
        } else {
            format!(" {}:{}", hint.key, hint.description)
        };
        spans.push(Span::styled(text, Style::default().fg(Color::Gray)));
    }

    let paragraph = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, area);
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

    // 3. Create App with the initial app mode so the first frame renders the
    //    correct screen (avoids a flash of the draft UI when the app starts
    //    in onboarding mode).
    let mut app = app::App::new(initial_mode);

    // 4. Create crossterm EventStream for async keyboard input
    let mut event_stream = EventStream::new();

    // 5. Create render interval (~30fps)
    let mut render_tick = tokio::time::interval(Duration::from_millis(33));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // 6. Create subscription manager and keybind manager for the new input system.
    let mut sub_manager = SubscriptionManager::<AppMessage>::new();
    let mut kb_manager = KeybindManager::new();

    // 7. Main loop (game-loop pattern)
    //
    // Instead of giving `ui_rx.recv()` its own select branch (which causes
    // render-tick starvation when many updates queue up, e.g. LLM streaming
    // tokens), we drain ALL pending UI updates inside the render-tick branch
    // via `try_recv()`. This batches updates between frames and guarantees
    // the render tick is never starved.
    loop {
        tokio::select! {
            // Keyboard input - handle immediately (low volume, latency-sensitive)
            maybe_event = event_stream.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key_event))) => {
                        if let Some(msg) = sub_manager.process(&AppEvent::Key(key_event)) {
                            if let Some(action) = app.update(msg) {
                                match action {
                                    Action::Quit => {
                                        let _ = cmd_tx.send(UserCommand::Quit).await;
                                        break;
                                    }
                                    Action::Command(cmd) => {
                                        let _ = cmd_tx.send(cmd).await;
                                    }
                                }
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

            // Render tick - drain all pending UI updates, then render
            _ = render_tick.tick() => {
                // Drain all pending UI updates (game-loop batching).
                loop {
                    match ui_rx.try_recv() {
                        Ok(ui_update) => app.apply_update(ui_update),
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            // Channel closed: app is shutting down.
                            // Restore terminal before returning.
                            ratatui::restore();
                            return Ok(());
                        }
                    }
                }

                // Fire a Tick event for timer subscriptions before rebuilding.
                // This allows TimerRecipe listeners to fire on the render cadence.
                let now = std::time::Instant::now();
                if let Some(msg) = sub_manager.process(&AppEvent::Tick(now)) {
                    if let Some(action) = app.update(msg) {
                        match action {
                            Action::Quit => {
                                let _ = cmd_tx.send(UserCommand::Quit).await;
                                break;
                            }
                            Action::Command(cmd) => {
                                let _ = cmd_tx.send(cmd).await;
                            }
                        }
                    }
                }

                // Clear and rebuild hint registry + sync subscriptions.
                kb_manager.clear();
                let sub = app.subscription(&mut kb_manager);
                sub_manager.sync(sub);

                // Draw using hints from kb_manager.
                app.active_keybinds = kb_manager.hints();
                terminal.draw(|frame| app.view(frame))?;
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
    use crate::protocol::{
        AppMode, ConnectionStatus, LlmStatus, LlmStreamUpdate, NominationInfo, TabId, TeamSnapshot,
    };
    use draft::main_panel::analysis::AnalysisPanelMessage;
    use draft::main_panel::MainPanelMessage;
    use draft::sidebar::plan::PlanPanelMessage;
    use llm_stream::LlmStreamMessage;

    // -- FocusPanel cycling --

    #[test]
    fn focus_next_cycles_forward() {
        assert_eq!(FocusPanel::next(None), Some(FocusPanel::MainPanel));
        assert_eq!(FocusPanel::next(Some(FocusPanel::MainPanel)), Some(FocusPanel::Budget));
        assert_eq!(FocusPanel::next(Some(FocusPanel::Budget)), Some(FocusPanel::Roster));
        assert_eq!(FocusPanel::next(Some(FocusPanel::Roster)), Some(FocusPanel::Scarcity));
        assert_eq!(FocusPanel::next(Some(FocusPanel::Scarcity)), Some(FocusPanel::NominationPlan));
        assert_eq!(FocusPanel::next(Some(FocusPanel::NominationPlan)), None);
    }

    #[test]
    fn focus_prev_cycles_backward() {
        assert_eq!(FocusPanel::prev(None), Some(FocusPanel::NominationPlan));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::NominationPlan)), Some(FocusPanel::Scarcity));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::Scarcity)), Some(FocusPanel::Roster));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::Roster)), Some(FocusPanel::Budget));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::Budget)), Some(FocusPanel::MainPanel));
        assert_eq!(FocusPanel::prev(Some(FocusPanel::MainPanel)), None);
    }

    #[test]
    fn focus_next_then_prev_is_identity() {
        let step1 = FocusPanel::next(None);
        let step2 = FocusPanel::prev(step1);
        assert_eq!(step2, None);
    }

    #[test]
    fn app_default_is_sensible() {
        let app = app::App::default();
        assert_eq!(app.app_mode, AppMode::Draft);
        assert!(app.draft_screen.current_nomination.is_none());
        assert!(app.draft_screen.instant_analysis.is_none());
        assert!(app.draft_screen.available_players.is_empty());
        assert!(app.draft_screen.positional_scarcity.is_empty());
        assert_eq!(app.draft_screen.pick_number, 0);
        assert_eq!(app.draft_screen.total_picks, 0);
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Analysis);
        assert_eq!(app.draft_screen.connection_status, ConnectionStatus::Disconnected);
        assert_eq!(app.draft_screen.main_panel.analysis.status(), LlmStatus::Idle);
        assert_eq!(app.draft_screen.sidebar.plan.status(), LlmStatus::Idle);
        assert!(app.draft_screen.main_panel.analysis.text().is_empty());
        assert!(app.draft_screen.sidebar.plan.text().is_empty());
        assert!(app.draft_screen.scroll_offset.is_empty());
        assert!(!app.draft_screen.main_panel.available.filter_mode());
        assert!(app.draft_screen.main_panel.available.filter_text().is_empty());
        assert!(app.draft_screen.main_panel.available.position_filter().is_none());
        assert!(!app.draft_screen.modal_layer.quit_confirm.open);
        assert!(app.draft_screen.draft_log.is_empty());
        assert!(app.draft_screen.team_summaries.is_empty());
        assert!(app.draft_screen.my_roster.is_empty());
        assert!(app.draft_screen.focused_panel.is_none());
        assert!(!app.draft_screen.modal_layer.position_filter.open);
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
    fn test_snapshot(pick_count: usize, total_picks: usize, active_tab: Option<TabId>) -> crate::protocol::AppSnapshot {
        crate::protocol::AppSnapshot {
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
            llm_configured: true,
        }
    }

    #[test]
    fn apply_snapshot_updates_fields() {
        let mut app = app::App::default();
        let snapshot = test_snapshot(42, 260, Some(TabId::Teams));
        app.apply_snapshot(snapshot);
        assert_eq!(app.draft_screen.pick_number, 42);
        assert_eq!(app.draft_screen.total_picks, 260);
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Teams);
    }

    #[test]
    fn apply_snapshot_preserves_tab_when_none() {
        let mut app = app::App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        let snapshot = test_snapshot(10, 260, None);
        app.apply_snapshot(snapshot);
        assert_eq!(app.draft_screen.pick_number, 10);
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Available);
    }

    #[test]
    fn apply_update_state_snapshot() {
        let mut app = app::App::default();
        let snapshot = test_snapshot(5, 100, Some(TabId::DraftLog));
        app.apply_update(UiUpdate::StateSnapshot(Box::new(snapshot)));
        assert_eq!(app.draft_screen.pick_number, 5);
        assert_eq!(app.draft_screen.total_picks, 100);
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::DraftLog);
    }

    #[test]
    fn apply_snapshot_updates_budget_and_teams() {
        let mut app = app::App::default();
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

        app.apply_snapshot(snapshot);

        assert_eq!(app.draft_screen.budget.spent, 100);
        assert_eq!(app.draft_screen.budget.remaining, 160);
        assert!((app.draft_screen.budget.inflation_rate - 1.15).abs() < f64::EPSILON);
        assert_eq!(app.draft_screen.budget.max_bid, 140);
        assert!((app.draft_screen.inflation - 1.15).abs() < f64::EPSILON);
        assert_eq!(app.draft_screen.team_summaries.len(), 2);
        assert_eq!(app.draft_screen.team_summaries[0].name, "Team 1");
        assert_eq!(app.draft_screen.team_summaries[0].budget_remaining, 160);
        assert_eq!(app.draft_screen.team_summaries[0].slots_filled, 5);
        assert_eq!(app.draft_screen.team_summaries[1].name, "Team 2");
        assert_eq!(app.draft_screen.team_summaries[1].budget_remaining, 200);
    }

    #[test]
    fn apply_update_nomination_update() {
        use crate::protocol::{InstantAnalysis, InstantVerdict};

        let mut app = app::App::default();
        app.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(
            LlmStreamMessage::Complete("old analysis".into()),
        ));
        app.draft_screen.instant_analysis = Some(InstantAnalysis {
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
        app.apply_update(UiUpdate::NominationUpdate { info: Box::new(nom), analysis_request_id: None });

        assert!(app.draft_screen.current_nomination.is_some());
        assert_eq!(
            app.draft_screen.current_nomination.as_ref().unwrap().player_name,
            "Mike Trout"
        );
        assert!(app.draft_screen.main_panel.analysis.text().is_empty());
        assert_eq!(app.draft_screen.main_panel.analysis.status(), LlmStatus::Idle);
        assert!(app.draft_screen.instant_analysis.is_none());
    }

    #[test]
    fn apply_update_bid_update_preserves_analysis_text() {
        let mut app = app::App::default();
        app.draft_screen.current_nomination = Some(NominationInfo {
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            nominated_by: "Team Alpha".to_string(),
            current_bid: 45,
            current_bidder: Some("Team Beta".to_string()),
            time_remaining: Some(30),
            eligible_slots: vec![],
        });
        app.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("Trout is a strong target because...".into()),
        ));

        let updated_nom = NominationInfo {
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            nominated_by: "Team Alpha".to_string(),
            current_bid: 50,
            current_bidder: Some("Team Gamma".to_string()),
            time_remaining: Some(25),
            eligible_slots: vec![],
        };
        app.apply_update(UiUpdate::BidUpdate(Box::new(updated_nom)));

        let nom = app.draft_screen.current_nomination.as_ref().unwrap();
        assert_eq!(nom.current_bid, 50);
        assert_eq!(nom.current_bidder, Some("Team Gamma".to_string()));
        assert_eq!(app.draft_screen.main_panel.analysis.text(), "Trout is a strong target because...");
        assert_eq!(app.draft_screen.main_panel.analysis.status(), LlmStatus::Streaming);
    }

    #[test]
    fn apply_update_nomination_cleared() {
        let mut app = app::App::default();
        app.draft_screen.current_nomination = Some(NominationInfo {
            player_name: "Test".to_string(),
            position: "SP".to_string(),
            nominated_by: "Team".to_string(),
            current_bid: 10,
            current_bidder: None,
            time_remaining: None,
            eligible_slots: vec![],
        });
        app.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("some analysis".into()),
        ));

        app.apply_update(UiUpdate::NominationCleared);

        assert!(app.draft_screen.current_nomination.is_none());
        assert!(app.draft_screen.instant_analysis.is_none());
        assert!(app.draft_screen.main_panel.analysis.text().is_empty());
        assert_eq!(app.draft_screen.main_panel.analysis.status(), LlmStatus::Idle);
    }

    #[test]
    fn apply_update_analysis_token() {
        let mut app = app::App::default();
        app.draft_screen.analysis_request_id = Some(1);
        app.apply_update(UiUpdate::LlmUpdate { request_id: 1, update: LlmStreamUpdate::Token("Hello ".to_string()) });
        app.apply_update(UiUpdate::LlmUpdate { request_id: 1, update: LlmStreamUpdate::Token("World".to_string()) });
        assert_eq!(app.draft_screen.main_panel.analysis.text(), "Hello World");
        assert_eq!(app.draft_screen.main_panel.analysis.status(), LlmStatus::Streaming);
    }

    #[test]
    fn apply_update_analysis_complete() {
        let mut app = app::App::default();
        app.draft_screen.analysis_request_id = Some(1);
        app.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("partial token".into()),
        ));
        app.apply_update(UiUpdate::LlmUpdate { request_id: 1, update: LlmStreamUpdate::Complete("Full analysis text.".to_string()) });
        assert_eq!(app.draft_screen.main_panel.analysis.status(), LlmStatus::Complete);
        assert_eq!(app.draft_screen.main_panel.analysis.text(), "Full analysis text.");
    }

    #[test]
    fn apply_update_plan_started_clears_previous_text() {
        let mut app = app::App::default();
        app.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
            LlmStreamMessage::Complete("Old plan from last pick cycle.".into()),
        ));

        app.apply_update(UiUpdate::PlanStarted { request_id: 1 });

        assert!(app.draft_screen.sidebar.plan.text().is_empty(), "plan text should be cleared on PlanStarted");
        assert_eq!(app.draft_screen.sidebar.plan.status(), LlmStatus::Streaming);
        assert_eq!(app.draft_screen.plan_request_id, Some(1));
    }

    #[test]
    fn apply_update_plan_started_then_tokens_replace_not_append() {
        let mut app = app::App::default();
        app.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
            LlmStreamMessage::Complete("Stale plan text.".into()),
        ));

        app.apply_update(UiUpdate::PlanStarted { request_id: 1 });
        app.apply_update(UiUpdate::LlmUpdate { request_id: 1, update: LlmStreamUpdate::Token("New plan: ".to_string()) });
        app.apply_update(UiUpdate::LlmUpdate { request_id: 1, update: LlmStreamUpdate::Token("nominate X".to_string()) });

        assert_eq!(app.draft_screen.sidebar.plan.text(), "New plan: nominate X");
        assert_eq!(app.draft_screen.sidebar.plan.status(), LlmStatus::Streaming);
    }

    #[test]
    fn apply_update_plan_token() {
        let mut app = app::App::default();
        app.draft_screen.plan_request_id = Some(1);
        app.apply_update(UiUpdate::LlmUpdate { request_id: 1, update: LlmStreamUpdate::Token("Plan: ".to_string()) });
        app.apply_update(UiUpdate::LlmUpdate { request_id: 1, update: LlmStreamUpdate::Token("nominate X".to_string()) });
        assert_eq!(app.draft_screen.sidebar.plan.text(), "Plan: nominate X");
        assert_eq!(app.draft_screen.sidebar.plan.status(), LlmStatus::Streaming);
    }

    #[test]
    fn apply_update_plan_complete() {
        let mut app = app::App::default();
        app.draft_screen.plan_request_id = Some(1);
        app.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("partial token".into()),
        ));
        app.apply_update(UiUpdate::LlmUpdate { request_id: 1, update: LlmStreamUpdate::Complete("Full plan text.".to_string()) });
        assert_eq!(app.draft_screen.sidebar.plan.status(), LlmStatus::Complete);
        assert_eq!(app.draft_screen.sidebar.plan.text(), "Full plan text.");
    }

    #[test]
    fn apply_update_connection_status() {
        let mut app = app::App::default();
        assert_eq!(app.draft_screen.connection_status, ConnectionStatus::Disconnected);
        app.apply_update(UiUpdate::ConnectionStatus(ConnectionStatus::Connected));
        assert_eq!(app.draft_screen.connection_status, ConnectionStatus::Connected);
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

    // -- AppMode in App --

    #[test]
    fn apply_snapshot_updates_app_mode() {
        use crate::onboarding::OnboardingStep;

        let mut app = app::App::default();
        assert_eq!(app.app_mode, AppMode::Draft);

        let mut snapshot = test_snapshot(0, 0, None);
        snapshot.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        app.apply_snapshot(snapshot);
        assert_eq!(app.app_mode, AppMode::Onboarding(OnboardingStep::StrategySetup));
    }

    #[test]
    fn apply_snapshot_updates_llm_configured() {
        let mut app = app::App::default();
        assert!(app.draft_screen.llm_configured);

        let mut snapshot = test_snapshot(0, 0, None);
        snapshot.llm_configured = false;
        app.apply_snapshot(snapshot);
        assert!(!app.draft_screen.llm_configured);

        let mut snapshot2 = test_snapshot(0, 0, None);
        snapshot2.llm_configured = true;
        app.apply_snapshot(snapshot2);
        assert!(app.draft_screen.llm_configured);
    }

    #[test]
    fn apply_update_mode_changed() {
        use crate::onboarding::OnboardingStep;

        let mut app = app::App::default();
        assert_eq!(app.app_mode, AppMode::Draft);

        app.apply_update(UiUpdate::ModeChanged(AppMode::Onboarding(OnboardingStep::LlmSetup)));
        assert_eq!(app.app_mode, AppMode::Onboarding(OnboardingStep::LlmSetup));
    }

    #[test]
    fn apply_update_mode_changed_to_draft() {
        use crate::onboarding::OnboardingStep;

        let mut app = app::App::default();
        app.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);

        app.apply_update(UiUpdate::ModeChanged(AppMode::Draft));
        assert_eq!(app.app_mode, AppMode::Draft);
    }

    #[test]
    fn apply_update_mode_changed_resets_confirm_exit_settings() {
        let mut app = app::App::default();
        app.confirm_exit_settings.open = true;

        app.apply_update(UiUpdate::ModeChanged(AppMode::Draft));
        assert!(
            !app.confirm_exit_settings.open,
            "ModeChanged should reset confirm_exit_settings to false"
        );
    }

    // -- OnboardingUpdate::Strategy* variants --

    #[test]
    fn apply_update_strategy_llm_token() {
        use crate::protocol::OnboardingUpdate;

        let mut app = app::App::default();
        assert!(app.strategy_setup.generation_output.is_empty());

        app.apply_update(UiUpdate::OnboardingUpdate(OnboardingUpdate::StrategyLlmToken("Hello ".to_string())));
        assert_eq!(app.strategy_setup.generation_output, "Hello ");

        app.apply_update(UiUpdate::OnboardingUpdate(OnboardingUpdate::StrategyLlmToken("World".to_string())));
        assert_eq!(app.strategy_setup.generation_output, "Hello World");
    }

    #[test]
    fn apply_update_strategy_llm_complete() {
        use crate::protocol::OnboardingUpdate;
        use crate::tui::onboarding::strategy_setup::CategoryWeights;

        let mut app = app::App::default();
        app.strategy_setup.generating = true;
        app.strategy_setup.generation_error = Some("old error".to_string());

        let weights = CategoryWeights {
            r: 1.0, hr: 1.1, rbi: 1.0, bb: 1.3, sb: 1.0, avg: 1.0,
            k: 1.0, w: 1.0, sv: 0.3, hd: 1.2, era: 1.0, whip: 1.0,
        };

        app.apply_update(UiUpdate::OnboardingUpdate(OnboardingUpdate::StrategyLlmComplete {
            hitting_budget_pct: 70,
            category_weights: weights.clone(),
            strategy_overview: "Focus on elite hitters with high walk rates.".to_string(),
        }));

        assert!(!app.strategy_setup.generating);
        assert!(app.strategy_setup.generation_error.is_none());
        assert_eq!(app.strategy_setup.hitting_budget_pct, 70);
        assert!((app.strategy_setup.category_weights.bb - 1.3).abs() < f32::EPSILON);
        assert!((app.strategy_setup.category_weights.sv - 0.3).abs() < f32::EPSILON);
        assert!(!app.strategy_setup.input_editing);
    }

    /// When entering Settings -> StrategyConfig, the StrategyLlmComplete event
    /// lands the user on the Review step with input_editing = false so that
    /// arrow keys navigate instead of being captured by the text input.
    #[test]
    fn strategy_llm_complete_deactivates_input_for_settings() {
        use crate::protocol::OnboardingUpdate;
        use crate::tui::onboarding::strategy_setup::CategoryWeights;

        let mut app = app::App::default();
        assert!(app.strategy_setup.input_editing);

        let weights = CategoryWeights::default();
        app.apply_update(UiUpdate::OnboardingUpdate(OnboardingUpdate::StrategyLlmComplete {
            hitting_budget_pct: 65,
            category_weights: weights,
            strategy_overview: "Test overview".to_string(),
        }));

        assert!(!app.strategy_setup.input_editing);
        assert_eq!(
            app.strategy_setup.step,
            crate::tui::onboarding::strategy_setup::StrategyWizardStep::Review,
        );
        assert_eq!(
            app.strategy_setup.review_section,
            crate::tui::onboarding::strategy_setup::ReviewSection::Overview,
        );
    }

    #[test]
    fn apply_update_strategy_llm_error() {
        use crate::protocol::OnboardingUpdate;

        let mut app = app::App::default();
        app.strategy_setup.generating = true;

        app.apply_update(UiUpdate::OnboardingUpdate(OnboardingUpdate::StrategyLlmError(
            "API rate limit exceeded".to_string(),
        )));

        assert!(!app.strategy_setup.generating);
        assert_eq!(
            app.strategy_setup.generation_error.as_deref(),
            Some("API rate limit exceeded")
        );
    }
}
