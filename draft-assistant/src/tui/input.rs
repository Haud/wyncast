// Keyboard input handling and command dispatch.
//
// Translates crossterm key events into UserCommand messages sent to the
// app orchestrator, or into local ViewState mutations (e.g. tab switching,
// scroll, filtering).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::protocol::{AppMode, OnboardingAction, TabFeature, TabId, UserCommand};
use crate::tui::draft::draft_log::DraftLogMessage;
use crate::tui::draft::main_panel::analysis::AnalysisPanelMessage;
use crate::tui::draft::main_panel::available::AvailablePanelMessage;
use crate::tui::draft::sidebar::plan::PlanPanelMessage;
use crate::tui::draft::sidebar::roster::RosterMessage;
use crate::tui::draft::sidebar::scarcity::ScarcityPanelMessage;
use crate::tui::draft::teams::TeamsMessage;
use crate::tui::scroll::ScrollDirection;
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
        AppMode::Draft => handle_draft_key(key_event, view_state),
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

    view_state.available_panel.filter_mode()
        || view_state.llm_setup.api_key_editing
        || strategy_editing
        || view_state.position_filter_modal.open
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
            handle_llm_setup_key(key_event, view_state, false)
        }
        AppMode::Onboarding(OnboardingStep::StrategySetup) |
        AppMode::Onboarding(OnboardingStep::Complete) => {
            handle_strategy_setup_key(key_event, view_state)
        }
        _ => None,
    }
}

/// Handle keyboard input on the strategy setup wizard (onboarding step 2).
///
/// Dispatches to step-specific handlers based on the current wizard step:
/// - Input: text editing for strategy description
/// - Generating: wait for LLM, handle errors
/// - Review: navigate/edit budget and category weights
/// - Confirm: Yes/No selection
fn handle_strategy_setup_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    use super::onboarding::strategy_setup::{
        ReviewSection, StrategyWizardStep, CATEGORIES, WEIGHT_COLS,
    };

    let state = &mut view_state.strategy_setup;

    match state.step {
        // ----- Step 1: Input -----
        StrategyWizardStep::Input => {
            if state.input_editing {
                // Text editing mode
                return match key_event.code {
                    KeyCode::Esc => {
                        state.input_editing = false;
                        None
                    }
                    KeyCode::Enter => {
                        // Submit text to LLM if non-empty
                        if !state.strategy_input.value().trim().is_empty() {
                            state.input_editing = false;
                            state.step = StrategyWizardStep::Generating;
                            state.generating = true;
                            state.generation_output.clear();
                            state.generation_error = None;
                            let text = state.strategy_input.value().to_string();
                            Some(UserCommand::OnboardingAction(
                                OnboardingAction::ConfigureStrategyWithLlm(text),
                            ))
                        } else {
                            None
                        }
                    }
                    _ => {
                        if let Some(msg) = super::TextInput::key_to_message(&key_event) {
                            state.strategy_input.update(msg);
                        }
                        None
                    }
                };
            }

            // Not editing text
            match key_event.code {
                KeyCode::Enter => {
                    // Send to LLM if there's text
                    if !state.strategy_input.value().trim().is_empty() {
                        state.step = StrategyWizardStep::Generating;
                        state.generating = true;
                        state.generation_output.clear();
                        state.generation_error = None;
                        let text = state.strategy_input.value().to_string();
                        Some(UserCommand::OnboardingAction(
                            OnboardingAction::ConfigureStrategyWithLlm(text),
                        ))
                    } else {
                        // No text, enter edit mode
                        state.input_editing = true;
                        None
                    }
                }
                KeyCode::Char('e') => {
                    state.input_editing = true;
                    None
                }
                KeyCode::Esc => {
                    Some(UserCommand::OnboardingAction(OnboardingAction::GoBack))
                }
                KeyCode::Char('q') => Some(UserCommand::Quit),
                _ => None,
            }
        }

        // ----- Step 2: Generating -----
        StrategyWizardStep::Generating => {
            // If there's an error, allow retry or go back
            if state.generation_error.is_some() {
                match key_event.code {
                    KeyCode::Enter => {
                        // Retry
                        state.generating = true;
                        state.generation_output.clear();
                        state.generation_error = None;
                        let text = state.strategy_input.value().to_string();
                        Some(UserCommand::OnboardingAction(
                            OnboardingAction::ConfigureStrategyWithLlm(text),
                        ))
                    }
                    KeyCode::Esc => {
                        // Go back to input
                        state.step = StrategyWizardStep::Input;
                        state.input_editing = true;
                        state.generating = false;
                        state.generation_error = None;
                        None
                    }
                    KeyCode::Char('q') => Some(UserCommand::Quit),
                    _ => None,
                }
            } else {
                // Still generating, no input allowed except quit
                match key_event.code {
                    KeyCode::Char('q') => Some(UserCommand::Quit),
                    _ => None,
                }
            }
        }

        // ----- Step 3: Review -----
        StrategyWizardStep::Review => {
            // Overview editing mode (text input for strategy overview)
            if state.overview_editing {
                return match key_event.code {
                    KeyCode::Enter => {
                        // Submit overview text to LLM for regeneration
                        let text = state.overview_input.value().to_string();
                        if !text.trim().is_empty() {
                            state.overview_editing = false;
                            state.generating = true;
                            state.generation_output.clear();
                            state.generation_error = None;
                            // Copy the edited text as the strategy input for the LLM
                            state.strategy_input.set_value(&text);
                            Some(UserCommand::OnboardingAction(
                                OnboardingAction::ConfigureStrategyWithLlm(text),
                            ))
                        } else {
                            None
                        }
                    }
                    KeyCode::Esc => {
                        state.cancel_overview_editing();
                        None
                    }
                    _ => {
                        if let Some(msg) = super::TextInput::key_to_message(&key_event) {
                            state.overview_input.update(msg);
                        }
                        None
                    }
                };
            }

            // Generating mode within review (LLM regenerating strategy)
            if state.generating {
                return match key_event.code {
                    KeyCode::Esc => {
                        state.generating = false;
                        state.generation_error = None;
                        state.start_overview_editing();
                        None
                    }
                    KeyCode::Enter if state.generation_error.is_some() => {
                        // Retry generation
                        state.generating = true;
                        state.generation_output.clear();
                        state.generation_error = None;
                        let text = state.strategy_input.value().to_string();
                        Some(UserCommand::OnboardingAction(
                            OnboardingAction::ConfigureStrategyWithLlm(text),
                        ))
                    }
                    KeyCode::Char('q') => Some(UserCommand::Quit),
                    _ => None,
                };
            }

            // Error state: LLM generation failed (generating is false, but error is set)
            if state.generation_error.is_some() {
                return match key_event.code {
                    KeyCode::Esc => {
                        state.generation_error = None;
                        state.start_overview_editing();
                        None
                    }
                    KeyCode::Enter => {
                        // Retry: resubmit the last input
                        let text = state.strategy_input.value().to_string();
                        state.generation_error = None;
                        state.generating = true;
                        state.generation_output.clear();
                        Some(UserCommand::OnboardingAction(
                            OnboardingAction::ConfigureStrategyWithLlm(text),
                        ))
                    }
                    KeyCode::Char('q') => Some(UserCommand::Quit),
                    _ => None,
                };
            }

            // Numeric field editing mode
            if state.editing_field.is_some() {
                return match key_event.code {
                    KeyCode::Enter => {
                        if state.confirm_edit() {
                            state.settings_dirty = true;
                        }
                        None
                    }
                    KeyCode::Esc => {
                        state.cancel_edit();
                        None
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                        state.field_input.insert_char(c);
                        None
                    }
                    KeyCode::Char(_) => None,
                    _ => {
                        if let Some(msg) = super::TextInput::key_to_message(&key_event) {
                            state.field_input.update(msg);
                        }
                        None
                    }
                };
            }

            // Normal review navigation
            match key_event.code {
                // Up/Down: move between sections naturally
                KeyCode::Up | KeyCode::Char('k') => {
                    match state.review_section {
                        ReviewSection::Overview => {} // already at top
                        ReviewSection::BudgetField => {
                            state.review_section = ReviewSection::Overview;
                        }
                        ReviewSection::CategoryWeights => {
                            // If in top row of grid, move up to budget
                            if state.selected_weight_idx < WEIGHT_COLS {
                                state.review_section = ReviewSection::BudgetField;
                            } else {
                                state.weight_up();
                            }
                        }
                    }
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    match state.review_section {
                        ReviewSection::Overview => {
                            state.review_section = ReviewSection::BudgetField;
                        }
                        ReviewSection::BudgetField => {
                            state.review_section = ReviewSection::CategoryWeights;
                        }
                        ReviewSection::CategoryWeights => {
                            state.weight_down();
                        }
                    }
                    None
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if state.review_section == ReviewSection::CategoryWeights {
                        state.weight_left();
                    }
                    None
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    if state.review_section == ReviewSection::CategoryWeights {
                        state.weight_right();
                    }
                    None
                }
                // Enter: edit field or advance to confirm
                KeyCode::Enter => {
                    match state.review_section {
                        ReviewSection::Overview => {
                            // Enter editing mode for strategy overview
                            state.start_overview_editing();
                            None
                        }
                        ReviewSection::BudgetField => {
                            let current = format!("{}", state.hitting_budget_pct);
                            state.start_editing("budget", &current);
                            None
                        }
                        ReviewSection::CategoryWeights => {
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
                // s: save (advance to confirm)
                KeyCode::Char('s') => {
                    state.step = StrategyWizardStep::Confirm;
                    state.confirm_yes = true;
                    None
                }
                // S: skip this step
                KeyCode::Char('S') => {
                    Some(UserCommand::OnboardingAction(OnboardingAction::Skip))
                }
                // Esc: go back to Input step (keep values)
                KeyCode::Esc => {
                    state.step = StrategyWizardStep::Input;
                    state.input_editing = true;
                    None
                }
                KeyCode::Char('q') => Some(UserCommand::Quit),
                _ => None,
            }
        }

        // ----- Step 4: Confirm -----
        StrategyWizardStep::Confirm => {
            match key_event.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
                    state.confirm_yes = !state.confirm_yes;
                    None
                }
                // 'y' is an absolute shortcut: always confirms regardless of button selection
                KeyCode::Char('y') => {
                    let weights = state.category_weights.clone();
                    let pct = state.hitting_budget_pct;
                    let overview = if state.strategy_overview.is_empty() {
                        None
                    } else {
                        Some(state.strategy_overview.clone())
                    };
                    Some(UserCommand::OnboardingAction(
                        OnboardingAction::SaveStrategyConfig {
                            hitting_budget_pct: pct,
                            category_weights: weights,
                            strategy_overview: overview,
                        },
                    ))
                }
                // 'n' is an absolute shortcut: always goes back regardless of button selection
                KeyCode::Char('n') => {
                    state.step = StrategyWizardStep::Review;
                    None
                }
                // Enter confirms whichever button is currently selected
                KeyCode::Enter if state.confirm_yes => {
                    let weights = state.category_weights.clone();
                    let pct = state.hitting_budget_pct;
                    let overview = if state.strategy_overview.is_empty() {
                        None
                    } else {
                        Some(state.strategy_overview.clone())
                    };
                    Some(UserCommand::OnboardingAction(
                        OnboardingAction::SaveStrategyConfig {
                            hitting_budget_pct: pct,
                            category_weights: weights,
                            strategy_overview: overview,
                        },
                    ))
                }
                KeyCode::Enter if !state.confirm_yes => {
                    // Go back to review
                    state.step = StrategyWizardStep::Review;
                    None
                }
                KeyCode::Esc => {
                    // Go back to review
                    state.step = StrategyWizardStep::Review;
                    None
                }
                KeyCode::Char('q') => Some(UserCommand::Quit),
                _ => None,
            }
        }
    }
}

