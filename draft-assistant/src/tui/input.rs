// Keyboard input handling and command dispatch.
//
// Translates crossterm key events into UserCommand messages sent to the
// app orchestrator, or into local ViewState mutations (e.g. tab switching,
// scroll, filtering).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::protocol::{AppMode, OnboardingAction, TabFeature, TabId, UserCommand};
use super::{FocusPanel, PositionFilterModal, ViewState};

/// Handle a keyboard event.
///
/// Returns `Some(UserCommand)` when the key press should be forwarded to the
/// app orchestrator (e.g. RequestKeyframe, Quit). Returns `None` when the
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

    // Dispatch to mode-specific input handlers
    match &view_state.app_mode {
        AppMode::Onboarding(_) => handle_onboarding_key(key_event, view_state),
        AppMode::Settings(_) => handle_settings_key(key_event, view_state),
        AppMode::Draft => handle_draft_key(key_event, view_state),
    }
}

/// Handle keyboard input during the onboarding wizard.
///
/// Dispatches to step-specific handlers based on the current onboarding step.
fn handle_onboarding_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    use crate::onboarding::OnboardingStep;

    match &view_state.app_mode {
        AppMode::Onboarding(OnboardingStep::LlmSetup) => {
            handle_llm_setup_key(key_event, view_state)
        }
        AppMode::Onboarding(OnboardingStep::StrategySetup) |
        AppMode::Onboarding(OnboardingStep::Complete) => {
            handle_strategy_setup_key(key_event, view_state)
        }
        _ => None,
    }
}

/// Handle keyboard input on the strategy setup screen (onboarding step 2).
///
/// Input handling depends on the current editing state:
/// - When editing AI text: captures typed characters, Enter confirms, Esc cancels
/// - When editing a numeric field: captures digits and '.', Enter confirms, Esc cancels
/// - When not editing: Tab/Shift+Tab cycle sections, Up/Down navigate weights,
///   Enter activates editing or triggers actions, s saves, Esc goes back
fn handle_strategy_setup_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    use super::onboarding::strategy_setup::{
        StrategySection, StrategySetupMode, CATEGORIES,
    };

    let state = &mut view_state.strategy_setup;

    // --- AI text input editing mode ---
    if state.ai_input_editing {
        return match key_event.code {
            KeyCode::Enter => {
                state.ai_input_editing = false;
                None
            }
            KeyCode::Esc => {
                state.ai_input_editing = false;
                None
            }
            KeyCode::Backspace => {
                state.ai_input.pop();
                None
            }
            KeyCode::Char(c) => {
                state.ai_input.push(c);
                None
            }
            _ => None,
        };
    }

    // --- Numeric field editing mode ---
    if state.editing_field.is_some() {
        return match key_event.code {
            KeyCode::Enter => {
                state.confirm_edit();
                None
            }
            KeyCode::Esc => {
                state.cancel_edit();
                None
            }
            KeyCode::Backspace => {
                state.field_input.pop();
                None
            }
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                state.field_input.push(c);
                None
            }
            _ => None,
        };
    }

    // --- Normal navigation mode ---
    match key_event.code {
        // Tab: cycle forward through sections
        KeyCode::Tab => {
            state.active_section = state.active_section.next(state.mode);
            None
        }
        // Shift+Tab: cycle backward
        KeyCode::BackTab => {
            state.active_section = state.active_section.prev(state.mode);
            None
        }
        // Up/Down: navigate within the active section
        KeyCode::Up | KeyCode::Char('k') => {
            match state.active_section {
                StrategySection::CategoryWeights => {
                    state.weight_up();
                }
                _ => {}
            }
            None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            match state.active_section {
                StrategySection::CategoryWeights => {
                    state.weight_down();
                }
                _ => {}
            }
            None
        }
        KeyCode::Left | KeyCode::Char('h') => {
            match state.active_section {
                StrategySection::CategoryWeights => {
                    state.weight_left();
                }
                StrategySection::ModeToggle => {
                    if state.mode == StrategySetupMode::Manual {
                        state.toggle_mode();
                    }
                }
                _ => {}
            }
            None
        }
        KeyCode::Right | KeyCode::Char('l') => {
            match state.active_section {
                StrategySection::CategoryWeights => {
                    state.weight_right();
                }
                StrategySection::ModeToggle => {
                    if state.mode == StrategySetupMode::Ai {
                        state.toggle_mode();
                    }
                }
                _ => {}
            }
            None
        }
        // Enter: context-dependent activation
        KeyCode::Enter => {
            match state.active_section {
                StrategySection::ModeToggle => {
                    state.toggle_mode();
                    None
                }
                StrategySection::AiInput => {
                    state.ai_input_editing = true;
                    None
                }
                StrategySection::GenerateButton => {
                    if !state.generating && !state.ai_input.trim().is_empty() {
                        state.generating = true;
                        state.generation_output.clear();
                        state.generation_error = None;
                        let text = state.ai_input.clone();
                        Some(UserCommand::OnboardingAction(
                            OnboardingAction::ConfigureStrategyWithLlm(text),
                        ))
                    } else {
                        None
                    }
                }
                StrategySection::BudgetField => {
                    let current = format!("{}", state.hitting_budget_pct);
                    state.start_editing("budget", &current);
                    None
                }
                StrategySection::CategoryWeights => {
                    let idx = state.selected_weight_idx;
                    if idx < CATEGORIES.len() {
                        let cat_name = CATEGORIES[idx];
                        let current = format!("{:.1}", state.category_weights.get(idx));
                        state.start_editing(cat_name, &current);
                    }
                    None
                }
            }
        }
        // s: save and continue
        KeyCode::Char('s') => {
            let weights = state.category_weights.clone();
            let pct = state.hitting_budget_pct;
            Some(UserCommand::OnboardingAction(
                OnboardingAction::SaveStrategyConfig {
                    hitting_budget_pct: pct,
                    category_weights: weights,
                },
            ))
        }
        // Esc: go back
        KeyCode::Esc => {
            Some(UserCommand::OnboardingAction(OnboardingAction::GoBack))
        }
        // q: quit
        KeyCode::Char('q') => Some(UserCommand::Quit),
        _ => None,
    }
}

