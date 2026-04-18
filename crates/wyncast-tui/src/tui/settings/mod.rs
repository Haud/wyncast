// Settings screen: tabbed pane reusing onboarding widgets for LLM and strategy config.
//
// Accessible from draft mode via the `,` keybind. Renders a tabbed layout that
// delegates to the same render functions used by the onboarding wizard, avoiding
// code duplication.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crossterm::event::KeyCode;
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
use crate::tui::subscription::{Subscription, SubscriptionId};
use crate::tui::subscription::keybinding::{
    exact, KeyBindingRecipe, KeybindHint, KeybindManager, PRIORITY_NORMAL,
};

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
// subscription
// ---------------------------------------------------------------------------

/// Compose keybinding subscriptions for the settings screen.
///
/// Layer order (highest precedence first):
/// 1. Confirm-exit dialog — modal; swallows all input when open.
/// 2. Active tab:
///    - `LlmConfig` → delegates entirely to `LlmSetupState::subscription`.
///    - `StrategyConfig` (editing/generating) → delegates entirely to
///      `StrategySetupState::subscription`.
///    - `StrategyConfig` (overview/nav) → settings-nav keys (tab switch,
///      save, quit) composed in a batch with `StrategySetupState::subscription`
///      so the child can still claim inner-navigation keys.
pub fn subscription(
    settings_tab: SettingsSection,
    llm_setup: &LlmSetupState,
    strategy_setup: &StrategySetupState,
    confirm_exit_settings: &ConfirmDialog,
    kb: &mut KeybindManager,
) -> Subscription<SettingsMessage> {
    // Modal layer: confirm-exit dialog intercepts everything when open.
    let confirm_sub = confirm_exit_settings
        .subscription(kb)
        .map(SettingsMessage::ConfirmExit);

    // Active tab layer.
    let tab_sub = match settings_tab {
        SettingsSection::LlmConfig => llm_setup
            .subscription(kb, true)
            .map(SettingsMessage::LlmConfig),

        SettingsSection::StrategyConfig => {
            // Determine if strategy is actively editing/generating (not in
            // overview/nav mode). In those states we delegate entirely to the
            // child; in overview mode we also add settings-nav keys on top.
            let is_editing_or_generating = strategy_setup.overview_editing
                || strategy_setup.editing_field.is_some()
                || strategy_setup.generating
                || strategy_setup.input_editing;

            let strategy_sub = strategy_setup
                .subscription(kb)
                .map(SettingsMessage::Strategy);

            if is_editing_or_generating {
                strategy_sub
            } else {
                // Overview/nav mode: add settings-nav keys (tab switch, save,
                // exit, quit) with first-match-wins precedence before the child.
                let nav_sub = settings_nav_subscription(kb);
                Subscription::batch([nav_sub, strategy_sub])
            }
        }
    };

    Subscription::batch([confirm_sub, tab_sub])
}

/// Build a subscription for settings navigation keys.
///
/// Binds `1`/`2` for tab switching, `s` for save, `Esc` for exit, and `q`
/// for quit. Used when `StrategyConfig` is in overview/navigation mode so
/// that the settings-level keys take precedence over child bindings for the
/// same keys.
fn settings_nav_subscription(kb: &mut KeybindManager) -> Subscription<SettingsMessage> {
    // Stable ID derived from a constant seed so this recipe is not rebuilt
    // unnecessarily on every frame.
    let mut hasher = DefaultHasher::new();
    0xdead_beef_cafe_u64.hash(&mut hasher);
    let sub_id = SubscriptionId::from_u64(hasher.finish());

    let recipe = KeyBindingRecipe::new(sub_id)
        .priority(PRIORITY_NORMAL)
        .bind(
            exact(KeyCode::Char('1')),
            |_| SettingsMessage::SwitchTab(SettingsSection::LlmConfig),
            KeybindHint::new("1", "LLM tab"),
        )
        .bind(
            exact(KeyCode::Char('2')),
            |_| SettingsMessage::SwitchTab(SettingsSection::StrategyConfig),
            KeybindHint::new("2", "Strategy tab"),
        )
        .bind(
            exact(KeyCode::Char('s')),
            |_| SettingsMessage::SaveStrategy,
            KeybindHint::new("s", "Save"),
        )
        .bind(
            exact(KeyCode::Esc),
            |_| SettingsMessage::ExitSettings,
            KeybindHint::new("Esc", "Exit settings"),
        )
        .bind(
            exact(KeyCode::Char('q')),
            |_| SettingsMessage::Quit,
            KeybindHint::new("q", "Quit"),
        );

    kb.subscribe(recipe)
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
        Some(UserCommand::OnboardingAction(
            OnboardingAction::GoBack | OnboardingAction::GoNext | OnboardingAction::Skip,
        )) => None,
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