/// Handle keyboard input on the LLM setup screen (onboarding step 1).
///
/// Uses progressive disclosure: sections are revealed one at a time as each
/// is confirmed via Enter. Input handling depends on the active section and
/// whether the API key text input is in edit mode:
/// - When editing API key: captures typed characters, Enter confirms and
///   triggers a connection test, Esc restores the backup and navigates back
/// - Provider/Model sections: Up/Down select within lists, Enter confirms
///   the current section and reveals the next
/// - ApiKey section (not editing): Enter behavior is context-sensitive based
///   on connection status (input key / test / edit key / continue)
///
/// Provider and model selections dispatch `SetProvider`/`SetModel` commands to
/// the app orchestrator immediately on each arrow key press. This keeps
/// `OnboardingProgress` in sync so that when `GoNext` fires, the app already
/// has the correct values and only needs to persist the API key and advance.
fn handle_llm_setup_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
    settings_mode: bool,
) -> Option<UserCommand> {
    use super::onboarding::llm_setup::{LlmConnectionStatus, LlmSetupSection};

    let state = &mut view_state.llm_setup;

    // --- API key editing mode ---
    if state.api_key_editing {
        return match key_event.code {
            KeyCode::Enter => {
                state.api_key_editing = false;
                // Sync the key to the backend on confirm. The backend will
                // automatically trigger a connection test after receiving
                // SetApiKey, so we set Testing status here for immediate
                // visual feedback.
                let key = state.api_key_input.value().to_string();
                if key.is_empty() {
                    None
                } else {
                    state.confirmed_through = Some(LlmSetupSection::ApiKey);
                    state.connection_status = LlmConnectionStatus::Testing;
                    Some(UserCommand::OnboardingAction(OnboardingAction::SetApiKey(key)))
                }
            }
            KeyCode::Esc => {
                // Esc while editing: go back to Model (linear back navigation)
                state.api_key_input.set_value(&state.api_key_backup.clone());
                state.api_key_editing = false;
                if settings_mode {
                    // In settings mode, just exit editing, don't navigate back
                    None
                } else {
                    state.go_back_section();
                    None
                }
            }
            _ => {
                if let Some(msg) = super::TextInput::key_to_message(&key_event) {
                    state.api_key_input.update(msg);
                }
                None
            }
        };
    }

    // --- Normal navigation mode ---
    match key_event.code {
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
        // Enter: progressive disclosure — confirm current section and reveal next
        KeyCode::Enter => {
            match state.active_section {
                LlmSetupSection::Provider => {
                    // Confirm provider, reveal model list
                    let provider = state.selected_provider().clone();
                    state.confirm_current_section();
                    Some(UserCommand::OnboardingAction(
                        OnboardingAction::SetProvider(provider),
                    ))
                }
                LlmSetupSection::Model => {
                    // Confirm model, reveal API key input
                    let model_id = state
                        .selected_model()
                        .map(|m| m.model_id.to_string())
                        .unwrap_or_default();
                    state.confirm_current_section();
                    Some(UserCommand::OnboardingAction(
                        OnboardingAction::SetModel(model_id),
                    ))
                }
                LlmSetupSection::ApiKey => {
                    if state.connection_tested_ok() {
                        // Connection test passed — advance to next step
                        Some(UserCommand::OnboardingAction(OnboardingAction::GoNext))
                    } else if state.api_key_input.is_empty() {
                        // Enter edit mode if no key entered yet
                        state.api_key_backup = state.api_key_input.value().to_string();
                        state.api_key_editing = true;
                        None
                    } else if matches!(state.connection_status, LlmConnectionStatus::Failed(_)) {
                        // Connection test failed — re-enter editing so the user
                        // can fix their key rather than re-triggering the same
                        // failing test.
                        state.api_key_backup = state.api_key_input.value().to_string();
                        state.api_key_editing = true;
                        state.connection_status = LlmConnectionStatus::Untested;
                        None
                    } else {
                        // Key already entered — trigger connection test
                        state.confirmed_through = Some(LlmSetupSection::ApiKey);
                        state.connection_status = LlmConnectionStatus::Testing;
                        Some(UserCommand::OnboardingAction(
                            OnboardingAction::TestConnection,
                        ))
                    }
                }
            }
        }
        // n: no longer used for advancing (Enter handles it now), but keep
        // for backward compatibility in case muscle memory persists
        KeyCode::Char('n') => {
            if state.connection_tested_ok() {
                Some(UserCommand::OnboardingAction(OnboardingAction::GoNext))
            } else {
                None
            }
        }
        // s: skip this step (always available)
        KeyCode::Char('s') => {
            Some(UserCommand::OnboardingAction(OnboardingAction::Skip))
        }
        // Esc: go back to previous section, or go back in onboarding if at first section
        KeyCode::Esc => {
            if settings_mode {
                // In settings mode, all sections are always visible.
                // Esc just moves focus to the previous section without
                // un-confirming anything (preserves the "all visible" invariant).
                if state.active_section != LlmSetupSection::Provider {
                    state.active_section = state.active_section.prev();
                }
                None
            } else if state.active_section == LlmSetupSection::Provider {
                // At the first section — propagate GoBack to onboarding flow
                Some(UserCommand::OnboardingAction(OnboardingAction::GoBack))
            } else {
                // Go back to previous section within LLM setup
                state.go_back_section();
                None
            }
        }
        // Tab: advance to next section (only within visible/confirmed sections)
        KeyCode::Tab => {
            let next = state.active_section.next();
            if state.is_section_visible(next) {
                state.active_section = next;
            }
            None
        }
        // Shift+Tab: go to previous section
        KeyCode::BackTab => {
            let prev = state.active_section.prev();
            // Only go back if the prev section is visible (always true since
            // we can only go to earlier sections which are always visible)
            if state.is_section_visible(prev) && prev < state.active_section {
                state.active_section = prev;
            }
            None
        }
        // q: quit
        KeyCode::Char('q') => Some(UserCommand::Quit),
        _ => None,
    }
}

