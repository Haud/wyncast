// Keyboard input handling and command dispatch.
//
// Translates crossterm key events into UserCommand messages sent to the
// app orchestrator, or into local ViewState mutations (e.g. tab switching,
// scroll, filtering).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::draft::pick::Position;
use crate::protocol::{TabId, UserCommand};
use super::ViewState;

/// The ordered list of positions for cycling with the `p` key.
///
/// None -> C -> 1B -> 2B -> 3B -> SS -> LF -> CF -> RF -> UTIL -> SP -> RP -> None
const POSITION_CYCLE: &[Position] = &[
    Position::Catcher,
    Position::FirstBase,
    Position::SecondBase,
    Position::ThirdBase,
    Position::ShortStop,
    Position::LeftField,
    Position::CenterField,
    Position::RightField,
    Position::Utility,
    Position::StartingPitcher,
    Position::ReliefPitcher,
];

/// Handle a keyboard event.
///
/// Returns `Some(UserCommand)` when the key press should be forwarded to the
/// app orchestrator (e.g. RefreshAnalysis, Quit). Returns `None` when the
/// key press was handled locally by mutating `ViewState` (e.g. tab switching,
/// scrolling, filtering).
pub fn handle_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    // Only process key press events. On Windows, crossterm emits both
    // Press and Release events for each physical keypress; ignoring
    // non-Press events prevents double-processing.
    if key_event.kind != KeyEventKind::Press {
        return None;
    }

    // Ctrl+C always quits immediately regardless of mode (escape hatch)
    if key_event.modifiers.contains(KeyModifiers::CONTROL)
        && key_event.code == KeyCode::Char('c')
    {
        return Some(UserCommand::Quit);
    }

    // Quit confirmation mode: only y/q confirm, n/Esc cancel, everything else blocked
    if view_state.confirm_quit {
        return handle_confirm_quit(key_event, view_state);
    }

    // Filter mode: capture printable characters and special keys
    if view_state.filter_mode {
        return handle_filter_mode(key_event, view_state);
    }

    // Normal mode key dispatch
    match key_event.code {
        // Tab switching
        KeyCode::Char('1') => {
            view_state.active_tab = TabId::Analysis;
            None
        }
        KeyCode::Char('2') => {
            view_state.active_tab = TabId::Available;
            None
        }
        KeyCode::Char('3') => {
            view_state.active_tab = TabId::DraftLog;
            None
        }
        KeyCode::Char('4') => {
            view_state.active_tab = TabId::Teams;
            None
        }

        // Scrolling (main panel)
        KeyCode::Up | KeyCode::Char('k') => {
            scroll_up(view_state, 1);
            None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            scroll_down(view_state, 1);
            None
        }
        KeyCode::PageUp => {
            scroll_up(view_state, page_size());
            None
        }
        KeyCode::PageDown => {
            scroll_down(view_state, page_size());
            None
        }

        // Sidebar scrolling
        KeyCode::Char('[') => {
            sidebar_scroll_up(view_state, 1);
            None
        }
        KeyCode::Char(']') => {
            sidebar_scroll_down(view_state, 1);
            None
        }

        // Filter mode entry: only available on the Players tab where it is relevant
        KeyCode::Char('/') => {
            if view_state.active_tab == TabId::Available {
                view_state.filter_mode = true;
            }
            None
        }

        // Escape: clear filter text if any, otherwise no-op
        KeyCode::Esc => {
            view_state.filter_text.clear();
            view_state.position_filter = None;
            None
        }

        // Position filter cycling
        KeyCode::Char('p') => {
            cycle_position_filter(view_state);
            None
        }

        // LLM refresh commands
        KeyCode::Char('r') => Some(UserCommand::RefreshAnalysis),
        KeyCode::Char('n') => Some(UserCommand::RefreshPlan),

        // Quit: enter confirmation mode instead of quitting immediately
        KeyCode::Char('q') => {
            view_state.confirm_quit = true;
            None
        }

        _ => None,
    }
}

