// Keyboard input handling and command dispatch.
//
// Translates crossterm key events into UserCommand messages sent to the
// app orchestrator, or into local ViewState mutations (e.g. tab switching,
// scroll, filtering).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::protocol::{AppMode, UserCommand};
use super::ViewState;

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

    // Suppress a stray `[` that some terminals emit as the CSI introducer
    // byte after an escape sequence is partially parsed. The flag is set
    // when entering a text-editing mode and cleared on the next key event.
    if std::mem::take(&mut view_state.suppress_next_bracket)
        && key_event.code == KeyCode::Char('[')
        && key_event.modifiers == KeyModifiers::NONE
    {
        return None;
    }

    // Ctrl+C always quits immediately regardless of mode (escape hatch)
    if key_event.modifiers.contains(KeyModifiers::CONTROL)
        && key_event.code == KeyCode::Char('c')
    {
        return Some(UserCommand::Quit);
    }

    // Snapshot which text-editing sub-modes are active *before* handling the
    // key. After the handler returns we compare: if a text-editing mode was
    // just activated (wasn't active before, is active now) we set the bracket
    // suppression flag so that a stray `[` CSI byte on the *next* key event
    // is silently discarded instead of being inserted into the text buffer.
    let was_editing = is_text_editing_active(view_state);

    // Dispatch to mode-specific input handlers
    let result = match &view_state.app_mode {
        AppMode::Onboarding(_) => handle_onboarding_key(key_event, view_state),
        AppMode::Settings(_) => handle_settings_key(key_event, view_state),
        AppMode::Draft => view_state.draft_screen.handle_key(key_event),
    };

    if !was_editing && is_text_editing_active(view_state) {
        view_state.suppress_next_bracket = true;
    }

    result
}

/// Return `true` if any text-editing sub-mode is currently active.
///
/// Used to detect transitions *into* editing mode so we can suppress a
/// potential stray `[` character from a partially-parsed CSI escape sequence.
fn is_text_editing_active(view_state: &ViewState) -> bool {
    let strategy_editing = matches!(
        view_state.app_mode,
        AppMode::Onboarding(crate::onboarding::OnboardingStep::StrategySetup)
            | AppMode::Onboarding(crate::onboarding::OnboardingStep::Complete)
            | AppMode::Settings(crate::protocol::SettingsSection::StrategyConfig)
    ) && (view_state.strategy_setup.input_editing
        || view_state.strategy_setup.editing_field.is_some()
        || view_state.strategy_setup.overview_editing);

    view_state.draft_screen.main_panel.available.filter_mode()
        || view_state.llm_setup.api_key_editing
        || strategy_editing
        || view_state.draft_screen.modal_layer.position_filter.open
}

/// Handle keyboard input during the onboarding wizard.
///
/// Dispatches to step-specific handlers based on the current onboarding step.
fn handle_onboarding_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    use super::onboarding;

    let step = match &view_state.app_mode {
        AppMode::Onboarding(step) => step.clone(),
        _ => return None,
    };
    let msg = onboarding::key_to_message(
        &step,
        &view_state.llm_setup,
        &view_state.strategy_setup,
        key_event,
    );
    match msg {
        Some(m) => onboarding::update(
            &step,
            &mut view_state.llm_setup,
            &mut view_state.strategy_setup,
            m,
        ),
        None => None,
    }
}