/// Handle keyboard input on the settings screen.
///
/// Dispatches to the appropriate handler depending on the active settings tab.
/// The LLM tab uses a dedicated settings-mode handler with field-level
/// navigation (Up/Down between fields, Enter to open, Esc to cancel, 's' to
/// save). The Strategy tab reuses the onboarding handler.
fn handle_settings_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    use crate::protocol::SettingsSection;

    // --- Unsaved changes confirmation modal: intercept all input ---
    if view_state.confirm_exit_settings {
        return handle_confirm_exit_settings(key_event, view_state);
    }

    let active_tab = view_state.settings_tab;

    // --- LLM tab: dedicated settings-mode handler ---
    if active_tab == SettingsSection::LlmConfig {
        return handle_llm_settings_key(key_event, view_state);
    }

    // --- Strategy tab: delegate to onboarding handler ---
    // Check if we're in an editing sub-mode (text input, overview editing,
    // generating). If so, delegate fully to the strategy handler.
    if view_state.settings_is_editing()
        || view_state.strategy_setup.generating
    {
        let cmd = handle_strategy_setup_key(key_event, view_state);
        return filter_onboarding_commands(cmd);
    }

    // Not editing: handle settings-level keys first, then delegate
    match key_event.code {
        // Tab switching between LLM and Strategy tabs
        KeyCode::Char('1') => {
            Some(UserCommand::SwitchSettingsTab(SettingsSection::LlmConfig))
        }
        KeyCode::Char('2') => {
            Some(UserCommand::SwitchSettingsTab(SettingsSection::StrategyConfig))
        }

        // s: save strategy settings
        KeyCode::Char('s') => {
            let state = &mut view_state.strategy_setup;
            let weights = state.category_weights.clone();
            let pct = state.hitting_budget_pct;
            let overview = if state.strategy_overview.is_empty() {
                None
            } else {
                Some(state.strategy_overview.clone())
            };
            state.settings_dirty = false;
            // Update snapshot to reflect saved state
            state.snapshot_settings();
            Some(UserCommand::OnboardingAction(
                OnboardingAction::SaveStrategyConfig {
                    hitting_budget_pct: pct,
                    category_weights: weights,
                    strategy_overview: overview,
                },
            ))
        }

        // Esc: if unsaved changes exist, show confirmation modal; otherwise exit
        KeyCode::Esc => {
            if view_state.strategy_setup.settings_dirty
                || view_state.llm_setup.settings_dirty
                || view_state.llm_setup.settings_needs_connection_test
            {
                view_state.confirm_exit_settings = true;
                None
            } else {
                Some(UserCommand::ExitSettings)
            }
        }

        // q: quit the application
        KeyCode::Char('q') => Some(UserCommand::Quit),

        // For all other keys, delegate to the strategy tab's handler
        _ => {
            let cmd = handle_strategy_setup_key(key_event, view_state);
            filter_onboarding_commands(cmd)
        }
    }
}