/// Handle key events while in quit confirmation mode.
///
/// In quit confirmation mode:
/// - `y` or `q` confirms quit (sends UserCommand::Quit)
/// - `n` or `Esc` cancels (returns to normal mode)
/// - All other keys are blocked (no-op)
fn handle_confirm_quit(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    match key_event.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('q') | KeyCode::Char('Q') => {
            Some(UserCommand::Quit)
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            view_state.confirm_quit = false;
            None
        }
        _ => None, // Block all other input
    }
}

/// Handle key events while in filter mode.
///
/// In filter mode:
/// - Printable characters are appended to filter_text
/// - Backspace removes the last character
/// - Enter or Esc exits filter mode
fn handle_filter_mode(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    match key_event.code {
        KeyCode::Esc => {
            view_state.filter_mode = false;
            // Clear filter text on Esc
            view_state.filter_text.clear();
            None
        }
        KeyCode::Enter => {
            view_state.filter_mode = false;
            // Keep the filter text on Enter
            None
        }
        KeyCode::Backspace => {
            view_state.filter_text.pop();
            None
        }
        KeyCode::Char(c) => {
            view_state.filter_text.push(c);
            None
        }
        _ => None,
    }
}

/// Cycle the position filter through the defined positions.
///
/// None -> C -> 1B -> 2B -> 3B -> SS -> LF -> CF -> RF -> UTIL -> SP -> RP -> None
fn cycle_position_filter(view_state: &mut ViewState) {
    view_state.position_filter = match &view_state.position_filter {
        None => Some(POSITION_CYCLE[0]),
        Some(current) => {
            // Find the current position in the cycle
            let idx = POSITION_CYCLE
                .iter()
                .position(|p| p == current);
            match idx {
                Some(i) if i + 1 < POSITION_CYCLE.len() => {
                    Some(POSITION_CYCLE[i + 1])
                }
                _ => None, // Last position or not found -> wrap to None
            }
        }
    };
}

/// Get the widget key for scroll state based on the active tab.
fn active_widget_key(view_state: &ViewState) -> &'static str {
    match view_state.active_tab {
        TabId::Analysis => "analysis",
        TabId::Available => "available",
        TabId::DraftLog => "draft_log",
        TabId::Teams => "teams",
    }
}

/// Scroll up by the given number of lines.
fn scroll_up(view_state: &mut ViewState, lines: usize) {
    let key = active_widget_key(view_state);
    let offset = view_state.scroll_offset.entry(key.to_string()).or_insert(0);
    *offset = offset.saturating_sub(lines);
}

/// Scroll down by the given number of lines.
fn scroll_down(view_state: &mut ViewState, lines: usize) {
    let key = active_widget_key(view_state);
    let offset = view_state.scroll_offset.entry(key.to_string()).or_insert(0);
    *offset = offset.saturating_add(lines);
}

/// Scroll the sidebar up by the given number of lines.
fn sidebar_scroll_up(view_state: &mut ViewState, lines: usize) {
    let offset = view_state.scroll_offset.entry("sidebar".to_string()).or_insert(0);
    *offset = offset.saturating_sub(lines);
}

/// Scroll the sidebar down by the given number of lines.
fn sidebar_scroll_down(view_state: &mut ViewState, lines: usize) {
    let offset = view_state.scroll_offset.entry("sidebar".to_string()).or_insert(0);
    *offset = offset.saturating_add(lines);
}