/// Handle keyboard input on the LLM setup screen (onboarding step 1).
///
/// Input handling depends on whether the API key text input is active:
/// - When editing: captures typed characters, Enter confirms, Esc cancels
/// - When not editing: Tab/Shift+Tab cycle sections, Up/Down select within
///   lists, Enter activates API key editing or test button, n advances to next
///
/// Provider and model selections dispatch `SetProvider`/`SetModel` commands to
/// the app orchestrator immediately on each arrow key press. This keeps
/// `OnboardingProgress` in sync so that when `GoNext` fires, the app already
/// has the correct values and only needs to persist the API key and advance.
fn handle_llm_setup_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    use super::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

    let state = &mut view_state.llm_setup;

    // --- API key editing mode ---
    if state.api_key_editing {
        return match key_event.code {
            KeyCode::Enter => {
                state.api_key_editing = false;
                // Sync the key to the app on confirm
                let key = state.api_key_input.clone();
                Some(UserCommand::OnboardingAction(OnboardingAction::SetApiKey(key)))
            }
            KeyCode::Esc => {
                state.api_key_input = state.api_key_backup.clone();
                state.api_key_editing = false;
                None
            }
            KeyCode::Backspace => {
                state.api_key_input.pop();
                None
            }
            KeyCode::Char(c) => {
                state.api_key_input.push(c);
                None
            }
            _ => None,
        };
    }

    // --- Normal navigation mode ---
    match key_event.code {
        // Tab: cycle forward through sections
        KeyCode::Tab => {
            state.active_section = state.active_section.next();
            None
        }
        // Shift+Tab (BackTab): cycle backward through sections
        KeyCode::BackTab => {
            state.active_section = state.active_section.prev();
            None
        }
        // Up/Down: select within the active list section
        KeyCode::Up | KeyCode::Char('k') => {
            match state.active_section {
                LlmSetupSection::Provider => {
                    state.provider_up();
                    let provider = state.selected_provider().clone();
                    return Some(UserCommand::OnboardingAction(
                        OnboardingAction::SetProvider(provider),
                    ));
                }
                LlmSetupSection::Model => {
                    state.model_up();
                    if let Some(model) = state.selected_model() {
                        return Some(UserCommand::OnboardingAction(
                            OnboardingAction::SetModel(model.model_id.to_string()),
                        ));
                    }
                }
                _ => {}
            }
            None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            match state.active_section {
                LlmSetupSection::Provider => {
                    state.provider_down();
                    let provider = state.selected_provider().clone();
                    return Some(UserCommand::OnboardingAction(
                        OnboardingAction::SetProvider(provider),
                    ));
                }
                LlmSetupSection::Model => {
                    state.model_down();
                    if let Some(model) = state.selected_model() {
                        return Some(UserCommand::OnboardingAction(
                            OnboardingAction::SetModel(model.model_id.to_string()),
                        ));
                    }
                }
                _ => {}
            }
            None
        }
        // Enter: context-dependent activation
        KeyCode::Enter => {
            match state.active_section {
                LlmSetupSection::ApiKey => {
                    state.api_key_backup = state.api_key_input.clone();
                    state.api_key_editing = true;
                    None
                }
                LlmSetupSection::TestButton => {
                    state.connection_status = LlmConnectionStatus::Testing;
                    Some(UserCommand::OnboardingAction(OnboardingAction::TestConnection))
                }
                _ => None,
            }
        }
        // n: advance to next step
        // OnboardingProgress should already be in sync from real-time
        // SetProvider/SetModel/SetApiKey dispatches. GoNext persists
        // and advances to the next onboarding step.
        KeyCode::Char('n') => {
            Some(UserCommand::OnboardingAction(OnboardingAction::GoNext))
        }
        // Esc: go back (from LLM setup, this is a no-op since it's the first step)
        KeyCode::Esc => {
            Some(UserCommand::OnboardingAction(OnboardingAction::GoBack))
        }
        // q: quit
        KeyCode::Char('q') => Some(UserCommand::Quit),
        _ => None,
    }
}