/// Handle keyboard input while the unsaved-changes confirmation modal is showing.
///
/// - `y`/`Y`: Save all dirty settings and exit to draft mode.
/// - `n`/`N`: Discard unsaved changes (restore snapshots) and exit.
/// - `Esc`:   Dismiss the modal and return to settings.
/// - All other keys: ignored.
fn handle_confirm_exit_settings(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    match key_event.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            view_state.confirm_exit_settings = false;

            // Gather save payloads for each dirty tab.
            // Only save LLM config when dirty AND not blocked by a pending/failed
            // connection test. If save is blocked, discard LLM changes instead.
            let llm_save = if view_state.llm_setup.settings_dirty
                && !view_state.llm_setup.is_save_blocked()
            {
                let provider = view_state.llm_setup.selected_provider().clone();
                let model_id = view_state
                    .llm_setup
                    .selected_model()
                    .map(|m| m.model_id.to_string())
                    .unwrap_or_default();
                let api_key_val = view_state.llm_setup.api_key_input.value().to_string();
                let api_key = if api_key_val.is_empty() {
                    None
                } else {
                    Some(api_key_val)
                };
                view_state.llm_setup.settings_dirty = false;
                view_state.llm_setup.settings_needs_connection_test = false;
                view_state.llm_setup.snapshot_settings();
                Some((provider, model_id, api_key))
            } else {
                if view_state.llm_setup.is_save_blocked() {
                    view_state.llm_setup.restore_settings_snapshot();
                }
                None
            };

            let strategy_save = if view_state.strategy_setup.settings_dirty {
                let pct = view_state.strategy_setup.hitting_budget_pct;
                let weights = view_state.strategy_setup.category_weights.clone();
                let overview = if view_state.strategy_setup.strategy_overview.is_empty() {
                    None
                } else {
                    Some(view_state.strategy_setup.strategy_overview.clone())
                };
                view_state.strategy_setup.settings_dirty = false;
                view_state.strategy_setup.snapshot_settings();
                Some((pct, weights, overview))
            } else {
                None
            };

            Some(UserCommand::SaveAndExitSettings {
                llm: llm_save,
                strategy: strategy_save,
            })
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            view_state.confirm_exit_settings = false;
            // Restore snapshots to discard unsaved changes
            if view_state.llm_setup.settings_dirty
                || view_state.llm_setup.settings_needs_connection_test
            {
                view_state.llm_setup.restore_settings_snapshot();
            }
            if view_state.strategy_setup.settings_dirty {
                view_state.strategy_setup.restore_settings_snapshot();
            }
            Some(UserCommand::ExitSettings)
        }
        KeyCode::Esc => {
            // Cancel: go back to settings
            view_state.confirm_exit_settings = false;
            None
        }
        _ => None,
    }
}

