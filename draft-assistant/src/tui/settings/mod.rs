// Settings screen: tabbed pane reusing onboarding widgets for LLM and strategy config.
//
// Accessible from draft mode via the `,` keybind. Renders a tabbed layout that
// delegates to the same render functions used by the onboarding wizard, avoiding
// code duplication.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::protocol::{OnboardingAction, SettingsSection, UserCommand};
use crate::tui::app::App;
use crate::tui::confirm_dialog::{ConfirmDialog, ConfirmMessage, ConfirmResult};
use crate::tui::onboarding::llm_setup::{LlmSetupMessage, LlmSetupState};
use crate::tui::onboarding::strategy_setup::{StrategySetupMessage, StrategySetupState};

// ---------------------------------------------------------------------------
// SettingsMessage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsMessage {
    SwitchTab(SettingsSection),
    LlmConfig(LlmSetupMessage),
    Strategy(StrategySetupMessage),
    SaveStrategy,
    ExitSettings,
    ConfirmExit(ConfirmMessage),
    Quit,
}

// ---------------------------------------------------------------------------
// key_to_message
// ---------------------------------------------------------------------------

pub fn key_to_message(
    settings_tab: SettingsSection,
    llm_setup: &LlmSetupState,
    strategy_setup: &StrategySetupState,
    confirm_exit_settings: &ConfirmDialog,
    key: KeyEvent,
) -> Option<SettingsMessage> {
    // Confirm dialog intercepts all input when open
    if confirm_exit_settings.open {
        let msg = match key.code {
            KeyCode::Esc => Some(ConfirmMessage::Cancel),
            KeyCode::Char(ch) => Some(ConfirmMessage::Confirm(ch.to_ascii_lowercase())),
            _ => None,
        };
        return msg.map(SettingsMessage::ConfirmExit);
    }

    match settings_tab {
        SettingsSection::LlmConfig => {
            // LlmSetupState key routing is handled by the subscription system
            // via LlmSetupState::subscription(). This function is retained for
            // non-subscription callers but LlmConfig routing is a no-op here.
            let _ = (llm_setup, key);
            None
        }
        SettingsSection::StrategyConfig => {
            let is_editing = strategy_setup.is_editing();
            let is_generating = strategy_setup.generating;

            if is_editing || is_generating {
                return strategy_setup
                    .key_to_message(key)
                    .map(SettingsMessage::Strategy);
            }

            // Not editing: handle settings-level keys first
            match key.code {
                KeyCode::Char('1') => {
                    Some(SettingsMessage::SwitchTab(SettingsSection::LlmConfig))
                }
                KeyCode::Char('2') => {
                    Some(SettingsMessage::SwitchTab(SettingsSection::StrategyConfig))
                }
                KeyCode::Char('s') => Some(SettingsMessage::SaveStrategy),
                KeyCode::Esc => Some(SettingsMessage::ExitSettings),
                KeyCode::Char('q') => Some(SettingsMessage::Quit),
                _ => {
                    strategy_setup
                        .key_to_message(key)
                        .map(SettingsMessage::Strategy)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

pub fn update(
    _settings_tab: SettingsSection,
    llm_setup: &mut LlmSetupState,
    strategy_setup: &mut StrategySetupState,
    confirm_exit_settings: &mut ConfirmDialog,
    msg: SettingsMessage,
) -> Option<UserCommand> {
    match msg {
        SettingsMessage::ConfirmExit(confirm_msg) => {
            if let Some(result) = confirm_exit_settings.update(confirm_msg) {
                match result {
                    ConfirmResult::Confirmed(ch) => {
                        handle_confirm_exit_choice(ch, llm_setup, strategy_setup)
                    }
                    ConfirmResult::Cancelled => None,
                }
            } else {
                None
            }
        }

        SettingsMessage::LlmConfig(lm) => {
            // Intercept SettingsExit to check cross-component dirty state
            if matches!(lm, LlmSetupMessage::SettingsExit) {
                let llm_dirty =
                    llm_setup.settings_dirty || llm_setup.settings_needs_connection_test;
                let strategy_dirty = strategy_setup.settings_dirty;
                if llm_dirty || strategy_dirty {
                    confirm_exit_settings.update(ConfirmMessage::Open);
                    return None;
                }
            }
            llm_setup.update(lm)
        }

        SettingsMessage::Strategy(sm) => {
            let cmd = strategy_setup.update(sm);
            filter_onboarding_commands(cmd)
        }

        SettingsMessage::SwitchTab(tab) => {
            Some(UserCommand::SwitchSettingsTab(tab))
        }

        SettingsMessage::SaveStrategy => {
            let weights = strategy_setup.category_weights.clone();
            let pct = strategy_setup.hitting_budget_pct;
            let overview = if strategy_setup.strategy_overview.is_empty() {
                None
            } else {
                Some(strategy_setup.strategy_overview.clone())
            };
            strategy_setup.settings_dirty = false;
            strategy_setup.snapshot_settings();
            Some(UserCommand::OnboardingAction(
                OnboardingAction::SaveStrategyConfig {
                    hitting_budget_pct: pct,
                    category_weights: weights,
                    strategy_overview: overview,
                },
            ))
        }

        SettingsMessage::ExitSettings => {
            if strategy_setup.settings_dirty
                || llm_setup.settings_dirty
                || llm_setup.settings_needs_connection_test
            {
                confirm_exit_settings.update(ConfirmMessage::Open);
                None
            } else {
                Some(UserCommand::ExitSettings)
            }
        }

        SettingsMessage::Quit => Some(UserCommand::Quit),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn handle_confirm_exit_choice(
    ch: char,
    llm_setup: &mut LlmSetupState,
    strategy_setup: &mut StrategySetupState,
) -> Option<UserCommand> {
    match ch {
        'y' => {
            let llm_save = if llm_setup.settings_dirty && !llm_setup.is_save_blocked() {
                let provider = llm_setup.selected_provider().clone();
                let model_id = llm_setup
                    .selected_model()
                    .map(|m| m.model_id.to_string())
                    .unwrap_or_default();
                let api_key_val = llm_setup.api_key_input.value().to_string();
                let api_key = if api_key_val.is_empty() {
                    None
                } else {
                    Some(api_key_val)
                };
                llm_setup.settings_dirty = false;
                llm_setup.settings_needs_connection_test = false;
                llm_setup.snapshot_settings();
                Some((provider, model_id, api_key))
            } else {
                if llm_setup.is_save_blocked() {
                    llm_setup.restore_settings_snapshot();
                }
                None
            };

            let strategy_save = if strategy_setup.settings_dirty {
                let pct = strategy_setup.hitting_budget_pct;
                let weights = strategy_setup.category_weights.clone();
                let overview = if strategy_setup.strategy_overview.is_empty() {
                    None
                } else {
                    Some(strategy_setup.strategy_overview.clone())
                };
                strategy_setup.settings_dirty = false;
                strategy_setup.snapshot_settings();
                Some((pct, weights, overview))
            } else {
                None
            };

            Some(UserCommand::SaveAndExitSettings {
                llm: llm_save,
                strategy: strategy_save,
            })
        }
        'n' => {
            if llm_setup.settings_dirty || llm_setup.settings_needs_connection_test {
                llm_setup.restore_settings_snapshot();
            }
            if strategy_setup.settings_dirty {
                strategy_setup.restore_settings_snapshot();
            }
            Some(UserCommand::ExitSettings)
        }
        _ => None,
    }
}

fn filter_onboarding_commands(cmd: Option<UserCommand>) -> Option<UserCommand> {
    match &cmd {
        Some(UserCommand::OnboardingAction(action)) => match action {
            OnboardingAction::GoBack | OnboardingAction::GoNext | OnboardingAction::Skip => None,
            _ => cmd,
        },
        _ => cmd,
    }
}

/// Render the settings screen into the given frame.
///
/// Draws a tabbed layout with LLM and Strategy tabs, delegating content
/// rendering to the existing onboarding widgets. The active tab is determined
/// by `view_state.settings_tab`.
pub fn render(frame: &mut Frame, state: &App) {
    let area = frame.area();

    // Split: header (2 lines) | content | help bar (1 line)
    let outer = Layout::vertical([
        Constraint::Length(2), // tab bar
        Constraint::Min(0),   // content area
        Constraint::Length(1), // help bar
    ])
    .split(area);

    // --- Tab bar ---
    render_tab_bar(frame, outer[0], state.settings_tab);

    // --- Content area: delegate to onboarding renderers ---
    match state.settings_tab {
        SettingsSection::LlmConfig => {
            super::onboarding::llm_setup::render(frame, outer[1], &state.llm_setup);
        }
        SettingsSection::StrategyConfig => {
            super::onboarding::strategy_setup::render(frame, outer[1], &state.strategy_setup);
        }
    }

    // --- Help bar: render the pre-synced keybind hints ---
    super::render_help_bar(frame, outer[2], state, &state.active_keybinds);

    // --- Unsaved changes confirmation modal overlay ---
    if state.confirm_exit_settings.open {
        state.confirm_exit_settings.view(frame, area);
    }
}

/// Render the settings tab bar with LLM and Strategy tabs.
fn render_tab_bar(frame: &mut Frame, area: Rect, active_tab: SettingsSection) {
    let title_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let llm_style = if active_tab == SettingsSection::LlmConfig {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let strategy_style = if active_tab == SettingsSection::StrategyConfig {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let tab_line = Line::from(vec![
        Span::styled("  Settings  ", title_style),
        Span::styled("            ", Style::default()),
        Span::styled(" 1:LLM ", llm_style),
        Span::styled(" ", Style::default()),
        Span::styled(" 2:Strategy ", strategy_style),
    ]);

    // Render the tab bar with a bottom border using a second line
    let rows = Layout::vertical([
        Constraint::Length(1), // tab line
        Constraint::Length(1), // separator
    ])
    .split(area);

    frame.render_widget(Paragraph::new(tab_line), rows[0]);

    let separator = Paragraph::new(Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(separator, rows[1]);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SettingsSection;
    use crate::tui::app::App;

    #[test]
    fn render_llm_tab_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = App::default();
        state.settings_tab = SettingsSection::LlmConfig;
        terminal
            .draw(|frame| render(frame, &state))
            .unwrap();
    }

    #[test]
    fn render_strategy_tab_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = App::default();
        state.settings_tab = SettingsSection::StrategyConfig;
        terminal
            .draw(|frame| render(frame, &state))
            .unwrap();
    }

    #[test]
    fn render_small_terminal_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = App::default();
        state.settings_tab = SettingsSection::LlmConfig;
        terminal
            .draw(|frame| render(frame, &state))
            .unwrap();
    }

    #[test]
    fn render_with_editing_state_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = App::default();
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.api_key_editing = true;
        terminal
            .draw(|frame| render(frame, &state))
            .unwrap();
    }
}