/// Handle keyboard input on the settings screen.
///
/// Placeholder implementation: Esc returns to draft mode.
/// Real settings input handling will be implemented in Task 6.
fn handle_settings_key(
    key_event: KeyEvent,
    _view_state: &mut ViewState,
) -> Option<UserCommand> {
    match key_event.code {
        KeyCode::Esc => Some(UserCommand::ExitSettings),
        KeyCode::Char('q') => Some(UserCommand::Quit),
        _ => None,
    }
}

/// Handle keyboard input in draft mode (the main operational view).
///
/// This contains all the existing draft-mode key handling logic unchanged.
fn handle_draft_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    // Quit confirmation mode: only y/q confirm, n/Esc cancel, everything else blocked
    if view_state.confirm_quit {
        return handle_confirm_quit(key_event, view_state);
    }

    // Filter mode: capture printable characters and special keys
    if view_state.filter_mode {
        return handle_filter_mode(key_event, view_state);
    }

    // Position filter modal: intercept all keys when the modal is open
    if view_state.position_filter_modal.open {
        return handle_position_filter_modal(key_event, view_state);
    }

    // Normal mode key dispatch
    match key_event.code {
        // Tab switching
        KeyCode::Char('1') => {
            view_state.active_tab = TabId::Analysis;
            view_state.focused_panel = None;
            None
        }
        KeyCode::Char('2') => {
            view_state.active_tab = TabId::Available;
            view_state.focused_panel = None;
            None
        }
        KeyCode::Char('3') => {
            view_state.active_tab = TabId::DraftLog;
            view_state.focused_panel = None;
            None
        }
        KeyCode::Char('4') => {
            view_state.active_tab = TabId::Teams;
            view_state.focused_panel = None;
            None
        }

        // Scrolling: routes to focused panel (or main panel if no focus)
        KeyCode::Up | KeyCode::Char('k') => {
            dispatch_scroll_up(view_state, 1);
            None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            dispatch_scroll_down(view_state, 1);
            None
        }
        KeyCode::PageUp => {
            dispatch_scroll_up(view_state, page_size());
            None
        }
        KeyCode::PageDown => {
            dispatch_scroll_down(view_state, page_size());
            None
        }

        // Panel focus cycling
        KeyCode::Tab => {
            if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                view_state.focused_panel = FocusPanel::prev(view_state.focused_panel);
            } else {
                view_state.focused_panel = FocusPanel::next(view_state.focused_panel);
            }
            None
        }
        KeyCode::BackTab => {
            view_state.focused_panel = FocusPanel::prev(view_state.focused_panel);
            None
        }

        // Filter mode entry: only on tabs that support filtering
        KeyCode::Char('/') => {
            if view_state.active_tab.supports(TabFeature::Filter) {
                view_state.filter_mode = true;
            }
            None
        }

        // Escape: clear focus, filter text, and position filter
        KeyCode::Esc => {
            view_state.focused_panel = None;
            view_state.filter_text.clear();
            view_state.position_filter = None;
            None
        }

        // Position filter modal: only on tabs that support it
        KeyCode::Char('p') => {
            if view_state.active_tab.supports(TabFeature::PositionFilter) {
                open_position_filter_modal(view_state);
            }
            None
        }

        // Request a full keyframe (FULL_STATE_SYNC) from the extension
        KeyCode::Char('r') => Some(UserCommand::RequestKeyframe),

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

