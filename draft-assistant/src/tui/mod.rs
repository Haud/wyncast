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

use draft::DraftScreen;
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
/// The active set of hints is stored in [`app::App::active_keybinds`],
/// recomputed on every render frame by [`app::App::compute_keybinds`]. The
/// help bar is a dumb renderer that displays whatever hints are present there.
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
                app.draft_screen.main_panel.available.filter_text().value().to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("\u{258e}", Style::default().fg(Color::Cyan)),
            Span::styled(
                "  (Enter:apply | Esc:cancel)",
                Style::default().fg(Color::DarkGray),
            ),
        ];
        let paragraph = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
        return;
    }

    render_keybind_hints(frame, area, keybinds);
}

/// Render the help bar from within the DraftScreen component.
///
/// This is a draft-mode-specific variant that takes a DraftScreen reference
/// instead of an App reference, used by `DraftScreen::view()`.
pub(crate) fn render_help_bar_from_draft(
    frame: &mut Frame,
    area: Rect,
    draft_screen: &DraftScreen,
    keybinds: &[KeybindHint],
) {
    if draft_screen.main_panel.available.filter_mode() {
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
                draft_screen.main_panel.available.filter_text().value().to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("\u{258e}", Style::default().fg(Color::Cyan)),
            Span::styled(
                "  (Enter:apply | Esc:cancel)",
                Style::default().fg(Color::DarkGray),
            ),
        ];
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

    // 6. Main loop
    loop {
        tokio::select! {
            // UI updates from the app orchestrator
            update = ui_rx.recv() => {
                match update {
                    Some(ui_update) => {
                        app.apply_update(ui_update);
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
                        if let Some(cmd) = app.handle_key(key_event) {
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
                app.active_keybinds = app.compute_keybinds();
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
        AppMode, ConnectionStatus, LlmStatus, NominationInfo, TabId, TeamSnapshot,
    };
    use crossterm::event::KeyCode;
    use draft::main_panel::analysis::AnalysisPanelMessage;
    use draft::main_panel::available::AvailablePanelMessage;
    use draft::main_panel::MainPanelMessage;
    use draft::sidebar::plan::PlanPanelMessage;
    use llm_stream::LlmStreamMessage;

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
        app.apply_update(UiUpdate::NominationUpdate(Box::new(nom)));

        assert!(app.draft_screen.current_nomination.is_some());
        assert_eq!(
            app.draft_screen.current_nomination.as_ref().unwrap().player_name,
            "Mike Trout"
        );
        assert!(app.draft_screen.main_panel.analysis.text().is_empty());
        assert_eq!(app.draft_screen.main_panel.analysis.status(), LlmStatus::Idle);
        assert!(app.draft_screen.instant_analysis.is_none());
        assert_eq!(app.draft_screen.main_panel.available.scroll_offset(), 0);
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
        app.apply_update(UiUpdate::AnalysisToken("Hello ".to_string()));
        app.apply_update(UiUpdate::AnalysisToken("World".to_string()));
        assert_eq!(app.draft_screen.main_panel.analysis.text(), "Hello World");
        assert_eq!(app.draft_screen.main_panel.analysis.status(), LlmStatus::Streaming);
    }

    #[test]
    fn apply_update_analysis_complete() {
        let mut app = app::App::default();
        app.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("partial token".into()),
        ));
        app.apply_update(UiUpdate::AnalysisComplete("Full analysis text.".to_string()));
        assert_eq!(app.draft_screen.main_panel.analysis.status(), LlmStatus::Complete);
        assert_eq!(app.draft_screen.main_panel.analysis.text(), "Full analysis text.");
    }

    #[test]
    fn apply_update_plan_started_clears_previous_text() {
        let mut app = app::App::default();
        app.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
            LlmStreamMessage::Complete("Old plan from last pick cycle.".into()),
        ));

        app.apply_update(UiUpdate::PlanStarted);

        assert!(app.draft_screen.sidebar.plan.text().is_empty(), "plan text should be cleared on PlanStarted");
        assert_eq!(app.draft_screen.sidebar.plan.status(), LlmStatus::Streaming);
    }

    #[test]
    fn apply_update_plan_started_then_tokens_replace_not_append() {
        let mut app = app::App::default();
        app.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
            LlmStreamMessage::Complete("Stale plan text.".into()),
        ));

        app.apply_update(UiUpdate::PlanStarted);
        app.apply_update(UiUpdate::PlanToken("New plan: ".to_string()));
        app.apply_update(UiUpdate::PlanToken("nominate X".to_string()));

        assert_eq!(app.draft_screen.sidebar.plan.text(), "New plan: nominate X");
        assert_eq!(app.draft_screen.sidebar.plan.status(), LlmStatus::Streaming);
    }

    #[test]
    fn apply_update_plan_token() {
        let mut app = app::App::default();
        app.apply_update(UiUpdate::PlanToken("Plan: ".to_string()));
        app.apply_update(UiUpdate::PlanToken("nominate X".to_string()));
        assert_eq!(app.draft_screen.sidebar.plan.text(), "Plan: nominate X");
        assert_eq!(app.draft_screen.sidebar.plan.status(), LlmStatus::Streaming);
    }

    #[test]
    fn apply_update_plan_complete() {
        let mut app = app::App::default();
        app.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("partial token".into()),
        ));
        app.apply_update(UiUpdate::PlanComplete("Full plan text.".to_string()));
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

    // -- compute_keybinds --

    /// Helper: extract all key labels from a hint list.
    fn keys(hints: &[KeybindHint]) -> Vec<&str> {
        hints.iter().map(|h| h.key.as_str()).collect()
    }

    #[test]
    fn compute_keybinds_normal_mode_base_hints_present() {
        let app = app::App::default();
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"q"), "should contain quit hint");
        assert!(ks.contains(&"1-4"), "should contain tab-switch hint");
        assert!(ks.contains(&"Tab"), "should contain focus hint");
        assert!(ks.contains(&"r"), "should contain resync hint");
    }

    #[test]
    fn compute_keybinds_no_scroll_hint_without_focus() {
        let mut app = app::App::default();
        app.draft_screen.focused_panel = None;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(
            !ks.contains(&"\u{2191}\u{2193}/j/k/PgUp/PgDn"),
            "scroll hint should not appear without focus"
        );
    }

    #[test]
    fn compute_keybinds_scroll_hint_with_focus() {
        let mut app = app::App::default();
        app.draft_screen.focused_panel = Some(FocusPanel::MainPanel);
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(
            ks.contains(&"\u{2191}\u{2193}/j/k/PgUp/PgDn"),
            "scroll hint should appear when a panel is focused"
        );
    }

    #[test]
    fn compute_keybinds_filter_hints_on_available_tab() {
        let mut app = app::App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"/"), "filter hint should appear on Available tab");
        assert!(ks.contains(&"p"), "pos filter hint should appear on Available tab");
    }

    #[test]
    fn compute_keybinds_no_filter_hints_on_analysis_tab() {
        let mut app = app::App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Analysis));
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(
            !ks.contains(&"/"),
            "filter hint should not appear on Analysis tab"
        );
    }

    #[test]
    fn compute_keybinds_filter_mode() {
        let mut app = app::App::default();
        app.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"Enter"), "filter mode should show Enter hint");
        assert!(ks.contains(&"Esc"), "filter mode should show Esc hint");
        assert!(!ks.contains(&"q"), "normal quit hint should not appear in filter mode");
        assert!(!ks.contains(&"1-4"), "tab hint should not appear in filter mode");
    }

    #[test]
    fn compute_keybinds_position_modal_open() {
        let mut app = app::App::default();
        app.draft_screen.modal_layer.position_filter.open = true;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"\u{2191}\u{2193}"), "modal should show navigate hint");
        assert!(ks.contains(&"Enter"), "modal should show select hint");
        assert!(ks.contains(&"Esc"), "modal should show cancel hint");
        assert!(!ks.contains(&"q"), "quit hint should not appear when modal is open");
    }

    #[test]
    fn compute_keybinds_quit_confirm_mode() {
        let mut app = app::App::default();
        app.draft_screen.modal_layer.quit_confirm.open = true;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"y/q"), "confirm quit hint should appear");
        assert!(ks.contains(&"n/Esc"), "cancel hint should appear");
        assert!(!ks.contains(&"1-4"), "tab hint should not appear in confirm mode");
    }

    #[test]
    fn compute_keybinds_active_filter_reminder_on_available_tab() {
        let mut app = app::App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        for ch in "trout".chars() {
            app.draft_screen.main_panel.available.update(AvailablePanelMessage::FilterKeyPress(
                crossterm::event::KeyEvent {
                    code: KeyCode::Char(ch),
                    modifiers: crossterm::event::KeyModifiers::NONE,
                    kind: crossterm::event::KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                },
            ));
        }
        let hints = app.compute_keybinds();
        let has_reminder = hints.iter().any(|h| h.key.contains("trout"));
        assert!(has_reminder, "should show filter reminder hint with filter text");
    }

    #[test]
    fn compute_keybinds_no_filter_reminder_on_analysis_tab() {
        let mut app = app::App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Analysis));
        for ch in "trout".chars() {
            app.draft_screen.main_panel.available.update(AvailablePanelMessage::FilterKeyPress(
                crossterm::event::KeyEvent {
                    code: KeyCode::Char(ch),
                    modifiers: crossterm::event::KeyModifiers::NONE,
                    kind: crossterm::event::KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                },
            ));
        }
        let hints = app.compute_keybinds();
        let has_reminder = hints.iter().any(|h| h.key.contains("trout"));
        assert!(
            !has_reminder,
            "filter reminder should not appear on Analysis tab"
        );
    }

    #[test]
    fn app_default_active_keybinds_empty() {
        let app = app::App::default();
        assert!(
            app.active_keybinds.is_empty(),
            "active_keybinds should start empty before first render"
        );
    }

    #[test]
    fn quit_confirm_takes_priority_over_modal_and_filter_mode() {
        let mut app = app::App::default();
        app.draft_screen.modal_layer.quit_confirm.open = true;
        app.draft_screen.modal_layer.position_filter.open = true;
        app.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"y/q"), "quit confirm should take highest priority");
        assert!(!ks.contains(&"\u{2191}\u{2193}"), "modal nav hint should not appear");
        assert_eq!(hints.len(), 2, "only 2 quit-confirm hints should be present");
    }

    // -- AppMode-aware keybind computation --

    #[test]
    fn compute_keybinds_llm_setup_normal_mode() {
        use crate::onboarding::OnboardingStep;
        use onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut app = app::App::default();
        app.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"^v"), "LLM setup should show select hint");
        assert!(ks.contains(&"Enter"), "LLM setup should show confirm hint");
        assert!(ks.contains(&"s"), "LLM setup should show skip hint");
        assert!(!ks.contains(&"Esc"), "Esc should not appear on first section (Provider)");
        assert!(!ks.contains(&"n"), "n should not appear until connection tested");
        assert!(!ks.contains(&"1-4"), "tab hints should not appear in onboarding");

        app.llm_setup.confirmed_through = Some(LlmSetupSection::Provider);
        app.llm_setup.active_section = LlmSetupSection::Model;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"Esc"), "Esc should appear when not on first section");

        app.llm_setup.connection_status = LlmConnectionStatus::Success("ok".to_string());
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"n"), "n should appear after successful connection test");
    }

    #[test]
    fn compute_keybinds_llm_setup_editing_mode() {
        use crate::onboarding::OnboardingStep;

        let mut app = app::App::default();
        app.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        app.llm_setup.api_key_editing = true;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"Enter"), "editing should show confirm hint");
        assert!(ks.contains(&"Esc"), "editing should show cancel hint");
        assert!(!ks.contains(&"n"), "editing should not show Next hint");
    }

    #[test]
    fn compute_keybinds_strategy_setup_input_editing() {
        use crate::onboarding::OnboardingStep;

        let mut app = app::App::default();
        app.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"Enter"), "input editing should show Generate hint");
        assert!(ks.contains(&"Esc"), "input editing should show stop editing hint");
    }

    #[test]
    fn compute_keybinds_strategy_setup_review() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::strategy_setup::StrategyWizardStep;

        let mut app = app::App::default();
        app.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        app.strategy_setup.step = StrategyWizardStep::Review;
        app.strategy_setup.input_editing = false;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"s"), "review should show Save hint");
        assert!(ks.contains(&"Esc"), "review should show Back hint");
    }

    #[test]
    fn compute_keybinds_strategy_setup_editing() {
        use crate::onboarding::OnboardingStep;

        let mut app = app::App::default();
        app.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        app.strategy_setup.editing_field = Some("budget".to_string());
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"Enter"), "editing should show confirm hint");
        assert!(ks.contains(&"Esc"), "editing should show cancel hint");
        assert!(!ks.contains(&"s"), "editing should not show save hint");
    }

    #[test]
    fn compute_keybinds_strategy_setup_ai_editing() {
        use crate::onboarding::OnboardingStep;

        let mut app = app::App::default();
        app.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        app.strategy_setup.input_editing = true;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"Enter"), "ai editing should show confirm hint");
        assert!(ks.contains(&"Esc"), "ai editing should show cancel hint");
        assert!(!ks.contains(&"s"), "ai editing should not show save hint");
    }

    #[test]
    fn compute_keybinds_settings_mode() {
        use crate::protocol::SettingsSection;

        let mut app = app::App::default();
        app.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        app.settings_tab = SettingsSection::LlmConfig;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"Esc"), "settings should show Back hint");
        assert!(ks.contains(&"1/2"), "settings should show tab switch hint");
        assert!(ks.contains(&"Tab"), "settings should show section hint");
        assert!(ks.contains(&"Enter"), "LLM tab should show Test Connection hint");
        assert!(!ks.contains(&"s"), "LLM tab should not show save hint");
        assert!(!ks.contains(&"1-4"), "draft tab hints should not appear in settings");
        app.settings_tab = SettingsSection::StrategyConfig;
        app.strategy_setup.step = onboarding::strategy_setup::StrategyWizardStep::Review;
        app.strategy_setup.input_editing = false;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"s"), "Strategy tab should show save hint");
        assert!(ks.contains(&"Enter"), "Strategy tab should show Edit hint in normal mode");
    }

    #[test]
    fn compute_keybinds_settings_editing_mode() {
        use crate::protocol::SettingsSection;

        let mut app = app::App::default();
        app.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        app.settings_tab = SettingsSection::LlmConfig;
        app.llm_setup.api_key_editing = true;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"Enter"), "editing should show confirm hint");
        assert!(ks.contains(&"Esc"), "editing should show cancel hint");
        assert!(!ks.contains(&"s"), "editing should not show save hint");
    }

    #[test]
    fn compute_keybinds_draft_mode_unchanged() {
        let mut app = app::App::default();
        app.app_mode = AppMode::Draft;
        let hints = app.compute_keybinds();
        let ks = keys(&hints);
        assert!(ks.contains(&"q"), "draft mode should contain quit hint");
        assert!(ks.contains(&"1-4"), "draft mode should contain tab-switch hint");
        assert!(ks.contains(&"Tab"), "draft mode should contain focus hint");
        assert!(ks.contains(&"r"), "draft mode should contain resync hint");
        assert!(ks.contains(&","), "draft mode should contain settings hint");
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