/// Page size for PageUp/PageDown scrolling.
fn page_size() -> usize {
    20
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    /// Helper to create a KeyEvent with no modifiers.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    /// Helper to create a KeyEvent with Ctrl modifier.
    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // -- Tab switching --

    #[test]
    fn tab_1_switches_to_analysis() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Teams;
        let result = handle_key(key(KeyCode::Char('1')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.active_tab, TabId::Analysis);
    }

    #[test]
    fn tab_2_switches_to_available() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('2')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.active_tab, TabId::Available);
    }

    #[test]
    fn tab_3_switches_to_draft_log() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('3')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.active_tab, TabId::DraftLog);
    }

    #[test]
    fn tab_4_switches_to_teams() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('4')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.active_tab, TabId::Teams);
    }

    // -- Scroll --

    #[test]
    fn arrow_up_decrements_scroll() {
        let mut state = ViewState::default();
        state.scroll_offset.insert("analysis".to_string(), 5);
        let result = handle_key(key(KeyCode::Up), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset["analysis"], 4);
    }

    #[test]
    fn arrow_down_increments_scroll() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Down), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset["analysis"], 1);
    }

    #[test]
    fn k_scrolls_up() {
        let mut state = ViewState::default();
        state.scroll_offset.insert("analysis".to_string(), 3);
        let result = handle_key(key(KeyCode::Char('k')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset["analysis"], 2);
    }

    #[test]
    fn j_scrolls_down() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('j')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset["analysis"], 1);
    }

    #[test]
    fn scroll_up_does_not_underflow() {
        let mut state = ViewState::default();
        // Default is 0, scrolling up should stay at 0
        let result = handle_key(key(KeyCode::Up), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset["analysis"], 0);
    }

    #[test]
    fn page_down_scrolls_by_page_size() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::PageDown), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset["analysis"], 20);
    }

    #[test]
    fn page_up_scrolls_by_page_size() {
        let mut state = ViewState::default();
        state.scroll_offset.insert("analysis".to_string(), 25);
        let result = handle_key(key(KeyCode::PageUp), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset["analysis"], 5);
    }

    #[test]
    fn scroll_applies_to_active_tab_widget() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        handle_key(key(KeyCode::Down), &mut state);
        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.scroll_offset.get("available"), Some(&2));
        // Analysis tab should not have a scroll offset
        assert_eq!(state.scroll_offset.get("analysis"), None);
        // Nomination plan is no longer a tab key
        assert_eq!(state.scroll_offset.get("nom_plan"), None);
    }

    #[test]
    fn nom_plan_tab_uses_correct_scroll_key() {
        // Regression test: nomination_plan widget was looking up "plan" instead of "nom_plan".
        // This verifies the input handler writes to "nom_plan" for Tab 2.
        let mut state = ViewState::default();
        state.active_tab = TabId::NomPlan;
        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.scroll_offset.get("nom_plan"), Some(&1));
        assert_eq!(state.scroll_offset.get("plan"), None);
    }

    // -- Filter mode --

    #[test]
    fn slash_enters_filter_mode_on_available_tab() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        let result = handle_key(key(KeyCode::Char('/')), &mut state);
        assert!(result.is_none());
        assert!(state.filter_mode);
    }

    #[test]
    fn slash_does_not_enter_filter_mode_on_other_tabs() {
        for tab in [TabId::Analysis, TabId::DraftLog, TabId::Teams] {
            let mut state = ViewState::default();
            state.active_tab = tab;
            let result = handle_key(key(KeyCode::Char('/')), &mut state);
            assert!(result.is_none(), "/ on {:?} should return None", tab);
            assert!(
                !state.filter_mode,
                "/ on {:?} should not activate filter_mode",
                tab
            );
        }
    }

    #[test]
    fn filter_mode_appends_chars() {
        let mut state = ViewState::default();
        state.filter_mode = true;
        handle_key(key(KeyCode::Char('t')), &mut state);
        handle_key(key(KeyCode::Char('r')), &mut state);
        handle_key(key(KeyCode::Char('o')), &mut state);
        handle_key(key(KeyCode::Char('u')), &mut state);
        handle_key(key(KeyCode::Char('t')), &mut state);
        assert_eq!(state.filter_text, "trout");
        assert!(state.filter_mode);
    }

    #[test]
    fn filter_mode_backspace_removes_char() {
        let mut state = ViewState::default();
        state.filter_mode = true;
        state.filter_text = "test".to_string();
        handle_key(key(KeyCode::Backspace), &mut state);
        assert_eq!(state.filter_text, "tes");
    }

    #[test]
    fn filter_mode_backspace_on_empty_is_noop() {
        let mut state = ViewState::default();
        state.filter_mode = true;
        handle_key(key(KeyCode::Backspace), &mut state);
        assert!(state.filter_text.is_empty());
    }

    #[test]
    fn filter_mode_enter_exits_keeps_text() {
        let mut state = ViewState::default();
        state.filter_mode = true;
        state.filter_text = "trout".to_string();
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(result.is_none());
        assert!(!state.filter_mode);
        assert_eq!(state.filter_text, "trout");
    }

    #[test]
    fn filter_mode_esc_exits_clears_text() {
        let mut state = ViewState::default();
        state.filter_mode = true;
        state.filter_text = "trout".to_string();
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(!state.filter_mode);
        assert!(state.filter_text.is_empty());
    }

    #[test]
    fn filter_mode_does_not_switch_tabs() {
        let mut state = ViewState::default();
        state.filter_mode = true;
        state.active_tab = TabId::Analysis;
        handle_key(key(KeyCode::Char('3')), &mut state);
        // Should add '3' to filter text, not switch tabs
        assert_eq!(state.filter_text, "3");
        assert_eq!(state.active_tab, TabId::Analysis);
    }

    #[test]
    fn filter_mode_ctrl_c_still_quits() {
        let mut state = ViewState::default();
        state.filter_mode = true;
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    // -- Position filter cycling --

    #[test]
    fn position_filter_cycles_from_none() {
        let mut state = ViewState::default();
        assert!(state.position_filter.is_none());
        handle_key(key(KeyCode::Char('p')), &mut state);
        assert_eq!(state.position_filter, Some(Position::Catcher));
    }

    #[test]
    fn position_filter_cycles_through_all() {
        let mut state = ViewState::default();
        let expected = vec![
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
            None, // wraps back to None
        ];
        for expected_pos in expected {
            handle_key(key(KeyCode::Char('p')), &mut state);
            assert_eq!(
                state.position_filter, expected_pos,
                "Expected {:?}, got {:?}",
                expected_pos, state.position_filter
            );
        }
    }

    #[test]
    fn position_filter_wraps_from_rp_to_none() {
        let mut state = ViewState::default();
        state.position_filter = Some(Position::ReliefPitcher);
        handle_key(key(KeyCode::Char('p')), &mut state);
        assert!(state.position_filter.is_none());
    }

    // -- Command returns --

    #[test]
    fn r_returns_refresh_analysis() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('r')), &mut state);
        assert_eq!(result, Some(UserCommand::RefreshAnalysis));
    }

    #[test]
    fn n_returns_refresh_plan() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert_eq!(result, Some(UserCommand::RefreshPlan));
    }

    // -- Quit confirmation --

    #[test]
    fn q_enters_confirm_quit_mode() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert!(result.is_none(), "q should not send Quit immediately");
        assert!(state.confirm_quit, "q should enter confirm_quit mode");
    }

    #[test]
    fn confirm_quit_y_sends_quit() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        let result = handle_key(key(KeyCode::Char('y')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_q_sends_quit() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_n_cancels() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert!(result.is_none());
        assert!(!state.confirm_quit, "n should cancel confirm_quit mode");
    }

    #[test]
    fn confirm_quit_esc_cancels() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(!state.confirm_quit, "Esc should cancel confirm_quit mode");
    }

    #[test]
    fn confirm_quit_blocks_other_keys() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        state.active_tab = TabId::Analysis;

        // Tab switching should be blocked
        let result = handle_key(key(KeyCode::Char('3')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.active_tab, TabId::Analysis, "Tab switch should be blocked");
        assert!(state.confirm_quit, "confirm_quit should remain active");

        // Scrolling should be blocked
        let result = handle_key(key(KeyCode::Down), &mut state);
        assert!(result.is_none());
        assert!(state.scroll_offset.get("analysis").is_none(), "Scroll should be blocked");

        // r should be blocked
        let result = handle_key(key(KeyCode::Char('r')), &mut state);
        assert!(result.is_none());

        // Arbitrary keys should be blocked
        let result = handle_key(key(KeyCode::Char('x')), &mut state);
        assert!(result.is_none());
        assert!(state.confirm_quit, "confirm_quit should remain active");
    }

    #[test]
    fn ctrl_c_quits_immediately_no_confirmation() {
        let mut state = ViewState::default();
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
        assert!(!state.confirm_quit, "Ctrl+C should not enter confirm_quit mode");
    }

    #[test]
    fn ctrl_c_quits_even_during_confirmation() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_uppercase_y_sends_quit() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        let result = handle_key(key(KeyCode::Char('Y')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_uppercase_q_sends_quit() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        let result = handle_key(key(KeyCode::Char('Q')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_uppercase_n_cancels() {
        let mut state = ViewState::default();
        state.confirm_quit = true;
        let result = handle_key(key(KeyCode::Char('N')), &mut state);
        assert!(result.is_none());
        assert!(!state.confirm_quit, "N should cancel confirm_quit mode");
    }

    #[test]
    fn q_in_filter_mode_appends_to_filter_text() {
        let mut state = ViewState::default();
        state.filter_mode = true;
        state.filter_text = "test".to_string();
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert!(result.is_none(), "q in filter mode should not produce a command");
        assert_eq!(state.filter_text, "testq", "q should be appended to filter text");
        assert!(!state.confirm_quit, "q in filter mode should not set confirm_quit");
    }

    #[test]
    fn double_q_workflow_quits() {
        let mut state = ViewState::default();

        // First q: enters confirmation mode
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert!(result.is_none(), "First q should not send Quit");
        assert!(state.confirm_quit, "First q should enter confirm_quit mode");

        // Second q: confirms quit
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit), "Second q should confirm quit");
    }

    // -- Esc in normal mode --

    #[test]
    fn esc_clears_filter_text_and_position() {
        let mut state = ViewState::default();
        state.filter_text = "test".to_string();
        state.position_filter = Some(Position::Catcher);
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(state.filter_text.is_empty());
        assert!(state.position_filter.is_none());
    }

    // -- Unknown keys --

    #[test]
    fn unknown_key_returns_none() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('x')), &mut state);
        assert!(result.is_none());
    }

    // -- KeyEventKind filtering --

    #[test]
    fn release_events_are_ignored() {
        let mut state = ViewState::default();
        let release_event = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        };
        let result = handle_key(release_event, &mut state);
        assert!(result.is_none(), "Release events should be ignored");
    }

    #[test]
    fn repeat_events_are_ignored() {
        let mut state = ViewState::default();
        let repeat_event = KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Repeat,
            state: KeyEventState::NONE,
        };
        let result = handle_key(repeat_event, &mut state);
        assert!(result.is_none(), "Repeat events should be ignored");
        assert!(
            state.scroll_offset.get("analysis").is_none(),
            "Repeat event should not modify scroll state"
        );
    }

    // -- Sidebar scrolling --

    #[test]
    fn bracket_right_scrolls_sidebar_down() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char(']')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset.get("sidebar"), Some(&1));
    }

    #[test]
    fn bracket_left_scrolls_sidebar_up() {
        let mut state = ViewState::default();
        state.scroll_offset.insert("sidebar".to_string(), 5);
        let result = handle_key(key(KeyCode::Char('[')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset.get("sidebar"), Some(&4));
    }

    #[test]
    fn sidebar_scroll_up_does_not_underflow() {
        let mut state = ViewState::default();
        // sidebar offset is 0 (default)
        let result = handle_key(key(KeyCode::Char('[')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.scroll_offset.get("sidebar"), Some(&0));
    }

    #[test]
    fn sidebar_scroll_independent_of_main_scroll() {
        let mut state = ViewState::default();
        // Scroll main panel down
        handle_key(key(KeyCode::Down), &mut state);
        // Scroll sidebar down
        handle_key(key(KeyCode::Char(']')), &mut state);
        // Both should have independent offsets
        assert_eq!(state.scroll_offset.get("analysis"), Some(&1));
        assert_eq!(state.scroll_offset.get("sidebar"), Some(&1));
    }
}