/// Open the position filter modal, pre-selecting the row that matches the
/// current active position filter so the user's context is preserved.
fn open_position_filter_modal(view_state: &mut ViewState) {
    let modal = &mut view_state.position_filter_modal;
    modal.open = true;
    modal.search_text.clear();

    // Pre-select the option that matches the current position_filter
    let current = view_state.position_filter;
    let idx = PositionFilterModal::OPTIONS
        .iter()
        .position(|opt| *opt == current)
        .unwrap_or(0);
    view_state.position_filter_modal.selected_index = idx;
}

/// Handle key events while the position filter modal is open.
///
/// - Up/Down arrow: move selection
/// - Enter: apply the selected option and close
/// - Escape: close without applying
/// - Backspace: delete last character in search text
/// - Printable char: append to search text and reset selection to 0
fn handle_position_filter_modal(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    let options = view_state.position_filter_modal.filtered_options();
    let option_count = options.len();

    match key_event.code {
        KeyCode::Esc => {
            // Cancel: close modal without changing the filter
            view_state.position_filter_modal.open = false;
            view_state.position_filter_modal.search_text.clear();
            None
        }
        KeyCode::Enter => {
            // Apply selected option
            if !options.is_empty() {
                let idx = view_state.position_filter_modal.selected_index.min(option_count - 1);
                view_state.position_filter = options[idx];
            }
            view_state.position_filter_modal.open = false;
            view_state.position_filter_modal.search_text.clear();
            None
        }
        KeyCode::Up => {
            let idx = view_state.position_filter_modal.selected_index;
            view_state.position_filter_modal.selected_index = idx.saturating_sub(1);
            None
        }
        KeyCode::Down => {
            if option_count > 0 {
                let idx = view_state.position_filter_modal.selected_index;
                view_state.position_filter_modal.selected_index =
                    (idx + 1).min(option_count - 1);
            }
            None
        }
        KeyCode::Backspace => {
            view_state.position_filter_modal.search_text.pop();
            // Reset selection to 0 after search text change
            view_state.position_filter_modal.selected_index = 0;
            None
        }
        KeyCode::Char(c) => {
            view_state.position_filter_modal.search_text.push(c);
            // Reset selection to 0 so the user starts at the top of the new list
            view_state.position_filter_modal.selected_index = 0;
            None
        }
        _ => None,
    }
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

/// Return the scroll key for the currently focused panel.
///
/// Each focusable panel has its own scroll offset key:
/// - `MainPanel` -> the active tab widget key (analysis/available/draft_log/teams)
/// - `Roster` -> "roster"
/// - `Scarcity` -> "scarcity"
/// - `Budget` -> "budget"
/// - `NominationPlan` -> "nom_plan"
/// - `None` -> the active tab widget key (backward compatible default)
fn focused_scroll_key(view_state: &ViewState) -> &'static str {
    match view_state.focused_panel {
        Some(FocusPanel::Roster) => "roster",
        Some(FocusPanel::Scarcity) => "scarcity",
        Some(FocusPanel::Budget) => "budget",
        Some(FocusPanel::NominationPlan) => "nom_plan",
        Some(FocusPanel::MainPanel) | None => active_widget_key(view_state),
    }
}

/// Dispatch a scroll-up event to the appropriate panel based on focus state.
fn dispatch_scroll_up(view_state: &mut ViewState, lines: usize) {
    let key = focused_scroll_key(view_state);
    let offset = view_state.scroll_offset.entry(key.to_string()).or_insert(0);
    *offset = offset.saturating_sub(lines);
}