/// Handle keyboard input on the settings screen.
///
/// Dispatches to the appropriate handler depending on the active settings tab.
fn handle_settings_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    use super::settings;

    let msg = settings::key_to_message(
        view_state.settings_tab,
        &view_state.llm_setup,
        &view_state.strategy_setup,
        &view_state.confirm_exit_settings,
        key_event,
    );
    match msg {
        Some(m) => settings::update(
            view_state.settings_tab,
            &mut view_state.llm_setup,
            &mut view_state.strategy_setup,
            &mut view_state.confirm_exit_settings,
            m,
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
    use crate::draft::pick::Position;
    use crate::protocol::{OnboardingAction, TabId};
    use crate::tui::FocusPanel;
    use crate::tui::draft::main_panel::MainPanelMessage;
    use crate::tui::draft::main_panel::analysis::AnalysisPanelMessage;
    use crate::tui::draft::main_panel::available::AvailablePanelMessage;
    use crate::tui::draft::modal::position_filter::PositionFilterModalMessage;
    use crate::tui::scroll::ScrollDirection;
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
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Teams));
        let result = handle_key(key(KeyCode::Char('1')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.active_tab(), TabId::Analysis);
    }

    #[test]
    fn tab_2_switches_to_available() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('2')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.active_tab(), TabId::Available);
    }

    #[test]
    fn tab_3_switches_to_draft_log() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('3')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.active_tab(), TabId::DraftLog);
    }

    #[test]
    fn tab_4_switches_to_teams() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('4')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.active_tab(), TabId::Teams);
    }

    // -- Scroll --

    #[test]
    fn arrow_up_decrements_scroll() {
        let mut state = ViewState::default();
        // Pre-scroll the analysis panel down 5 positions
        for _ in 0..5 {
            state.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = handle_key(key(KeyCode::Up), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 4);
    }

    #[test]
    fn arrow_down_increments_scroll() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Down), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 1);
    }

    #[test]
    fn k_scrolls_up() {
        let mut state = ViewState::default();
        for _ in 0..3 {
            state.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = handle_key(key(KeyCode::Char('k')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 2);
    }

    #[test]
    fn j_scrolls_down() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('j')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 1);
    }

    #[test]
    fn scroll_up_does_not_underflow() {
        let mut state = ViewState::default();
        // Default is 0, scrolling up should stay at 0
        let result = handle_key(key(KeyCode::Up), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn page_down_scrolls_by_page_size() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::PageDown), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 20);
    }

    #[test]
    fn page_up_scrolls_by_page_size() {
        let mut state = ViewState::default();
        for _ in 0..25 {
            state.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = handle_key(key(KeyCode::PageUp), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 5);
    }

    #[test]
    fn scroll_applies_to_active_tab_widget() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        handle_key(key(KeyCode::Down), &mut state);
        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.draft_screen.main_panel.available.scroll_offset(), 2);
        // Analysis panel should not have been scrolled
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 0);
        // Nomination plan should not have been scrolled
        assert_eq!(state.draft_screen.sidebar.plan.scroll_offset(), 0);
    }

    // -- Panel focus --

    #[test]
    fn tab_cycles_focus_forward() {
        let mut state = ViewState::default();
        assert!(state.draft_screen.focused_panel.is_none());

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::MainPanel));

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::Roster));

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::Scarcity));

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::Budget));

        handle_key(key(KeyCode::Tab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::NominationPlan));

        handle_key(key(KeyCode::Tab), &mut state);
        assert!(state.draft_screen.focused_panel.is_none());
    }

    #[test]
    fn backtab_cycles_focus_backward() {
        let mut state = ViewState::default();
        assert!(state.draft_screen.focused_panel.is_none());

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::NominationPlan));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::Budget));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::Scarcity));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::Roster));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::MainPanel));

        handle_key(key(KeyCode::BackTab), &mut state);
        assert!(state.draft_screen.focused_panel.is_none());
    }

    #[test]
    fn shift_tab_cycles_focus_backward() {
        let mut state = ViewState::default();
        assert!(state.draft_screen.focused_panel.is_none());

        let shift_tab = KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        handle_key(shift_tab, &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::NominationPlan));

        handle_key(shift_tab, &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::Budget));

        handle_key(shift_tab, &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::Scarcity));

        handle_key(shift_tab, &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::Roster));

        handle_key(shift_tab, &mut state);
        assert_eq!(state.draft_screen.focused_panel, Some(FocusPanel::MainPanel));

        handle_key(shift_tab, &mut state);
        assert!(state.draft_screen.focused_panel.is_none());
    }

    #[test]
    fn esc_clears_focus() {
        let mut state = ViewState::default();
        state.draft_screen.focused_panel = Some(FocusPanel::MainPanel);

        handle_key(key(KeyCode::Esc), &mut state);
        assert!(state.draft_screen.focused_panel.is_none());
    }

    #[test]
    fn scroll_routes_to_roster_when_focused() {
        let mut state = ViewState::default();
        state.draft_screen.focused_panel = Some(FocusPanel::Roster);

        handle_key(key(KeyCode::Down), &mut state);
        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.draft_screen.sidebar.roster.scroll_offset(), 2);
        // Analysis panel scroll should not be affected
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_scarcity_when_focused() {
        let mut state = ViewState::default();
        state.draft_screen.focused_panel = Some(FocusPanel::Scarcity);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.draft_screen.sidebar.scarcity.scroll_offset(), 1);
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_budget_when_focused() {
        let mut state = ViewState::default();
        state.draft_screen.focused_panel = Some(FocusPanel::Budget);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.draft_screen.scroll_offset.get("budget"), Some(&1));
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_nom_plan_when_focused() {
        let mut state = ViewState::default();
        state.draft_screen.focused_panel = Some(FocusPanel::NominationPlan);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.draft_screen.sidebar.plan.scroll_offset(), 1);
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_main_when_focused() {
        let mut state = ViewState::default();
        state.draft_screen.focused_panel = Some(FocusPanel::MainPanel);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 1);
        assert!(state.draft_screen.scroll_offset.get("sidebar").is_none());
    }

    #[test]
    fn scroll_routes_to_main_when_no_focus() {
        let mut state = ViewState::default();
        assert!(state.draft_screen.focused_panel.is_none());

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 1);
        assert!(state.draft_screen.scroll_offset.get("sidebar").is_none());
    }

    #[test]
    fn page_scroll_routes_to_roster_when_focused() {
        let mut state = ViewState::default();
        state.draft_screen.focused_panel = Some(FocusPanel::Roster);

        handle_key(key(KeyCode::PageDown), &mut state);

        assert_eq!(state.draft_screen.sidebar.roster.scroll_offset(), 20);
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn tab_does_not_affect_other_state() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));

        handle_key(key(KeyCode::Tab), &mut state);

        assert_eq!(state.draft_screen.main_panel.active_tab(), TabId::Available, "Tab should not switch tabs");
        assert!(!state.draft_screen.main_panel.available.filter_mode(), "Tab should not enter filter mode");
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
            state.draft_screen.focused_panel = Some(FocusPanel::MainPanel);
            handle_key(key(KeyCode::Char(key_char)), &mut state);
            assert_eq!(state.draft_screen.main_panel.active_tab(), expected_tab, "Key '{}' should switch to {:?}", key_char, expected_tab);
            assert!(
                state.draft_screen.focused_panel.is_none(),
                "Key '{}': focused_panel should be None after tab switch, got {:?}",
                key_char,
                state.draft_screen.focused_panel
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
            state.draft_screen.focused_panel = Some(focused);
            handle_key(key(KeyCode::Char('2')), &mut state);
            assert!(
                state.draft_screen.focused_panel.is_none(),
                "focused_panel {:?} should be cleared after tab switch",
                focused
            );
        }
    }

    // -- Filter mode --

    #[test]
    fn slash_enters_filter_mode_on_available_tab() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        let result = handle_key(key(KeyCode::Char('/')), &mut state);
        assert!(result.is_none());
        assert!(state.draft_screen.main_panel.available.filter_mode());
    }

    #[test]
    fn slash_does_not_enter_filter_mode_on_other_tabs() {
        for tab in [TabId::Analysis, TabId::DraftLog, TabId::Teams] {
            let mut state = ViewState::default();
            state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(tab));
            let result = handle_key(key(KeyCode::Char('/')), &mut state);
            assert!(result.is_none(), "/ on {:?} should return None", tab);
            assert!(
                !state.draft_screen.main_panel.available.filter_mode(),
                "/ on {:?} should not activate filter_mode",
                tab
            );
        }
    }

    #[test]
    fn filter_mode_appends_chars() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        handle_key(key(KeyCode::Char('t')), &mut state);
        handle_key(key(KeyCode::Char('r')), &mut state);
        handle_key(key(KeyCode::Char('o')), &mut state);
        handle_key(key(KeyCode::Char('u')), &mut state);
        handle_key(key(KeyCode::Char('t')), &mut state);
        assert_eq!(state.draft_screen.main_panel.available.filter_text().value(), "trout");
        assert!(state.draft_screen.main_panel.available.filter_mode());
    }

    #[test]
    fn filter_mode_backspace_removes_char() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        // Type "test"
        for ch in "test".chars() {
            state.draft_screen.main_panel.available.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        handle_key(key(KeyCode::Backspace), &mut state);
        assert_eq!(state.draft_screen.main_panel.available.filter_text().value(), "tes");
    }

    #[test]
    fn filter_mode_backspace_on_empty_is_noop() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        handle_key(key(KeyCode::Backspace), &mut state);
        assert!(state.draft_screen.main_panel.available.filter_text().is_empty());
    }

    #[test]
    fn filter_mode_enter_exits_keeps_text() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        for ch in "trout".chars() {
            state.draft_screen.main_panel.available.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(result.is_none());
        assert!(!state.draft_screen.main_panel.available.filter_mode());
        assert_eq!(state.draft_screen.main_panel.available.filter_text().value(), "trout");
    }

    #[test]
    fn filter_mode_esc_exits_clears_text() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        for ch in "trout".chars() {
            state.draft_screen.main_panel.available.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(!state.draft_screen.main_panel.available.filter_mode());
        assert!(state.draft_screen.main_panel.available.filter_text().is_empty());
    }

    #[test]
    fn filter_mode_does_not_switch_tabs() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Analysis));
        handle_key(key(KeyCode::Char('3')), &mut state);
        // Should add '3' to filter text, not switch tabs
        assert_eq!(state.draft_screen.main_panel.available.filter_text().value(), "3");
        assert_eq!(state.draft_screen.main_panel.active_tab(), TabId::Analysis);
    }

    #[test]
    fn filter_mode_ctrl_c_still_quits() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    // -- Position filter modal --

    #[test]
    fn p_opens_modal_on_available_tab() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        assert!(!state.draft_screen.modal_layer.position_filter.open);
        handle_key(key(KeyCode::Char('p')), &mut state);
        assert!(state.draft_screen.modal_layer.position_filter.open, "p should open the modal on Available tab");
    }

    #[test]
    fn p_does_not_open_modal_on_other_tabs() {
        for tab in [TabId::Analysis, TabId::DraftLog, TabId::Teams] {
            let mut state = ViewState::default();
            state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(tab));
            handle_key(key(KeyCode::Char('p')), &mut state);
            assert!(
                !state.draft_screen.modal_layer.position_filter.open,
                "p on {:?} should not open modal",
                tab
            );
        }
    }

    #[test]
    fn modal_esc_closes_without_applying() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::SetPositionFilter(Some(Position::Catcher)));
        // Open via message, then move down twice to select "1B" (index 2)
        state.draft_screen.modal_layer.position_filter.update(PositionFilterModalMessage::Open {
            current_filter: Some(Position::Catcher),
        });

        handle_key(key(KeyCode::Esc), &mut state);

        assert!(!state.draft_screen.modal_layer.position_filter.open, "Esc should close modal");
        // Position filter must NOT have changed
        assert_eq!(
            state.draft_screen.main_panel.available.position_filter(),
            Some(Position::Catcher),
            "Esc should not change the position filter"
        );
    }

    #[test]
    fn modal_enter_applies_selected_option() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        // Open modal, then move down to index 1 (Catcher)
        state.draft_screen.modal_layer.position_filter.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        handle_key(key(KeyCode::Down), &mut state); // move to index 1 = C

        handle_key(key(KeyCode::Enter), &mut state);

        assert!(!state.draft_screen.modal_layer.position_filter.open, "Enter should close modal");
        assert_eq!(
            state.draft_screen.main_panel.available.position_filter(),
            Some(Position::Catcher),
            "Enter should apply selected option"
        );
    }

    #[test]
    fn modal_enter_applies_all_option() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::SetPositionFilter(Some(Position::Catcher)));
        // Open modal: selected_index defaults to 0 = "ALL"
        state.draft_screen.modal_layer.position_filter.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });

        handle_key(key(KeyCode::Enter), &mut state);

        assert!(!state.draft_screen.modal_layer.position_filter.open);
        assert!(
            state.draft_screen.main_panel.available.position_filter().is_none(),
            "Selecting ALL should clear position filter"
        );
    }

    #[test]
    fn modal_arrow_down_does_not_close() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.position_filter.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });

        handle_key(key(KeyCode::Down), &mut state);
        assert!(state.draft_screen.modal_layer.position_filter.open, "Down should not close modal");
    }

    #[test]
    fn modal_arrow_up_does_not_close() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.position_filter.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });

        handle_key(key(KeyCode::Up), &mut state);
        assert!(state.draft_screen.modal_layer.position_filter.open, "Up should not close modal");
    }

    #[test]
    fn modal_enter_with_filtered_list_applies_correct_option() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.position_filter.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        // Type "SP" to filter
        handle_key(key(KeyCode::Char('S')), &mut state);
        handle_key(key(KeyCode::Char('P')), &mut state);

        handle_key(key(KeyCode::Enter), &mut state);

        assert!(!state.draft_screen.modal_layer.position_filter.open);
        assert_eq!(state.draft_screen.main_panel.available.position_filter(), Some(Position::StartingPitcher));
    }

    #[test]
    fn modal_pre_selects_current_position_filter() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::SetPositionFilter(Some(Position::StartingPitcher)));

        handle_key(key(KeyCode::Char('p')), &mut state);

        // Verify the modal opened -- detailed selection index testing is in the
        // component's own unit tests.
        assert!(state.draft_screen.modal_layer.position_filter.open);
        // Confirm by pressing Enter: should apply the pre-selected SP
        handle_key(key(KeyCode::Enter), &mut state);
        assert_eq!(
            state.draft_screen.main_panel.available.position_filter(),
            Some(Position::StartingPitcher),
            "Pre-selected option should match current filter"
        );
    }

    #[test]
    fn modal_ctrl_c_still_quits() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.position_filter.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn modal_blocks_normal_navigation() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        state.draft_screen.modal_layer.position_filter.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });

        // '2' should NOT switch tabs while modal is open
        handle_key(key(KeyCode::Char('2')), &mut state);
        // It should have been treated as search text, not tab switch
        assert_eq!(state.draft_screen.main_panel.active_tab(), TabId::Available);
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
        assert!(state.draft_screen.modal_layer.quit_confirm.open,"q should enter confirm_quit mode");
    }

    #[test]
    fn confirm_quit_y_sends_quit() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.quit_confirm.open = true;
        let result = handle_key(key(KeyCode::Char('y')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_q_sends_quit() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.quit_confirm.open = true;
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_n_cancels() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.quit_confirm.open = true;
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert!(result.is_none());
        assert!(!state.draft_screen.modal_layer.quit_confirm.open,"n should cancel confirm_quit mode");
    }

    #[test]
    fn confirm_quit_esc_cancels() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.quit_confirm.open = true;
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(!state.draft_screen.modal_layer.quit_confirm.open,"Esc should cancel confirm_quit mode");
    }

    #[test]
    fn confirm_quit_blocks_other_keys() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.quit_confirm.open = true;
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Analysis));

        // Tab switching should be blocked
        let result = handle_key(key(KeyCode::Char('3')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.active_tab(), TabId::Analysis, "Tab switch should be blocked");
        assert!(state.draft_screen.modal_layer.quit_confirm.open,"confirm_quit should remain active");

        // Scrolling should be blocked
        let result = handle_key(key(KeyCode::Down), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 0, "Scroll should be blocked");

        // r should be blocked
        let result = handle_key(key(KeyCode::Char('r')), &mut state);
        assert!(result.is_none());

        // Arbitrary keys should be blocked
        let result = handle_key(key(KeyCode::Char('x')), &mut state);
        assert!(result.is_none());
        assert!(state.draft_screen.modal_layer.quit_confirm.open,"confirm_quit should remain active");
    }

    #[test]
    fn ctrl_c_quits_immediately_no_confirmation() {
        let mut state = ViewState::default();
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
        assert!(!state.draft_screen.modal_layer.quit_confirm.open,"Ctrl+C should not enter confirm_quit mode");
    }

    #[test]
    fn ctrl_c_quits_even_during_confirmation() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.quit_confirm.open = true;
        let result = handle_key(ctrl_key(KeyCode::Char('c')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_uppercase_y_sends_quit() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.quit_confirm.open = true;
        let result = handle_key(key(KeyCode::Char('Y')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_uppercase_q_sends_quit() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.quit_confirm.open = true;
        let result = handle_key(key(KeyCode::Char('Q')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn confirm_quit_uppercase_n_cancels() {
        let mut state = ViewState::default();
        state.draft_screen.modal_layer.quit_confirm.open = true;
        let result = handle_key(key(KeyCode::Char('N')), &mut state);
        assert!(result.is_none());
        assert!(!state.draft_screen.modal_layer.quit_confirm.open,"N should cancel confirm_quit mode");
    }

    #[test]
    fn q_in_filter_mode_appends_to_filter_text() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        for ch in "test".chars() {
            state.draft_screen.main_panel.available.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert!(result.is_none(), "q in filter mode should not produce a command");
        assert_eq!(state.draft_screen.main_panel.available.filter_text().value(), "testq", "q should be appended to filter text");
        assert!(!state.draft_screen.modal_layer.quit_confirm.open,"q in filter mode should not set confirm_quit");
    }

    #[test]
    fn double_q_workflow_quits() {
        let mut state = ViewState::default();

        // First q: enters confirmation mode
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert!(result.is_none(), "First q should not send Quit");
        assert!(state.draft_screen.modal_layer.quit_confirm.open,"First q should enter confirm_quit mode");

        // Second q: confirms quit
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert_eq!(result, Some(UserCommand::Quit), "Second q should confirm quit");
    }

    // -- Esc in normal mode --

    #[test]
    fn esc_clears_filter_text_position_and_focus() {
        let mut state = ViewState::default();
        for ch in "test".chars() {
            state.draft_screen.main_panel.available.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::SetPositionFilter(Some(Position::Catcher)));
        state.draft_screen.focused_panel = Some(FocusPanel::Roster);
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(state.draft_screen.main_panel.available.filter_text().is_empty());
        assert!(state.draft_screen.main_panel.available.position_filter().is_none());
        assert!(state.draft_screen.focused_panel.is_none());
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
        assert_eq!(
            state.draft_screen.main_panel.analysis.scroll_offset(), 0,
            "Repeat event should not modify scroll state"
        );
    }

    // -- Bracket suppression mechanism --

    #[test]
    fn entering_filter_mode_sets_suppress_next_bracket() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        assert!(!state.suppress_next_bracket);

        // '/' enters filter mode, which transitions into text editing,
        // so the post-handler check should set suppress_next_bracket.
        handle_key(key(KeyCode::Char('/')), &mut state);
        assert!(state.draft_screen.main_panel.available.filter_mode());
        assert!(
            state.suppress_next_bracket,
            "Entering filter mode should set suppress_next_bracket"
        );
    }

    #[test]
    fn bracket_immediately_after_entering_text_mode_is_suppressed() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));

        // Enter filter mode (sets the suppression flag)
        handle_key(key(KeyCode::Char('/')), &mut state);
        assert!(state.suppress_next_bracket);

        // The very next key is '[' — it should be silently suppressed
        let result = handle_key(key(KeyCode::Char('[')), &mut state);
        assert!(result.is_none(), "Stray '[' should be suppressed");
        assert!(
            state.draft_screen.main_panel.available.filter_text().is_empty(),
            "'[' should not be inserted into filter text"
        );
        assert!(
            !state.suppress_next_bracket,
            "Flag should be consumed after suppression"
        );
    }

    #[test]
    fn suppress_flag_consumed_even_when_next_key_is_not_bracket() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));

        // Enter filter mode (sets the suppression flag)
        handle_key(key(KeyCode::Char('/')), &mut state);
        assert!(state.suppress_next_bracket);

        // Next key is 'a', not '[' — flag should still be consumed
        handle_key(key(KeyCode::Char('a')), &mut state);
        assert!(
            !state.suppress_next_bracket,
            "Flag should be consumed even when the key is not '['"
        );
        assert_eq!(
            state.draft_screen.main_panel.available.filter_text().value(),
            "a",
            "'a' should be inserted normally"
        );
    }

    #[test]
    fn bracket_not_suppressed_when_flag_is_not_set() {
        let mut state = ViewState::default();
        state.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        assert!(!state.suppress_next_bracket);

        // '[' without the suppression flag should be inserted normally
        handle_key(key(KeyCode::Char('[')), &mut state);
        assert_eq!(
            state.draft_screen.main_panel.available.filter_text().value(),
            "[",
            "Normal '[' should be inserted when flag is not set"
        );
    }

    // -- Individual panel scroll independence --

    #[test]
    fn each_panel_scrolls_independently() {
        let mut state = ViewState::default();

        // Scroll main panel down
        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 1);

        // Switch focus to roster and scroll
        state.draft_screen.focused_panel = Some(FocusPanel::Roster);
        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.draft_screen.sidebar.roster.scroll_offset(), 1);

        // Main panel scroll should be untouched
        assert_eq!(state.draft_screen.main_panel.analysis.scroll_offset(), 1);
        // Other panels should be untouched
        assert!(state.draft_screen.scroll_offset.get("scarcity").is_none());
    }

    // -- AppMode-aware input dispatch --

    // -- LLM Setup screen input tests --

    #[test]
    fn llm_setup_n_blocked_until_connection_tested() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmConnectionStatus;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);

        // 'n' should be blocked when connection hasn't been tested
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert!(result.is_none());

        // 'n' should be blocked when test is in progress
        state.llm_setup.connection_status = LlmConnectionStatus::Testing;
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert!(result.is_none());

        // 'n' should be blocked when test failed
        state.llm_setup.connection_status = LlmConnectionStatus::Failed("error".to_string());
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert!(result.is_none());

        // 'n' should work after successful connection test
        state.llm_setup.connection_status = LlmConnectionStatus::Success("ok".to_string());
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
    fn llm_setup_tab_only_visits_visible_sections() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::Provider);

        // Tab does nothing when only Provider is visible (nothing confirmed yet)
        let result = handle_key(key(KeyCode::Tab), &mut state);
        assert!(result.is_none());
        // Model is not visible, so Tab shouldn't advance
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::Provider);

        // Confirm provider, making model visible
        state.llm_setup.confirm_current_section();
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::Model);

        // Tab from Model should not advance (ApiKey not visible yet)
        let result = handle_key(key(KeyCode::Tab), &mut state);
        assert!(result.is_none());
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::Model);

        // Confirm model, making API key visible (auto-focuses API key editing)
        state.llm_setup.confirm_current_section();
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::ApiKey);
        assert!(state.llm_setup.api_key_editing); // auto-focused
        // Exit editing mode so Tab can navigate
        state.llm_setup.api_key_editing = false;

        // Tab from ApiKey wraps to Provider (Provider is always visible)
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
        assert_eq!(state.llm_setup.api_key_input.value(), "abc");

        handle_key(key(KeyCode::Backspace), &mut state);
        assert_eq!(state.llm_setup.api_key_input.value(), "ab");
    }

    #[test]
    fn llm_setup_api_key_enter_confirms_and_sends_set_api_key() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.llm_setup.active_section = LlmSetupSection::ApiKey;
        state.llm_setup.api_key_editing = true;
        state.llm_setup.api_key_input.set_value("sk-test-key");

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(!state.llm_setup.api_key_editing);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::SetApiKey(_)))
        ));
        // Confirming the API key should immediately set Testing status
        // so the spinner is visible while the backend runs the test.
        assert_eq!(
            state.llm_setup.connection_status,
            LlmConnectionStatus::Testing,
        );
        assert_eq!(
            state.llm_setup.confirmed_through,
            Some(LlmSetupSection::ApiKey),
        );
    }

    #[test]
    fn llm_setup_api_key_esc_goes_back_to_model() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        // Set up state as if user is on ApiKey step, editing
        state.llm_setup.active_section = LlmSetupSection::ApiKey;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::Model);
        state.llm_setup.api_key_editing = true;
        state.llm_setup.api_key_input.set_value("partial-key");

        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(!state.llm_setup.api_key_editing);
        // Should have navigated back to Model
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::Model);
        assert!(result.is_none());
    }

    #[test]
    fn llm_setup_enter_on_apikey_with_key_triggers_test() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.llm_setup.active_section = LlmSetupSection::ApiKey;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::Model);
        state.llm_setup.api_key_input.set_value("sk-test-123");

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::TestConnection))
        ));
    }

    #[test]
    fn llm_setup_enter_after_success_advances() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.llm_setup.active_section = LlmSetupSection::ApiKey;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::Model);
        state.llm_setup.api_key_input.set_value("sk-test-123");
        state.llm_setup.connection_status = LlmConnectionStatus::Success("ok".to_string());

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::GoNext))
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

    #[test]
    fn llm_setup_s_sends_skip() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        let result = handle_key(key(KeyCode::Char('s')), &mut state);
        assert_eq!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::Skip))
        );
    }

    #[test]
    fn strategy_setup_shift_s_sends_skip() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::strategy_setup::StrategyWizardStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        // Shift-S (Skip) only works in the Review step
        state.strategy_setup.step = StrategyWizardStep::Review;
        state.strategy_setup.input_editing = false;
        let result = handle_key(key(KeyCode::Char('S')), &mut state);
        assert_eq!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::Skip))
        );
    }

    // -- Strategy setup placeholder input tests --

    #[test]
    fn strategy_setup_input_step_enter_generates() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::strategy_setup::StrategyWizardStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        // Default step is Input with input_editing = true
        // Type some text first, then Enter while editing triggers generation
        state.strategy_setup.strategy_input.set_value("My strategy");
        let result = handle_key(key(KeyCode::Enter), &mut state);
        // Enter on Input step with text should send ConfigureStrategyWithLlm
        assert!(result.is_some());
        assert_eq!(state.strategy_setup.step, StrategyWizardStep::Generating);
    }

    #[test]
    fn strategy_setup_esc_stops_editing_then_goes_back() {
        use crate::onboarding::OnboardingStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        // Default state has input_editing = true; first Esc stops editing
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(!state.strategy_setup.input_editing);

        // Second Esc (not editing) sends GoBack
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(matches!(
            result,
            Some(UserCommand::OnboardingAction(OnboardingAction::GoBack))
        ));
    }

    #[test]
    fn strategy_setup_confirm_yes_sends_save_strategy_config() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::strategy_setup::StrategyWizardStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        // Put state in Confirm step with confirm_yes = true
        state.strategy_setup.step = StrategyWizardStep::Confirm;
        state.strategy_setup.confirm_yes = true;
        let result = handle_key(key(KeyCode::Enter), &mut state);
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
    fn settings_mode_1_2_switch_tabs() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;

        // Press '2' to switch to Strategy tab — dispatches to backend
        let result = handle_key(key(KeyCode::Char('2')), &mut state);
        assert_eq!(
            result,
            Some(UserCommand::SwitchSettingsTab(SettingsSection::StrategyConfig)),
        );

        // Press '1' to switch back to LLM tab — dispatches to backend
        let result = handle_key(key(KeyCode::Char('1')), &mut state);
        assert_eq!(
            result,
            Some(UserCommand::SwitchSettingsTab(SettingsSection::LlmConfig)),
        );
    }

    #[test]
    fn draft_mode_tab_switching_still_works() {
        let mut state = ViewState::default();
        state.app_mode = AppMode::Draft;
        state.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Analysis));
        let result = handle_key(key(KeyCode::Char('2')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.draft_screen.main_panel.active_tab(), TabId::Available);
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

    // -- Settings mode (Task 6) --

    #[test]
    fn draft_mode_comma_opens_settings() {
        let mut state = ViewState::default();
        state.app_mode = AppMode::Draft;
        let result = handle_key(key(KeyCode::Char(',')), &mut state);
        assert_eq!(result, Some(UserCommand::OpenSettings));
    }

    #[test]
    fn settings_llm_tab_delegates_to_llm_setup_handler() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.in_settings_mode = true;
        // In settings mode, all sections are visible and we start in overview
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_editing_field = None;

        // Down arrow in overview mode moves focus to next field (no command)
        state.llm_setup.active_section = LlmSetupSection::Provider;
        let result = handle_key(key(KeyCode::Down), &mut state);
        assert!(result.is_none(), "Down in overview navigates fields, no command");
        assert_eq!(state.llm_setup.active_section, LlmSetupSection::Model);

        // Enter opens the field's editor (snapshot + edit mode, no command)
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(result.is_none(), "Enter opens field editor, no command");
        assert_eq!(state.llm_setup.settings_editing_field, Some(LlmSetupSection::Model));

        // Down in field-editing mode changes the model selection
        let initial_idx = state.llm_setup.selected_model_idx;
        let result = handle_key(key(KeyCode::Down), &mut state);
        assert!(result.is_none(), "Down in field editing selects within list");
        assert_ne!(state.llm_setup.selected_model_idx, initial_idx);
    }

    #[test]
    fn settings_strategy_tab_delegates_to_strategy_handler() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::strategy_setup::StrategyWizardStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);
        state.settings_tab = SettingsSection::StrategyConfig;
        // In settings, the user has already completed the wizard;
        // strategy setup should be in Review step.
        state.strategy_setup.step = StrategyWizardStep::Review;
        state.strategy_setup.input_editing = false;

        // 's' in settings strategy tab directly dispatches SaveStrategyConfig
        let result = handle_key(key(KeyCode::Char('s')), &mut state);
        assert!(
            matches!(
                result,
                Some(UserCommand::OnboardingAction(
                    crate::protocol::OnboardingAction::SaveStrategyConfig { .. }
                ))
            ),
            "expected SaveStrategyConfig, got {:?}",
            result,
        );
        // settings_dirty should be cleared after save
        assert!(!state.strategy_setup.settings_dirty);
    }

    #[test]
    fn settings_filters_out_go_back_from_onboarding_handler() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;

        // In onboarding LLM setup, Esc maps to GoBack. In settings mode,
        // the settings handler intercepts Esc before it reaches the
        // onboarding handler, dispatching ExitSettings instead.
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert_eq!(
            result,
            Some(UserCommand::ExitSettings),
            "Esc in settings should dispatch ExitSettings, not GoBack",
        );
    }

    #[test]
    fn settings_filters_out_go_next_from_onboarding_handler() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;

        // 'n' in onboarding LLM setup maps to GoNext. In settings mode,
        // it should be filtered out (GoNext makes no sense in settings).
        // 'n' reaches the onboarding handler which returns GoNext,
        // then filter_onboarding_commands strips it.
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert!(
            result.is_none(),
            "'n' in settings should not dispatch GoNext, got {:?}",
            result,
        );
    }

    #[test]
    fn settings_api_key_editing_delegates_correctly() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.in_settings_mode = true;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_editing_field = Some(LlmSetupSection::ApiKey);
        state.llm_setup.api_key_editing = true;
        state.llm_setup.api_key_input.set_value("sk-");

        // Typing a character should append to the API key and mark dirty
        let result = handle_key(key(KeyCode::Char('a')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.llm_setup.api_key_input.value(), "sk-a");
        assert!(state.llm_setup.settings_dirty);

        // Enter confirms and dispatches SetApiKey to trigger a connection test
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(
            matches!(result, Some(UserCommand::OnboardingAction(OnboardingAction::SetApiKey(_)))),
            "Enter in settings API key editing should dispatch SetApiKey, got {:?}",
            result,
        );
        assert!(!state.llm_setup.api_key_editing);
        // Should return to overview mode
        assert!(state.llm_setup.settings_editing_field.is_none());
        // Dirty flag should still be set (unsaved)
        assert!(state.llm_setup.settings_dirty);
        // Save should be blocked until connection test passes
        assert!(state.llm_setup.settings_needs_connection_test);
        assert!(state.llm_setup.is_save_blocked());
        assert_eq!(state.llm_setup.connection_status, LlmConnectionStatus::Testing);
    }

    #[test]
    fn settings_api_key_unchanged_enter_does_not_trigger_test() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.in_settings_mode = true;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_editing_field = Some(LlmSetupSection::ApiKey);
        state.llm_setup.api_key_editing = true;
        // Set both input and saved to the same value
        state.llm_setup.api_key_input.set_value("sk-same-key");
        state.llm_setup.settings_saved_api_key = "sk-same-key".to_string();

        // Enter should confirm locally without dispatching SetApiKey
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(
            result.is_none(),
            "Enter with unchanged API key should not dispatch, got {:?}",
            result,
        );
        assert!(!state.llm_setup.api_key_editing);
        assert!(!state.llm_setup.settings_needs_connection_test);
        assert!(!state.llm_setup.is_save_blocked());
    }

    #[test]
    fn settings_save_blocked_while_connection_test_pending() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_dirty = true;
        state.llm_setup.settings_needs_connection_test = true;
        state.llm_setup.connection_status = LlmConnectionStatus::Testing;

        // 's' should be blocked (no command dispatched)
        let result = handle_key(key(KeyCode::Char('s')), &mut state);
        assert!(
            result.is_none(),
            "'s' should be blocked while connection test is pending, got {:?}",
            result,
        );
    }

    #[test]
    fn settings_save_blocked_after_connection_test_failure() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_dirty = true;
        state.llm_setup.settings_needs_connection_test = true;
        state.llm_setup.connection_status = LlmConnectionStatus::Failed("Invalid key".to_string());

        // 's' should be blocked
        let result = handle_key(key(KeyCode::Char('s')), &mut state);
        assert!(
            result.is_none(),
            "'s' should be blocked after connection test failure, got {:?}",
            result,
        );
    }

    #[test]
    fn settings_save_allowed_after_connection_test_success() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_dirty = true;
        state.llm_setup.settings_needs_connection_test = false; // cleared by success handler
        state.llm_setup.connection_status = LlmConnectionStatus::Success("ok".to_string());
        state.llm_setup.api_key_input.set_value("sk-valid-key");

        // 's' should be allowed
        let result = handle_key(key(KeyCode::Char('s')), &mut state);
        assert!(
            matches!(result, Some(UserCommand::OnboardingAction(OnboardingAction::SaveLlmConfig { .. }))),
            "'s' should dispatch SaveLlmConfig after successful test, got {:?}",
            result,
        );
    }

    #[test]
    fn settings_esc_shows_confirm_modal_when_dirty() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_dirty = true;
        state.llm_setup.settings_needs_connection_test = true;
        state.llm_setup.connection_status = LlmConnectionStatus::Failed("bad key".to_string());
        state.llm_setup.api_key_input.set_value("sk-bad-key");
        state.llm_setup.settings_saved_api_key = "sk-original".to_string();
        state.llm_setup.settings_saved_provider_idx = 0;
        state.llm_setup.settings_saved_model_idx = 0;

        // Esc should show confirmation modal, not exit
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert_eq!(result, None);
        assert!(state.confirm_exit_settings.open);

        // 'n' should discard, revert, and exit
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert_eq!(result, Some(UserCommand::ExitSettings));
        assert!(!state.confirm_exit_settings.open);
        assert!(!state.llm_setup.settings_needs_connection_test);
        assert!(!state.llm_setup.is_save_blocked());
        assert_eq!(state.llm_setup.connection_status, LlmConnectionStatus::Untested);
    }

    #[test]
    fn settings_provider_change_triggers_connection_test() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.in_settings_mode = true;

        // Set an API key so the test can be triggered
        state.llm_setup.api_key_input.set_value("sk-test-key");

        // Snapshot the initial state (provider idx 0, model idx 0)
        state.llm_setup.snapshot_settings();

        // Enter overview -> open Provider dropdown
        state.llm_setup.active_section = LlmSetupSection::Provider;
        let _ = handle_key(key(KeyCode::Enter), &mut state);
        assert_eq!(
            state.llm_setup.settings_editing_field,
            Some(LlmSetupSection::Provider),
        );

        // Move provider down to a different value
        let _ = handle_key(key(KeyCode::Down), &mut state);
        assert_ne!(state.llm_setup.selected_provider_idx, 0);
        assert!(state.llm_setup.settings_dirty);

        // Confirm provider — in settings mode, this should immediately
        // trigger a connection test and return to overview (not advance
        // to the Model field)
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(
            matches!(
                result,
                Some(UserCommand::OnboardingAction(OnboardingAction::SetApiKey(_)))
            ),
            "Provider change should trigger connection test via SetApiKey, got {:?}",
            result,
        );
        assert!(state.llm_setup.settings_needs_connection_test);
        assert!(state.llm_setup.is_save_blocked());
        assert_eq!(state.llm_setup.connection_status, LlmConnectionStatus::Testing);
        // Should return to overview mode, not advance to Model
        assert_eq!(state.llm_setup.settings_editing_field, None);
    }

    #[test]
    fn settings_model_change_triggers_connection_test() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.in_settings_mode = true;

        // Set an API key so the test can be triggered
        state.llm_setup.api_key_input.set_value("sk-test-key");

        // Snapshot the initial state
        state.llm_setup.snapshot_settings();

        // Open Model dropdown directly from overview
        state.llm_setup.active_section = LlmSetupSection::Model;
        let _ = handle_key(key(KeyCode::Enter), &mut state);
        assert_eq!(
            state.llm_setup.settings_editing_field,
            Some(LlmSetupSection::Model),
        );

        // Move model down to a different value
        let _ = handle_key(key(KeyCode::Down), &mut state);
        assert!(state.llm_setup.settings_dirty);

        // Confirm model — in settings mode, this should immediately
        // trigger a connection test and return to overview (not advance
        // to ApiKey editing)
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(
            matches!(
                result,
                Some(UserCommand::OnboardingAction(OnboardingAction::SetApiKey(_)))
            ),
            "Model change should trigger connection test via SetApiKey, got {:?}",
            result,
        );
        assert!(state.llm_setup.settings_needs_connection_test);
        assert!(state.llm_setup.is_save_blocked());
        assert_eq!(state.llm_setup.connection_status, LlmConnectionStatus::Testing);
        // Should return to overview mode, not advance to ApiKey
        assert_eq!(state.llm_setup.settings_editing_field, None);
        assert!(!state.llm_setup.api_key_editing);
    }

    #[test]
    fn settings_provider_reverted_to_original_clears_test_requirement() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.in_settings_mode = true;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);

        // Saved state: provider=0, model=0, key="sk-same"
        state.llm_setup.api_key_input.set_value("sk-same");
        state.llm_setup.snapshot_settings();

        // Simulate the user editing and ending up back at original values.
        // Provider, model, and key all match the saved snapshot.
        state.llm_setup.selected_provider_idx = 0;
        state.llm_setup.selected_model_idx = 0;
        state.llm_setup.api_key_input.set_value("sk-same");

        // Put into API key editing mode as if the user just went through the flow
        state.llm_setup.settings_editing_field = Some(LlmSetupSection::ApiKey);
        state.llm_setup.api_key_editing = true;

        // Confirm API key — everything matches saved state, so no test needed
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(
            result.is_none(),
            "All fields matching saved state should not trigger test, got {:?}",
            result,
        );
        assert!(!state.llm_setup.settings_needs_connection_test);
        assert!(!state.llm_setup.is_save_blocked());
        assert_eq!(state.llm_setup.connection_status, LlmConnectionStatus::Untested);
    }

    #[test]
    fn settings_provider_change_with_saved_key_triggers_test_connection() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.in_settings_mode = true;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);

        // A saved key exists on disk but the input field is empty (masked display)
        state.llm_setup.has_saved_api_key = true;
        state.llm_setup.saved_api_key_mask = "sk-ant-*****6789".to_string();
        state.llm_setup.snapshot_settings();

        // Change the provider
        state.llm_setup.selected_provider_idx = 1;

        // Put into API key editing mode
        state.llm_setup.settings_editing_field = Some(LlmSetupSection::ApiKey);
        state.llm_setup.api_key_editing = true;
        // Key input is empty (user didn't type a new key)

        // Confirm — provider changed but key is empty with saved key on disk.
        // Should dispatch TestConnection (not SetApiKey) to test with persisted key.
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(
            matches!(
                result,
                Some(UserCommand::OnboardingAction(OnboardingAction::TestConnection))
            ),
            "Provider change with empty key input but saved key should dispatch TestConnection, got {:?}",
            result,
        );
        assert!(state.llm_setup.settings_needs_connection_test);
        assert!(state.llm_setup.is_save_blocked());
        assert_eq!(state.llm_setup.connection_status, LlmConnectionStatus::Testing);
    }

    #[test]
    fn settings_provider_dropdown_with_saved_key_triggers_test_immediately() {
        // In settings mode, confirming a provider change from the dropdown
        // should immediately trigger a connection test (using the saved key)
        // and return to overview mode — without advancing to Model.
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.in_settings_mode = true;

        // A saved key exists on disk but the input field is empty
        state.llm_setup.has_saved_api_key = true;
        state.llm_setup.saved_api_key_mask = "sk-ant-*****6789".to_string();
        state.llm_setup.snapshot_settings();

        // Open Provider dropdown
        state.llm_setup.active_section = LlmSetupSection::Provider;
        let _ = handle_key(key(KeyCode::Enter), &mut state);

        // Change provider
        let _ = handle_key(key(KeyCode::Down), &mut state);
        assert_ne!(state.llm_setup.selected_provider_idx, 0);

        // Confirm — should trigger TestConnection immediately
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(
            matches!(
                result,
                Some(UserCommand::OnboardingAction(OnboardingAction::TestConnection))
            ),
            "Provider change from dropdown with saved key should dispatch TestConnection, got {:?}",
            result,
        );
        assert!(state.llm_setup.settings_needs_connection_test);
        assert_eq!(state.llm_setup.connection_status, LlmConnectionStatus::Testing);
        assert_eq!(state.llm_setup.settings_editing_field, None);
    }

    #[test]
    fn settings_provider_unchanged_returns_to_overview_no_test() {
        // When the user opens the provider dropdown but doesn't change
        // anything, Enter should return to overview without triggering a test.
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.in_settings_mode = true;
        state.llm_setup.api_key_input.set_value("sk-test-key");
        state.llm_setup.snapshot_settings();

        // Open Provider dropdown
        state.llm_setup.active_section = LlmSetupSection::Provider;
        let _ = handle_key(key(KeyCode::Enter), &mut state);

        // Confirm without changing — should return to overview with no test
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(result.is_none(), "No change should not trigger test, got {:?}", result);
        assert!(!state.llm_setup.settings_needs_connection_test);
        assert_eq!(state.llm_setup.connection_status, LlmConnectionStatus::Untested);
        assert_eq!(state.llm_setup.settings_editing_field, None);
    }

    // -- Strategy overview editing tests --

    #[test]
    fn strategy_review_enter_on_overview_starts_editing() {
        use crate::tui::onboarding::strategy_setup::{ReviewSection, StrategyWizardStep};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(crate::protocol::SettingsSection::StrategyConfig);
        state.settings_tab = crate::protocol::SettingsSection::StrategyConfig;
        state.strategy_setup.step = StrategyWizardStep::Review;
        state.strategy_setup.input_editing = false;
        state.strategy_setup.review_section = ReviewSection::Overview;
        state.strategy_setup.strategy_overview = "Stars-and-scrubs approach.".to_string();

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(result.is_none(), "Enter on overview starts editing, no command");
        assert!(state.strategy_setup.overview_editing);
        assert_eq!(state.strategy_setup.overview_input.value(), "Stars-and-scrubs approach.");
    }

    #[test]
    fn strategy_overview_editing_esc_cancels() {
        use crate::tui::onboarding::strategy_setup::{ReviewSection, StrategyWizardStep};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(crate::protocol::SettingsSection::StrategyConfig);
        state.settings_tab = crate::protocol::SettingsSection::StrategyConfig;
        state.strategy_setup.step = StrategyWizardStep::Review;
        state.strategy_setup.input_editing = false;
        state.strategy_setup.review_section = ReviewSection::Overview;
        state.strategy_setup.overview_editing = true;
        state.strategy_setup.overview_input.set_value("Modified text");

        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(!state.strategy_setup.overview_editing);
        assert!(state.strategy_setup.overview_input.is_empty());
    }

    #[test]
    fn strategy_overview_editing_enter_submits_to_llm() {
        use crate::tui::onboarding::strategy_setup::{ReviewSection, StrategyWizardStep};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(crate::protocol::SettingsSection::StrategyConfig);
        state.settings_tab = crate::protocol::SettingsSection::StrategyConfig;
        state.strategy_setup.step = StrategyWizardStep::Review;
        state.strategy_setup.input_editing = false;
        state.strategy_setup.review_section = ReviewSection::Overview;
        state.strategy_setup.overview_editing = true;
        state.strategy_setup.overview_input.set_value("Punt saves, target BB and HD");

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(
            matches!(
                result,
                Some(UserCommand::OnboardingAction(
                    OnboardingAction::ConfigureStrategyWithLlm(_)
                ))
            ),
            "Enter on overview editing submits to LLM, got {:?}",
            result,
        );
        assert!(!state.strategy_setup.overview_editing);
        assert!(state.strategy_setup.generating);
    }

    #[test]
    fn strategy_generating_esc_cancels_back_to_overview_editing() {
        use crate::tui::onboarding::strategy_setup::{ReviewSection, StrategyWizardStep};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(crate::protocol::SettingsSection::StrategyConfig);
        state.settings_tab = crate::protocol::SettingsSection::StrategyConfig;
        state.strategy_setup.step = StrategyWizardStep::Review;
        state.strategy_setup.input_editing = false;
        state.strategy_setup.review_section = ReviewSection::Overview;
        state.strategy_setup.generating = true;

        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(!state.strategy_setup.generating);
        assert!(state.strategy_setup.overview_editing);
    }

    #[test]
    fn strategy_settings_esc_shows_confirm_modal_when_dirty() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::strategy_setup::StrategyWizardStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);
        state.settings_tab = SettingsSection::StrategyConfig;
        state.strategy_setup.step = StrategyWizardStep::Review;
        state.strategy_setup.input_editing = false;

        // Snapshot original values
        state.strategy_setup.strategy_overview = "Original".to_string();
        state.strategy_setup.hitting_budget_pct = 65;
        state.strategy_setup.snapshot_settings();

        // Modify values
        state.strategy_setup.strategy_overview = "Modified".to_string();
        state.strategy_setup.hitting_budget_pct = 80;
        state.strategy_setup.settings_dirty = true;

        // Esc should show confirmation modal, not exit
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert_eq!(result, None);
        assert!(state.confirm_exit_settings.open);

        // 'n' should discard, restore, and exit
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert_eq!(result, Some(UserCommand::ExitSettings));
        assert!(!state.confirm_exit_settings.open);
        assert_eq!(state.strategy_setup.strategy_overview, "Original");
        assert_eq!(state.strategy_setup.hitting_budget_pct, 65);
        assert!(!state.strategy_setup.settings_dirty);
    }

    #[test]
    fn strategy_overview_editing_typing_appends() {
        use crate::tui::onboarding::strategy_setup::{ReviewSection, StrategyWizardStep};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(crate::protocol::SettingsSection::StrategyConfig);
        state.settings_tab = crate::protocol::SettingsSection::StrategyConfig;
        state.strategy_setup.step = StrategyWizardStep::Review;
        state.strategy_setup.input_editing = false;
        state.strategy_setup.review_section = ReviewSection::Overview;
        state.strategy_setup.overview_editing = true;
        state.strategy_setup.overview_input.set_value("Punt");

        let result = handle_key(key(KeyCode::Char(' ')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.strategy_setup.overview_input.value(), "Punt ");
    }

    // -----------------------------------------------------------------------
    // Unsaved changes confirmation modal tests
    // -----------------------------------------------------------------------

    #[test]
    fn confirm_exit_settings_esc_cancels_modal() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.confirm_exit_settings.open = true;

        // Esc should dismiss the modal and return to settings
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert_eq!(result, None);
        assert!(!state.confirm_exit_settings.open);
    }

    #[test]
    fn confirm_exit_settings_y_saves_and_exits() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_dirty = true;
        state.llm_setup.api_key_input.set_value("sk-test");
        state.confirm_exit_settings.open = true;

        let result = handle_key(key(KeyCode::Char('y')), &mut state);
        assert!(
            matches!(result, Some(UserCommand::SaveAndExitSettings { llm: Some(_), strategy: None })),
            "expected SaveAndExitSettings with LLM save, got {:?}",
            result,
        );
        assert!(!state.confirm_exit_settings.open);
        assert!(!state.llm_setup.settings_dirty);
    }

    #[test]
    fn confirm_exit_settings_y_saves_both_tabs() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_dirty = true;
        state.llm_setup.api_key_input.set_value("sk-test");
        state.strategy_setup.settings_dirty = true;
        state.strategy_setup.strategy_overview = "My strategy".to_string();
        state.confirm_exit_settings.open = true;

        let result = handle_key(key(KeyCode::Char('y')), &mut state);
        assert!(
            matches!(result, Some(UserCommand::SaveAndExitSettings { llm: Some(_), strategy: Some(_) })),
            "expected SaveAndExitSettings with both saves, got {:?}",
            result,
        );
        assert!(!state.confirm_exit_settings.open);
        assert!(!state.llm_setup.settings_dirty);
        assert!(!state.strategy_setup.settings_dirty);
    }

    #[test]
    fn confirm_exit_settings_n_discards_both_tabs() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);
        state.settings_tab = SettingsSection::StrategyConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_dirty = true;
        state.llm_setup.settings_saved_provider_idx = 0;
        state.llm_setup.settings_saved_model_idx = 0;

        state.strategy_setup.strategy_overview = "Original".to_string();
        state.strategy_setup.hitting_budget_pct = 65;
        state.strategy_setup.snapshot_settings();
        state.strategy_setup.strategy_overview = "Modified".to_string();
        state.strategy_setup.settings_dirty = true;

        state.confirm_exit_settings.open = true;

        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert_eq!(result, Some(UserCommand::ExitSettings));
        assert!(!state.confirm_exit_settings.open);
        assert!(!state.llm_setup.settings_dirty);
        assert!(!state.strategy_setup.settings_dirty);
        assert_eq!(state.strategy_setup.strategy_overview, "Original");
    }

    #[test]
    fn confirm_exit_settings_blocks_other_keys() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.confirm_exit_settings.open = true;

        // Random keys should be ignored
        let result = handle_key(key(KeyCode::Char('a')), &mut state);
        assert_eq!(result, None);
        assert!(state.confirm_exit_settings.open);

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert_eq!(result, None);
        assert!(state.confirm_exit_settings.open);
    }

    #[test]
    fn settings_esc_exits_immediately_when_clean() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        // Both tabs are clean
        state.llm_setup.settings_dirty = false;
        state.llm_setup.settings_needs_connection_test = false;
        state.strategy_setup.settings_dirty = false;

        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert_eq!(result, Some(UserCommand::ExitSettings));
        assert!(!state.confirm_exit_settings.open);
    }

    #[test]
    fn strategy_settings_esc_exits_immediately_when_clean() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::strategy_setup::StrategyWizardStep;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);
        state.settings_tab = SettingsSection::StrategyConfig;
        state.strategy_setup.step = StrategyWizardStep::Review;
        state.strategy_setup.input_editing = false;
        state.strategy_setup.settings_dirty = false;
        state.llm_setup.settings_dirty = false;

        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert_eq!(result, Some(UserCommand::ExitSettings));
        assert!(!state.confirm_exit_settings.open);
    }

    #[test]
    fn confirm_exit_settings_uppercase_y_works() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);
        state.settings_tab = SettingsSection::StrategyConfig;
        state.strategy_setup.settings_dirty = true;
        state.strategy_setup.strategy_overview = "Test".to_string();
        state.confirm_exit_settings.open = true;

        let result = handle_key(key(KeyCode::Char('Y')), &mut state);
        assert!(
            matches!(result, Some(UserCommand::SaveAndExitSettings { .. })),
            "uppercase Y should save, got {:?}",
            result,
        );
    }

    #[test]
    fn confirm_exit_settings_uppercase_n_works() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);
        state.settings_tab = SettingsSection::StrategyConfig;
        state.strategy_setup.settings_dirty = true;
        state.confirm_exit_settings.open = true;

        let result = handle_key(key(KeyCode::Char('N')), &mut state);
        assert_eq!(result, Some(UserCommand::ExitSettings));
    }

    #[test]
    fn confirm_exit_settings_y_skips_llm_save_when_blocked() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::LlmSetupSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_dirty = true;
        state.llm_setup.settings_needs_connection_test = true;
        // connection_status defaults to Untested, so is_save_blocked() == true
        state.llm_setup.api_key_input.set_value("sk-test");
        state.confirm_exit_settings.open = true;

        // Snapshot original values so restore has something to go back to
        state.llm_setup.settings_saved_provider_idx = state.llm_setup.selected_provider_idx;
        state.llm_setup.settings_saved_model_idx = state.llm_setup.selected_model_idx;

        let result = handle_key(key(KeyCode::Char('y')), &mut state);
        // LLM save should be skipped because save is blocked
        assert!(
            matches!(result, Some(UserCommand::SaveAndExitSettings { llm: None, strategy: None })),
            "expected SaveAndExitSettings with no LLM save when blocked, got {:?}",
            result,
        );
        assert!(!state.confirm_exit_settings.open);
        // restore_settings_snapshot should have been called, clearing dirty flags
        assert!(!state.llm_setup.settings_dirty);
        assert!(!state.llm_setup.settings_needs_connection_test);
    }

    #[test]
    fn confirm_exit_settings_y_saves_llm_when_dirty_and_not_blocked() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.llm_setup.settings_dirty = true;
        state.llm_setup.settings_needs_connection_test = true;
        // Connection test passed, so is_save_blocked() == false
        state.llm_setup.connection_status =
            LlmConnectionStatus::Success("ok".to_string());
        state.llm_setup.api_key_input.set_value("sk-test");
        state.confirm_exit_settings.open = true;

        let result = handle_key(key(KeyCode::Char('y')), &mut state);
        // LLM save should proceed because connection test passed
        assert!(
            matches!(result, Some(UserCommand::SaveAndExitSettings { llm: Some(_), strategy: None })),
            "expected SaveAndExitSettings with LLM save when not blocked, got {:?}",
            result,
        );
        assert!(!state.confirm_exit_settings.open);
        assert!(!state.llm_setup.settings_dirty);
    }
}