/// Handle keyboard input on the LLM settings tab.
///
/// Implements a field-based navigation model:
/// - **Overview mode** (`settings_editing_field == None`): Up/Down navigate
///   between the three fields (Provider, Model, API Key). Enter opens the
///   focused field's dropdown/editor. 's' saves all settings. Esc exits
///   settings and returns to draft.
/// - **Field editing mode** (`settings_editing_field == Some(section)`):
///   Only the active field's dropdown/editor is shown. Up/Down select within
///   the list. Enter confirms the field and advances to the next field in
///   sequence. Esc resets to the last saved values and returns to overview.
fn handle_llm_settings_key(
    key_event: KeyEvent,
    view_state: &mut ViewState,
) -> Option<UserCommand> {
    use crate::protocol::SettingsSection;
    use super::onboarding::llm_setup::LlmSetupSection;

    let state = &mut view_state.llm_setup;

    // --- API key text editing mode (typing characters) ---
    if state.api_key_editing {
        return match key_event.code {
            KeyCode::Enter => {
                state.api_key_editing = false;
                state.settings_dirty = true;
                // ApiKey is the last field — return to overview
                state.settings_editing_field = None;
                state.active_section = LlmSetupSection::ApiKey;
                // If any config field (provider, model, or API key) changed
                // from the saved snapshot, require a connection test before
                // allowing save.
                let new_key = state.api_key_input.value().to_string();
                let has_key = !new_key.is_empty() || state.has_saved_api_key;
                if state.has_config_changed_from_snapshot() && has_key {
                    state.settings_needs_connection_test = true;
                    state.connection_status =
                        super::onboarding::llm_setup::LlmConnectionStatus::Testing;
                    if !new_key.is_empty() {
                        // Key text is present — send SetApiKey which
                        // auto-triggers a test on the backend.
                        Some(UserCommand::OnboardingAction(
                            OnboardingAction::SetApiKey(new_key),
                        ))
                    } else {
                        // Key input is empty but a saved key exists on disk.
                        // Trigger a connection test directly using the
                        // persisted key.
                        Some(UserCommand::OnboardingAction(
                            OnboardingAction::TestConnection,
                        ))
                    }
                } else {
                    state.settings_needs_connection_test = false;
                    state.connection_status = super::onboarding::llm_setup::LlmConnectionStatus::Untested;
                    None
                }
            }
            KeyCode::Esc => {
                // Reset to saved snapshot and return to overview
                state.restore_settings_snapshot();
                None
            }
            _ => {
                if let Some(msg) = super::TextInput::key_to_message(&key_event) {
                    state.api_key_input.update(msg);
                    state.settings_dirty = true;
                }
                None
            }
        };
    }

    // --- Field editing mode (dropdown open for Provider or Model) ---
    if let Some(editing) = state.settings_editing_field {
        return match key_event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                match editing {
                    LlmSetupSection::Provider => {
                        let before = state.selected_provider_idx;
                        state.provider_up();
                        if state.selected_provider_idx != before {
                            state.settings_dirty = true;
                        }
                    }
                    LlmSetupSection::Model => {
                        let before = state.selected_model_idx;
                        state.model_up();
                        if state.selected_model_idx != before {
                            state.settings_dirty = true;
                        }
                    }
                    LlmSetupSection::ApiKey => {
                        // Should not reach here; API key editing is handled above
                    }
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match editing {
                    LlmSetupSection::Provider => {
                        let before = state.selected_provider_idx;
                        state.provider_down();
                        if state.selected_provider_idx != before {
                            state.settings_dirty = true;
                        }
                    }
                    LlmSetupSection::Model => {
                        let before = state.selected_model_idx;
                        state.model_down();
                        if state.selected_model_idx != before {
                            state.settings_dirty = true;
                        }
                    }
                    LlmSetupSection::ApiKey => {}
                }
                None
            }
            KeyCode::Enter => {
                // Confirm the current field.
                // In settings mode (not onboarding), return to overview and
                // trigger a connection test if the config has changed and a
                // key is available. In onboarding, advance to the next field.
                match editing {
                    LlmSetupSection::Provider | LlmSetupSection::Model => {
                        if state.in_settings_mode {
                            // Return to overview mode
                            state.settings_editing_field = None;
                            // Check whether config changed and an API key exists
                            let has_key = !state.api_key_input.value().is_empty()
                                || state.has_saved_api_key;
                            if state.has_config_changed_from_snapshot() && has_key {
                                state.settings_needs_connection_test = true;
                                state.connection_status =
                                    super::onboarding::llm_setup::LlmConnectionStatus::Testing;
                                let new_key = state.api_key_input.value().to_string();
                                if !new_key.is_empty() {
                                    return Some(UserCommand::OnboardingAction(
                                        OnboardingAction::SetApiKey(new_key),
                                    ));
                                } else {
                                    return Some(UserCommand::OnboardingAction(
                                        OnboardingAction::TestConnection,
                                    ));
                                }
                            }
                            return None;
                        }
                        // Onboarding: advance to next field in sequence
                        if editing == LlmSetupSection::Provider {
                            state.active_section = LlmSetupSection::Model;
                            state.settings_editing_field = Some(LlmSetupSection::Model);
                        } else {
                            // Model -> ApiKey
                            state.active_section = LlmSetupSection::ApiKey;
                            state.settings_editing_field = Some(LlmSetupSection::ApiKey);
                            state.api_key_backup = state.api_key_input.value().to_string();
                            state.api_key_editing = true;
                        }
                        None
                    }
                    LlmSetupSection::ApiKey => {
                        // Last field — return to overview (should not reach here,
                        // handled in api_key_editing block above)
                        state.settings_editing_field = None;
                        None
                    }
                }
            }
            KeyCode::Esc => {
                // Reset to saved snapshot and return to overview
                state.restore_settings_snapshot();
                None
            }
            KeyCode::Char('q') => Some(UserCommand::Quit),
            _ => None,
        };
    }

    // --- Overview mode (no field editing, all fields shown as summaries) ---

    // Handle Esc before the main match so we can access `view_state` fields
    // outside the `llm_setup` borrow (strategy_setup, confirm_exit_settings).
    if key_event.code == KeyCode::Esc {
        let llm_dirty = view_state.llm_setup.settings_dirty
            || view_state.llm_setup.settings_needs_connection_test;
        let strategy_dirty = view_state.strategy_setup.settings_dirty;
        if llm_dirty || strategy_dirty {
            view_state.confirm_exit_settings = true;
            return None;
        } else {
            return Some(UserCommand::ExitSettings);
        }
    }

    match key_event.code {
        // Up/Down: navigate between the three fields (clamped, no wrapping)
        KeyCode::Up | KeyCode::Char('k') => {
            let idx = state.active_section.step_index();
            if idx > 0 {
                state.active_section = LlmSetupSection::CYCLE[idx - 1];
            }
            None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let idx = state.active_section.step_index();
            if idx + 1 < LlmSetupSection::CYCLE.len() {
                state.active_section = LlmSetupSection::CYCLE[idx + 1];
            }
            None
        }

        // Enter: open the focused field's dropdown/editor
        KeyCode::Enter => {
            // Snapshot current state for Escape restoration
            state.snapshot_settings();
            state.settings_editing_field = Some(state.active_section);
            // If opening ApiKey, enter text editing mode
            if state.active_section == LlmSetupSection::ApiKey {
                state.api_key_backup = state.api_key_input.value().to_string();
                state.api_key_editing = true;
            }
            None
        }

        // s: save all settings (blocked while connection test is pending/failed)
        KeyCode::Char('s') => {
            if state.is_save_blocked() {
                // Save is gated on a successful connection test — ignore
                None
            } else {
                let provider = state.selected_provider().clone();
                let model_id = state
                    .selected_model()
                    .map(|m| m.model_id.to_string())
                    .unwrap_or_default();
                let api_key_val = state.api_key_input.value().to_string();
                let api_key = if api_key_val.is_empty() {
                    None
                } else {
                    Some(api_key_val)
                };

                state.settings_dirty = false;
                state.settings_needs_connection_test = false;
                // Update the saved snapshot to reflect the saved state
                state.snapshot_settings();

                Some(UserCommand::OnboardingAction(
                    OnboardingAction::SaveLlmConfig {
                        provider,
                        model_id,
                        api_key,
                    },
                ))
            }
        }

        // Tab switching between LLM and Strategy tabs
        KeyCode::Char('1') => {
            Some(UserCommand::SwitchSettingsTab(SettingsSection::LlmConfig))
        }
        KeyCode::Char('2') => {
            Some(UserCommand::SwitchSettingsTab(SettingsSection::StrategyConfig))
        }

        // Esc is handled above, before the match (to avoid borrow conflicts)
        KeyCode::Esc => unreachable!(),

        // q: quit
        KeyCode::Char('q') => Some(UserCommand::Quit),

        _ => None,
    }
}