/// Dispatch a scroll-down event to the appropriate panel based on focus state.
fn dispatch_scroll_down(view_state: &mut ViewState, lines: usize) {
    let key = focused_scroll_key(view_state);
    let offset = view_state.scroll_offset.entry(key.to_string()).or_insert(0);
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
    use crate::draft::pick::Position;
    use crate::tui::FocusPanel;
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

    // -- Panel focus --

    #[test]
    fn tab_cycles_focus_forward() {
        let mut state = ViewState::default();
        assert!(state.focused_panel.is_none());

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::MainPanel));

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::Roster));

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::Scarcity));

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::Budget));

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::NominationPlan));

        handle_key(key(KeyCode::Tab), &mut state);
        assert!(state.focused_panel.is_none());
    }

    #[test]
    fn backtab_cycles_focus_backward() {
        let mut state = ViewState::default();
        assert!(state.focused_panel.is_none());

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::NominationPlan));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::Budget));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::Scarcity));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::Roster));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::MainPanel));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert!(state.focused_panel.is_none());
    }

    #[test]
    fn shift_tab_cycles_focus_backward() {
        let mut state = ViewState::default();
        assert!(state.focused_panel.is_none());

        let shift_tab = KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        handle_key(shift_tab, &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::NominationPlan));

        handle_key(shift_tab, &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::Budget));

        handle_key(shift_tab, &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::Scarcity));

        handle_key(shift_tab, &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::Roster));

        handle_key(shift_tab, &mut state);
        assert_eq!(state.focused_panel, Some(FocusPanel::MainPanel));

        handle_key(shift_tab, &mut state);
        assert!(state.focused_panel.is_none());
    }

    #[test]
    fn esc_clears_focus() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::MainPanel);

        handle_key(key(KeyCode::Esc), &mut state);
        assert!(state.focused_panel.is_none());
    }

    #[test]
    fn scroll_routes_to_roster_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::Roster);

        handle_key(key(KeyCode::Down), &mut state);
        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.scroll_offset.get("roster"), Some(&2));
        // Main panel scroll should not be affected
        assert!(state.scroll_offset.get("analysis").is_none());
    }

    #[test]
    fn scroll_routes_to_scarcity_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::Scarcity);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.scroll_offset.get("scarcity"), Some(&1));
        assert!(state.scroll_offset.get("analysis").is_none());
    }

    #[test]
    fn scroll_routes_to_budget_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::Budget);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.scroll_offset.get("budget"), Some(&1));
        assert!(state.scroll_offset.get("analysis").is_none());
    }

    #[test]
    fn scroll_routes_to_nom_plan_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::NominationPlan);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.scroll_offset.get("nom_plan"), Some(&1));
        assert!(state.scroll_offset.get("analysis").is_none());
    }

    #[test]
    fn scroll_routes_to_main_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::MainPanel);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.scroll_offset.get("analysis"), Some(&1));
        assert!(state.scroll_offset.get("sidebar").is_none());
    }

    #[test]
    fn scroll_routes_to_main_when_no_focus() {
        let mut state = ViewState::default();
        assert!(state.focused_panel.is_none());

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.scroll_offset.get("analysis"), Some(&1));
        assert!(state.scroll_offset.get("sidebar").is_none());
    }

    #[test]
    fn page_scroll_routes_to_roster_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::Roster);

        handle_key(key(KeyCode::PageDown), &mut state);

        assert_eq!(state.scroll_offset.get("roster"), Some(&20));
        assert!(state.scroll_offset.get("analysis").is_none());
    }

    #[test]
    fn tab_does_not_affect_other_state() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;

        handle_key(key(KeyCode::Tab), &mut state);

        assert_eq!(state.active_tab, TabId::Available, "Tab should not switch tabs");
        assert!(!state.filter_mode, "Tab should not enter filter mode");
    }

    #[test]
    fn tab_switch_clears_focused_panel() {
        // Pressing 1-4 to switch tabs should always clear focused_panel
        for (key_char, expected_tab) in [
            ('1', TabId::Analysis),
            ('2', TabId::Available),
            ('3', TabId::DraftLog),
            ('4', TabId::Teams),
        ] {
            let mut state = ViewState::default();
            state.focused_panel = Some(FocusPanel::MainPanel);
            handle_key(key(KeyCode::Char(key_char)), &mut state);
            assert_eq!(state.active_tab, expected_tab, "Key '{}' should switch to {:?}", key_char, expected_tab);
            assert!(
                state.focused_panel.is_none(),
                "Key '{}': focused_panel should be None after tab switch, got {:?}",
                key_char,
                state.focused_panel
            );
        }
    }

    #[test]
    fn tab_switch_clears_sidebar_focused_panel() {
        // Switching tabs clears sidebar panel focus too (not just MainPanel)
        for focused in [
            FocusPanel::Roster,
            FocusPanel::Scarcity,
            FocusPanel::Budget,
            FocusPanel::NominationPlan,
        ] {
            let mut state = ViewState::default();
            state.focused_panel = Some(focused);
            handle_key(key(KeyCode::Char('2')), &mut state);
            assert!(
                state.focused_panel.is_none(),
                "focused_panel {:?} should be cleared after tab switch",
                focused
            );
        }
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

    // -- Position filter modal --

    #[test]
    fn p_opens_modal_on_available_tab() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        assert!(!state.position_filter_modal.open);
        handle_key(key(KeyCode::Char('p')), &mut state);
        assert!(state.position_filter_modal.open, "p should open the modal on Available tab");
    }

    #[test]
    fn p_does_not_open_modal_on_other_tabs() {
        for tab in [TabId::Analysis, TabId::DraftLog, TabId::Teams] {
            let mut state = ViewState::default();
            state.active_tab = tab;
            handle_key(key(KeyCode::Char('p')), &mut state);
            assert!(
                !state.position_filter_modal.open,
                "p on {:?} should not open modal",
                tab
            );
        }
    }

    #[test]
    fn modal_esc_closes_without_applying() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        state.position_filter = Some(Position::Catcher);
        state.position_filter_modal.open = true;
        state.position_filter_modal.selected_index = 2; // e.g. "1B"
        state.position_filter_modal.search_text = "1".to_string();

        handle_key(key(KeyCode::Esc), &mut state);

        assert!(!state.position_filter_modal.open, "Esc should close modal");
        assert!(
            state.position_filter_modal.search_text.is_empty(),
            "Esc should clear search text"
        );
        // Position filter must NOT have changed
        assert_eq!(
            state.position_filter,
            Some(Position::Catcher),
            "Esc should not change the position filter"
        );
    }

    #[test]
    fn modal_enter_applies_selected_option() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        state.position_filter = None;
        state.position_filter_modal.open = true;
        // Options (unfiltered): ALL(0), C(1), 1B(2), ...
        state.position_filter_modal.selected_index = 1; // "C"

        handle_key(key(KeyCode::Enter), &mut state);

        assert!(!state.position_filter_modal.open, "Enter should close modal");
        assert_eq!(
            state.position_filter,
            Some(Position::Catcher),
            "Enter should apply selected option"
        );
    }

    #[test]
    fn modal_enter_applies_all_option() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        state.position_filter = Some(Position::Catcher);
        state.position_filter_modal.open = true;
        state.position_filter_modal.selected_index = 0; // "ALL"

        handle_key(key(KeyCode::Enter), &mut state);

        assert!(!state.position_filter_modal.open);
        assert!(
            state.position_filter.is_none(),
            "Selecting ALL should clear position filter"
        );
    }

    #[test]
    fn modal_arrow_down_increments_selection() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        state.position_filter_modal.selected_index = 0;

        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.position_filter_modal.selected_index, 1);
    }

    #[test]
    fn modal_arrow_up_decrements_selection() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        state.position_filter_modal.selected_index = 3;

        handle_key(key(KeyCode::Up), &mut state);
        assert_eq!(state.position_filter_modal.selected_index, 2);
    }

    #[test]
    fn modal_arrow_up_does_not_underflow() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        state.position_filter_modal.selected_index = 0;

        handle_key(key(KeyCode::Up), &mut state);
        assert_eq!(state.position_filter_modal.selected_index, 0);
    }

    #[test]
    fn modal_arrow_down_does_not_exceed_option_count() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        let max_idx = crate::tui::PositionFilterModal::OPTIONS.len() - 1;
        state.position_filter_modal.selected_index = max_idx;

        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.position_filter_modal.selected_index, max_idx);
    }

    #[test]
    fn modal_typing_appends_to_search_and_resets_selection() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        state.position_filter_modal.selected_index = 3;

        handle_key(key(KeyCode::Char('s')), &mut state);
        assert_eq!(state.position_filter_modal.search_text, "s");
        assert_eq!(state.position_filter_modal.selected_index, 0, "Typing resets selection");
    }

    #[test]
    fn modal_backspace_removes_char_and_resets_selection() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        state.position_filter_modal.search_text = "SP".to_string();
        state.position_filter_modal.selected_index = 2;

        handle_key(key(KeyCode::Backspace), &mut state);
        assert_eq!(state.position_filter_modal.search_text, "S");
        assert_eq!(state.position_filter_modal.selected_index, 0);
    }

    #[test]
    fn modal_enter_with_filtered_list_applies_correct_option() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        // Type "S" to filter: options with "S" -> SS, SP (and "ALL"? no, ALL doesn't contain S)
        // Actually: SS contains S, SP contains S
        state.position_filter_modal.search_text = "SP".to_string();
        state.position_filter_modal.selected_index = 0; // first match

        handle_key(key(KeyCode::Enter), &mut state);

        assert!(!state.position_filter_modal.open);
        assert_eq!(state.position_filter, Some(Position::StartingPitcher));
    }

    #[test]
    fn modal_pre_selects_current_position_filter() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        state.position_filter = Some(Position::StartingPitcher);

        handle_key(key(KeyCode::Char('p')), &mut state);

        // SP is at index 10 in OPTIONS (0=ALL, 1=C, 2=1B, 3=2B, 4=3B, 5=SS, 6=LF, 7=CF, 8=RF, 9=UTIL, 10=SP, 11=RP)
        let expected_idx = crate::tui::PositionFilterModal::OPTIONS
            .iter()
            .position(|opt| *opt == Some(Position::StartingPitcher))
            .unwrap();
        assert_eq!(state.position_filter_modal.selected_index, expected_idx);
    }

    #[test]
    fn modal_ctrl_c_still_quits() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn modal_blocks_normal_navigation() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        state.position_filter_modal.open = true;

        // '2' should NOT switch tabs while modal is open
        handle_key(key(KeyCode::Char('2')), &mut state);
        // It should have been treated as search text, not tab switch
        assert_eq!(state.position_filter_modal.search_text, "2");
        assert_eq!(state.active_tab, TabId::Available);
    }

    // -- Command returns --

    #[test]
    fn r_returns_request_keyframe() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('r')), &mut state);
        assert_eq!(result, Some(UserCommand::RequestKeyframe));
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
    fn esc_clears_filter_text_position_and_focus() {
        let mut state = ViewState::default();
        state.filter_text = "test".to_string();
        state.position_filter = Some(Position::Catcher);
        state.focused_panel = Some(FocusPanel::Roster);
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(state.filter_text.is_empty());
        assert!(state.position_filter.is_none());
        assert!(state.focused_panel.is_none());
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

    // -- Individual panel scroll independence --

    #[test]
    fn each_panel_scrolls_independently() {
        let mut state = ViewState::default();

        // Scroll main panel down
        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.scroll_offset.get("analysis"), Some(&1));

        // Switch focus to roster and scroll
        state.focused_panel = Some(FocusPanel::Roster);
        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.scroll_offset.get("roster"), Some(&1));

        // Main panel scroll should be untouched
        assert_eq!(state.scroll_offset.get("analysis"), Some(&1));
        // Other panels should be untouched
        assert!(state.scroll_offset.get("scarcity").is_none());
    }

    // -- AppMode-aware input dispatch --

    // -- LLM Setup screen input tests --

    #[test]
    fn llm_setup_n_sends_go_next() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::GoNext))
        ));
    }

    #[test]
    fn llm_setup_esc_sends_go_back() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::GoBack))
        ));
    }

    #[test]
    fn llm_setup_tab_cycles_sections() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::Provider);

        let result = handle_key(key(KeyCode::Tab), &mut state);
        assert!(result.is_none()); // local state mutation
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::Model);

        let result = handle_key(key(KeyCode::Tab), &mut state);
        assert!(result.is_none());
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::ApiKey);

        let result = handle_key(key(KeyCode::Tab), &mut state);
        assert!(result.is_none());
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::TestButton);

        // Wraps back to Provider
        let result = handle_key(key(KeyCode::Tab), &mut state);
        assert!(result.is_none());
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::Provider);
    }

    #[test]
    fn llm_setup_down_changes_provider() {
        use crate::onboarding::OnboardingStep;
        use crate::llm::provider::LlmProvider;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);

        let result = handle_key(key(KeyCode::Down), &mut state);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::SetProvider(LlmProvider::Google)))
        ));
        assert_eq!(state.llm_setup.selected_provider_idx, 1);
    }

    #[test]
    fn llm_setup_enter_on_api_key_starts_editing() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.llm_setup.active_section = LlmSetupSection::ApiKey;

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(result.is_none());
        assert!(state.llm_setup.api_key_editing);
    }

    #[test]
    fn llm_setup_api_key_editing_captures_chars() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.llm_setup.active_section = LlmSetupSection::ApiKey;
        state.llm_setup.api_key_editing = true;

        handle_key(key(KeyCode::Char('a')), &mut state);
        handle_key(key(KeyCode::Char('b')), &mut state);
        handle_key(key(KeyCode::Char('c')), &mut state);
        assert_eq!(state.llm_setup.api_key_input, "abc");

        handle_key(key(KeyCode::Backspace), &mut state);
        assert_eq!(state.llm_setup.api_key_input, "ab");
    }

    #[test]
    fn llm_setup_api_key_enter_confirms_and_sends_set_api_key() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.llm_setup.active_section = LlmSetupSection::ApiKey;
        state.llm_setup.api_key_editing = true;
        state.llm_setup.api_key_input = "sk-test-key".to_string();

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(!state.llm_setup.api_key_editing);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::SetApiKey(_)))
        ));
    }

    #[test]
    fn llm_setup_api_key_esc_cancels_editing() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.llm_setup.api_key_editing = true;

        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(!state.llm_setup.api_key_editing);
        assert!(result.is_none());
    }

    #[test]
    fn llm_setup_enter_on_test_button_sends_test_connection() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.llm_setup.active_section = LlmSetupSection::TestButton;

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::TestConnection))
        ));
    }

    #[test]
    fn llm_setup_q_quits() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    // -- Strategy setup placeholder input tests --

    #[test]
    fn strategy_setup_enter_on_mode_toggle_toggles_mode() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::strategy_setup::StrategySetupMode;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        // Default section is ModeToggle, default mode is Ai
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(result.is_none()); // toggle is UI-only, no command
        assert_eq!(state.strategy_setup.mode, StrategySetupMode::Manual);
    }

    #[test]
    fn strategy_setup_esc_sends_go_back() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::GoBack))
        ));
    }

    #[test]
    fn strategy_setup_s_sends_save_strategy_config() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        let result = handle_key(key(KeyCode::Char('s')), &mut state);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(
                OnboardingAction::SaveStrategyConfig { .. }
            ))
        ));
    }

    #[test]
    fn onboarding_mode_ignores_draft_keys() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        // Tab switching should not work in onboarding mode
        let result = handle_key(key(KeyCode::Char('1')), &mut state);
        assert!(result.is_none());
    }

    #[test]
    fn settings_mode_esc_returns_to_draft() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        let result = handle_key(key(KeyCode::Esc), &mut state);
        // Esc now returns ExitSettings command instead of mutating view_state directly
        assert_eq!(result, Some(UserCommand::ExitSettings));
        // ViewState.app_mode should NOT be mutated; the app orchestrator handles the transition
        assert_eq!(state.app_mode, AppMode::Settings(SettingsSection::LlmConfig));
    }

    #[test]
    fn settings_mode_q_quits() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn settings_mode_ignores_draft_keys() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        // Tab switching should not work in settings mode
        let result = handle_key(key(KeyCode::Char('1')), &mut state);
        assert!(result.is_none());
    }

    #[test]
    fn draft_mode_tab_switching_still_works() {
        let mut state = ViewState::default();
        state.app_mode = AppMode::Draft;
        state.active_tab = TabId::Analysis;
        let result = handle_key(key(KeyCode::Char('2')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.active_tab, TabId::Available);
    }

    #[test]
    fn ctrl_c_quits_in_any_mode() {
        use crate::onboarding::OnboardingStep;
        use crate::protocol::SettingsSection;

        // Onboarding mode
        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));

        // Settings mode
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));

        // Draft mode
        state.app_mode = AppMode::Draft;
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }
}