/// Filter out onboarding-specific commands that don't apply in settings mode.
///
/// In settings mode, GoBack/GoNext/Skip make no sense. `n` in LLM setup
/// normally maps to GoNext, but in settings we suppress it. Esc in the
/// onboarding handlers normally maps to GoBack, but we handle Esc at the
/// settings level (exit settings). This function strips those commands.
fn filter_onboarding_commands(cmd: Option<UserCommand>) -> Option<UserCommand> {
    use crate::protocol::OnboardingAction;

    match &cmd {
        Some(UserCommand::OnboardingAction(action)) => match action {
            OnboardingAction::GoBack | OnboardingAction::GoNext | OnboardingAction::Skip => None,
            _ => cmd,
        },
        _ => cmd,
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

    // Filter mode: route keys through the available panel component
    if view_state.available_panel.filter_mode() {
        if let Some(msg) = view_state.available_panel.key_to_message(key_event) {
            view_state.available_panel.update(msg);
        }
        return None;
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
                view_state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
            }
            None
        }

        // Escape: clear focus, filter text, and position filter
        KeyCode::Esc => {
            view_state.focused_panel = None;
            view_state.available_panel.update(AvailablePanelMessage::ClearFilters);
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

        // Open settings screen
        KeyCode::Char(',') => Some(UserCommand::OpenSettings),

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

/// Open the position filter modal, pre-selecting the row that matches the
/// current active position filter so the user's context is preserved.
fn open_position_filter_modal(view_state: &mut ViewState) {
    let modal = &mut view_state.position_filter_modal;
    modal.open = true;
    modal.search_text.clear();

    // Pre-select the option that matches the current position_filter
    let current = view_state.available_panel.position_filter();
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
/// - Backspace: delete the character before the cursor in search text
/// - Delete: delete the character at the cursor in search text
/// - Left/Right: move cursor within search text
/// - Home/End: jump to start/end of search text
/// - Printable char: insert at cursor and reset selection to 0
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
                view_state.available_panel.update(AvailablePanelMessage::SetPositionFilter(options[idx]));
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
        _ => {
            let modifies_text = matches!(
                key_event.code,
                KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_)
            );
            if let Some(msg) = super::TextInput::key_to_message(&key_event) {
                view_state.position_filter_modal.search_text.update(msg);
            }
            if modifies_text {
                // Reset selection so the user starts at the top of the
                // (potentially changed) filtered list.
                view_state.position_filter_modal.selected_index = 0;
            }
            None
        }
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
    if key == "analysis" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageUp
        } else {
            ScrollDirection::Up
        };
        view_state
            .analysis_panel
            .update(AnalysisPanelMessage::Scroll(dir));
        return;
    }
    if key == "draft_log" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageUp
        } else {
            ScrollDirection::Up
        };
        view_state
            .draft_log_panel
            .update(DraftLogMessage::Scroll(dir));
        return;
    }
    if key == "teams" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageUp
        } else {
            ScrollDirection::Up
        };
        view_state
            .teams_panel
            .update(TeamsMessage::Scroll(dir));
        return;
    }
    if key == "roster" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageUp
        } else {
            ScrollDirection::Up
        };
        view_state
            .roster_panel
            .update(RosterMessage::Scroll(dir));
        return;
    }
    if key == "scarcity" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageUp
        } else {
            ScrollDirection::Up
        };
        view_state
            .scarcity_panel
            .update(ScarcityPanelMessage::Scroll(dir));
        return;
    }
    if key == "nom_plan" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageUp
        } else {
            ScrollDirection::Up
        };
        view_state
            .plan_panel
            .update(PlanPanelMessage::Scroll(dir));
        return;
    }
    if key == "available" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageUp
        } else {
            ScrollDirection::Up
        };
        view_state
            .available_panel
            .update(AvailablePanelMessage::Scroll(dir));
        return;
    }
    let offset = view_state.scroll_offset.entry(key.to_string()).or_insert(0);
    *offset = offset.saturating_sub(lines);
}

/// Dispatch a scroll-down event to the appropriate panel based on focus state.
fn dispatch_scroll_down(view_state: &mut ViewState, lines: usize) {
    let key = focused_scroll_key(view_state);
    if key == "analysis" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageDown
        } else {
            ScrollDirection::Down
        };
        view_state
            .analysis_panel
            .update(AnalysisPanelMessage::Scroll(dir));
        return;
    }
    if key == "draft_log" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageDown
        } else {
            ScrollDirection::Down
        };
        view_state
            .draft_log_panel
            .update(DraftLogMessage::Scroll(dir));
        return;
    }
    if key == "teams" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageDown
        } else {
            ScrollDirection::Down
        };
        view_state
            .teams_panel
            .update(TeamsMessage::Scroll(dir));
        return;
    }
    if key == "roster" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageDown
        } else {
            ScrollDirection::Down
        };
        view_state
            .roster_panel
            .update(RosterMessage::Scroll(dir));
        return;
    }
    if key == "scarcity" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageDown
        } else {
            ScrollDirection::Down
        };
        view_state
            .scarcity_panel
            .update(ScarcityPanelMessage::Scroll(dir));
        return;
    }
    if key == "nom_plan" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageDown
        } else {
            ScrollDirection::Down
        };
        view_state
            .plan_panel
            .update(PlanPanelMessage::Scroll(dir));
        return;
    }
    if key == "available" {
        let dir = if lines >= page_size() {
            ScrollDirection::PageDown
        } else {
            ScrollDirection::Down
        };
        view_state
            .available_panel
            .update(AvailablePanelMessage::Scroll(dir));
        return;
    }
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
        // Pre-scroll the analysis panel down 5 positions
        for _ in 0..5 {
            state.analysis_panel.update(AnalysisPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = handle_key(key(KeyCode::Up), &mut state);
        assert!(result.is_none());
        assert_eq!(state.analysis_panel.scroll_offset(), 4);
    }

    #[test]
    fn arrow_down_increments_scroll() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Down), &mut state);
        assert!(result.is_none());
        assert_eq!(state.analysis_panel.scroll_offset(), 1);
    }

    #[test]
    fn k_scrolls_up() {
        let mut state = ViewState::default();
        for _ in 0..3 {
            state.analysis_panel.update(AnalysisPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = handle_key(key(KeyCode::Char('k')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.analysis_panel.scroll_offset(), 2);
    }

    #[test]
    fn j_scrolls_down() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::Char('j')), &mut state);
        assert!(result.is_none());
        assert_eq!(state.analysis_panel.scroll_offset(), 1);
    }

    #[test]
    fn scroll_up_does_not_underflow() {
        let mut state = ViewState::default();
        // Default is 0, scrolling up should stay at 0
        let result = handle_key(key(KeyCode::Up), &mut state);
        assert!(result.is_none());
        assert_eq!(state.analysis_panel.scroll_offset(), 0);
    }

    #[test]
    fn page_down_scrolls_by_page_size() {
        let mut state = ViewState::default();
        let result = handle_key(key(KeyCode::PageDown), &mut state);
        assert!(result.is_none());
        assert_eq!(state.analysis_panel.scroll_offset(), 20);
    }

    #[test]
    fn page_up_scrolls_by_page_size() {
        let mut state = ViewState::default();
        for _ in 0..25 {
            state.analysis_panel.update(AnalysisPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = handle_key(key(KeyCode::PageUp), &mut state);
        assert!(result.is_none());
        assert_eq!(state.analysis_panel.scroll_offset(), 5);
    }

    #[test]
    fn scroll_applies_to_active_tab_widget() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        handle_key(key(KeyCode::Down), &mut state);
        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.available_panel.scroll_offset(), 2);
        // Analysis panel should not have been scrolled
        assert_eq!(state.analysis_panel.scroll_offset(), 0);
        // Nomination plan should not have been scrolled
        assert_eq!(state.plan_panel.scroll_offset(), 0);
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

        assert_eq!(state.roster_panel.scroll_offset(), 2);
        // Analysis panel scroll should not be affected
        assert_eq!(state.analysis_panel.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_scarcity_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::Scarcity);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.scarcity_panel.scroll_offset(), 1);
        assert_eq!(state.analysis_panel.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_budget_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::Budget);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.scroll_offset.get("budget"), Some(&1));
        assert_eq!(state.analysis_panel.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_nom_plan_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::NominationPlan);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.plan_panel.scroll_offset(), 1);
        assert_eq!(state.analysis_panel.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_main_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::MainPanel);

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.analysis_panel.scroll_offset(), 1);
        assert!(state.scroll_offset.get("sidebar").is_none());
    }

    #[test]
    fn scroll_routes_to_main_when_no_focus() {
        let mut state = ViewState::default();
        assert!(state.focused_panel.is_none());

        handle_key(key(KeyCode::Down), &mut state);

        assert_eq!(state.analysis_panel.scroll_offset(), 1);
        assert!(state.scroll_offset.get("sidebar").is_none());
    }

    #[test]
    fn page_scroll_routes_to_roster_when_focused() {
        let mut state = ViewState::default();
        state.focused_panel = Some(FocusPanel::Roster);

        handle_key(key(KeyCode::PageDown), &mut state);

        assert_eq!(state.roster_panel.scroll_offset(), 20);
        assert_eq!(state.analysis_panel.scroll_offset(), 0);
    }

    #[test]
    fn tab_does_not_affect_other_state() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;

        handle_key(key(KeyCode::Tab), &mut state);

        assert_eq!(state.active_tab, TabId::Available, "Tab should not switch tabs");
        assert!(!state.available_panel.filter_mode(), "Tab should not enter filter mode");
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
        assert!(state.available_panel.filter_mode());
    }

    #[test]
    fn slash_does_not_enter_filter_mode_on_other_tabs() {
        for tab in [TabId::Analysis, TabId::DraftLog, TabId::Teams] {
            let mut state = ViewState::default();
            state.active_tab = tab;
            let result = handle_key(key(KeyCode::Char('/')), &mut state);
            assert!(result.is_none(), "/ on {:?} should return None", tab);
            assert!(
                !state.available_panel.filter_mode(),
                "/ on {:?} should not activate filter_mode",
                tab
            );
        }
    }

    #[test]
    fn filter_mode_appends_chars() {
        let mut state = ViewState::default();
        state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
        handle_key(key(KeyCode::Char('t')), &mut state);
        handle_key(key(KeyCode::Char('r')), &mut state);
        handle_key(key(KeyCode::Char('o')), &mut state);
        handle_key(key(KeyCode::Char('u')), &mut state);
        handle_key(key(KeyCode::Char('t')), &mut state);
        assert_eq!(state.available_panel.filter_text().value(), "trout");
        assert!(state.available_panel.filter_mode());
    }

    #[test]
    fn filter_mode_backspace_removes_char() {
        let mut state = ViewState::default();
        state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
        // Type "test"
        for ch in "test".chars() {
            state.available_panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        handle_key(key(KeyCode::Backspace), &mut state);
        assert_eq!(state.available_panel.filter_text().value(), "tes");
    }

    #[test]
    fn filter_mode_backspace_on_empty_is_noop() {
        let mut state = ViewState::default();
        state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
        handle_key(key(KeyCode::Backspace), &mut state);
        assert!(state.available_panel.filter_text().is_empty());
    }

    #[test]
    fn filter_mode_enter_exits_keeps_text() {
        let mut state = ViewState::default();
        state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
        for ch in "trout".chars() {
            state.available_panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert!(result.is_none());
        assert!(!state.available_panel.filter_mode());
        assert_eq!(state.available_panel.filter_text().value(), "trout");
    }

    #[test]
    fn filter_mode_esc_exits_clears_text() {
        let mut state = ViewState::default();
        state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
        for ch in "trout".chars() {
            state.available_panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(!state.available_panel.filter_mode());
        assert!(state.available_panel.filter_text().is_empty());
    }

    #[test]
    fn filter_mode_does_not_switch_tabs() {
        let mut state = ViewState::default();
        state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
        state.active_tab = TabId::Analysis;
        handle_key(key(KeyCode::Char('3')), &mut state);
        // Should add '3' to filter text, not switch tabs
        assert_eq!(state.available_panel.filter_text().value(), "3");
        assert_eq!(state.active_tab, TabId::Analysis);
    }

    #[test]
    fn filter_mode_ctrl_c_still_quits() {
        let mut state = ViewState::default();
        state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
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
        state.available_panel.update(AvailablePanelMessage::SetPositionFilter(Some(Position::Catcher)));
        state.position_filter_modal.open = true;
        state.position_filter_modal.selected_index = 2; // e.g. "1B"
        state.position_filter_modal.search_text.set_value("1");

        handle_key(key(KeyCode::Esc), &mut state);

        assert!(!state.position_filter_modal.open, "Esc should close modal");
        assert!(
            state.position_filter_modal.search_text.is_empty(),
            "Esc should clear search text"
        );
        // Position filter must NOT have changed
        assert_eq!(
            state.available_panel.position_filter(),
            Some(Position::Catcher),
            "Esc should not change the position filter"
        );
    }

    #[test]
    fn modal_enter_applies_selected_option() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        state.position_filter_modal.open = true;
        // Options (unfiltered): ALL(0), C(1), 1B(2), ...
        state.position_filter_modal.selected_index = 1; // "C"

        handle_key(key(KeyCode::Enter), &mut state);

        assert!(!state.position_filter_modal.open, "Enter should close modal");
        assert_eq!(
            state.available_panel.position_filter(),
            Some(Position::Catcher),
            "Enter should apply selected option"
        );
    }

    #[test]
    fn modal_enter_applies_all_option() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        state.available_panel.update(AvailablePanelMessage::SetPositionFilter(Some(Position::Catcher)));
        state.position_filter_modal.open = true;
        state.position_filter_modal.selected_index = 0; // "ALL"

        handle_key(key(KeyCode::Enter), &mut state);

        assert!(!state.position_filter_modal.open);
        assert!(
            state.available_panel.position_filter().is_none(),
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
        assert_eq!(state.position_filter_modal.search_text.value(), "s");
        assert_eq!(state.position_filter_modal.selected_index, 0, "Typing resets selection");
    }

    #[test]
    fn modal_backspace_removes_char_and_resets_selection() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        state.position_filter_modal.search_text.set_value("SP");
        state.position_filter_modal.selected_index = 2;

        handle_key(key(KeyCode::Backspace), &mut state);
        assert_eq!(state.position_filter_modal.search_text.value(), "S");
        assert_eq!(state.position_filter_modal.selected_index, 0);
    }

    #[test]
    fn modal_enter_with_filtered_list_applies_correct_option() {
        let mut state = ViewState::default();
        state.position_filter_modal.open = true;
        // Type "S" to filter: options with "S" -> SS, SP (and "ALL"? no, ALL doesn't contain S)
        // Actually: SS contains S, SP contains S
        state.position_filter_modal.search_text.set_value("SP");
        state.position_filter_modal.selected_index = 0; // first match

        handle_key(key(KeyCode::Enter), &mut state);

        assert!(!state.position_filter_modal.open);
        assert_eq!(state.available_panel.position_filter(), Some(Position::StartingPitcher));
    }

    #[test]
    fn modal_pre_selects_current_position_filter() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        state.available_panel.update(AvailablePanelMessage::SetPositionFilter(Some(Position::StartingPitcher)));

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
        assert_eq!(state.position_filter_modal.search_text.value(), "2");
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
        assert_eq!(state.analysis_panel.scroll_offset(), 0, "Scroll should be blocked");

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
        state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
        for ch in "test".chars() {
            state.available_panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        let result = handle_key(key(KeyCode::Char('q')), &mut state);
        assert!(result.is_none(), "q in filter mode should not produce a command");
        assert_eq!(state.available_panel.filter_text().value(), "testq", "q should be appended to filter text");
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
        for ch in "test".chars() {
            state.available_panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(ch))));
        }
        state.available_panel.update(AvailablePanelMessage::SetPositionFilter(Some(Position::Catcher)));
        state.focused_panel = Some(FocusPanel::Roster);
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert!(result.is_none());
        assert!(state.available_panel.filter_text().is_empty());
        assert!(state.available_panel.position_filter().is_none());
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
        assert_eq!(
            state.analysis_panel.scroll_offset(), 0,
            "Repeat event should not modify scroll state"
        );
    }

    // -- Bracket suppression mechanism --

    #[test]
    fn entering_filter_mode_sets_suppress_next_bracket() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;
        assert!(!state.suppress_next_bracket);

        // '/' enters filter mode, which transitions into text editing,
        // so the post-handler check should set suppress_next_bracket.
        handle_key(key(KeyCode::Char('/')), &mut state);
        assert!(state.available_panel.filter_mode());
        assert!(
            state.suppress_next_bracket,
            "Entering filter mode should set suppress_next_bracket"
        );
    }

    #[test]
    fn bracket_immediately_after_entering_text_mode_is_suppressed() {
        let mut state = ViewState::default();
        state.active_tab = TabId::Available;

        // Enter filter mode (sets the suppression flag)
        handle_key(key(KeyCode::Char('/')), &mut state);
        assert!(state.suppress_next_bracket);

        // The very next key is '[' — it should be silently suppressed
        let result = handle_key(key(KeyCode::Char('[')), &mut state);
        assert!(result.is_none(), "Stray '[' should be suppressed");
        assert!(
            state.available_panel.filter_text().is_empty(),
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
        state.active_tab = TabId::Available;

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
            state.available_panel.filter_text().value(),
            "a",
            "'a' should be inserted normally"
        );
    }

    #[test]
    fn bracket_not_suppressed_when_flag_is_not_set() {
        let mut state = ViewState::default();
        state.available_panel.update(AvailablePanelMessage::ToggleFilterMode);
        assert!(!state.suppress_next_bracket);

        // '[' without the suppression flag should be inserted normally
        handle_key(key(KeyCode::Char('[')), &mut state);
        assert_eq!(
            state.available_panel.filter_text().value(),
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
        assert_eq!(state.analysis_panel.scroll_offset(), 1);

        // Switch focus to roster and scroll
        state.focused_panel = Some(FocusPanel::Roster);
        handle_key(key(KeyCode::Down), &mut state);
        assert_eq!(state.roster_panel.scroll_offset(), 1);

        // Main panel scroll should be untouched
        assert_eq!(state.analysis_panel.scroll_offset(), 1);
        // Other panels should be untouched
        assert!(state.scroll_offset.get("scarcity").is_none());
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
        assert!(state.confirm_exit_settings);

        // 'n' should discard, revert, and exit
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert_eq!(result, Some(UserCommand::ExitSettings));
        assert!(!state.confirm_exit_settings);
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
        assert!(state.confirm_exit_settings);

        // 'n' should discard, restore, and exit
        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert_eq!(result, Some(UserCommand::ExitSettings));
        assert!(!state.confirm_exit_settings);
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
        state.confirm_exit_settings = true;

        // Esc should dismiss the modal and return to settings
        let result = handle_key(key(KeyCode::Esc), &mut state);
        assert_eq!(result, None);
        assert!(!state.confirm_exit_settings);
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
        state.confirm_exit_settings = true;

        let result = handle_key(key(KeyCode::Char('y')), &mut state);
        assert!(
            matches!(result, Some(UserCommand::SaveAndExitSettings { llm: Some(_), strategy: None })),
            "expected SaveAndExitSettings with LLM save, got {:?}",
            result,
        );
        assert!(!state.confirm_exit_settings);
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
        state.confirm_exit_settings = true;

        let result = handle_key(key(KeyCode::Char('y')), &mut state);
        assert!(
            matches!(result, Some(UserCommand::SaveAndExitSettings { llm: Some(_), strategy: Some(_) })),
            "expected SaveAndExitSettings with both saves, got {:?}",
            result,
        );
        assert!(!state.confirm_exit_settings);
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

        state.confirm_exit_settings = true;

        let result = handle_key(key(KeyCode::Char('n')), &mut state);
        assert_eq!(result, Some(UserCommand::ExitSettings));
        assert!(!state.confirm_exit_settings);
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
        state.confirm_exit_settings = true;

        // Random keys should be ignored
        let result = handle_key(key(KeyCode::Char('a')), &mut state);
        assert_eq!(result, None);
        assert!(state.confirm_exit_settings);

        let result = handle_key(key(KeyCode::Enter), &mut state);
        assert_eq!(result, None);
        assert!(state.confirm_exit_settings);
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
        assert!(!state.confirm_exit_settings);
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
        assert!(!state.confirm_exit_settings);
    }

    #[test]
    fn confirm_exit_settings_uppercase_y_works() {
        use crate::protocol::SettingsSection;

        let mut state = ViewState::default();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);
        state.settings_tab = SettingsSection::StrategyConfig;
        state.strategy_setup.settings_dirty = true;
        state.strategy_setup.strategy_overview = "Test".to_string();
        state.confirm_exit_settings = true;

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
        state.confirm_exit_settings = true;

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
        state.confirm_exit_settings = true;

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
        assert!(!state.confirm_exit_settings);
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
        state.confirm_exit_settings = true;

        let result = handle_key(key(KeyCode::Char('y')), &mut state);
        // LLM save should proceed because connection test passed
        assert!(
            matches!(result, Some(UserCommand::SaveAndExitSettings { llm: Some(_), strategy: None })),
            "expected SaveAndExitSettings with LLM save when not blocked, got {:?}",
            result,
        );
        assert!(!state.confirm_exit_settings);
        assert!(!state.llm_setup.settings_dirty);
    }
}
