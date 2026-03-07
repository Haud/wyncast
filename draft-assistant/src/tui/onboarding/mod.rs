// Onboarding screen dispatcher: routes rendering and input to the correct step.

pub mod llm_setup;
pub mod strategy_setup;

use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Layout};
use ratatui::Frame;

use crate::onboarding::OnboardingStep;
use crate::protocol::UserCommand;
use crate::tui::app::App;

use self::llm_setup::{LlmSetupMessage, LlmSetupState};
use self::strategy_setup::{StrategySetupMessage, StrategySetupState};

#[derive(Debug, Clone, PartialEq)]
pub enum OnboardingMessage {
    LlmSetup(LlmSetupMessage),
    Strategy(StrategySetupMessage),
}

pub fn key_to_message(
    step: &OnboardingStep,
    llm_setup: &LlmSetupState,
    strategy_setup: &StrategySetupState,
    key: KeyEvent,
) -> Option<OnboardingMessage> {
    match step {
        OnboardingStep::LlmSetup => {
            // LlmSetupState key routing is handled by the subscription system
            // via LlmSetupState::subscription(). This function is retained for
            // non-subscription callers but LlmSetup routing is a no-op here.
            let _ = (llm_setup, key);
            None
        }
        OnboardingStep::StrategySetup | OnboardingStep::Complete => {
            // StrategySetupState key routing is handled by the subscription system
            // via StrategySetupState::subscription(). This function is retained for
            // non-subscription callers but StrategySetup routing is a no-op here.
            let _ = (strategy_setup, key);
            None
        }
    }
}

pub fn update(
    _step: &OnboardingStep,
    llm_setup: &mut LlmSetupState,
    strategy_setup: &mut StrategySetupState,
    msg: OnboardingMessage,
) -> Option<UserCommand> {
    match msg {
        OnboardingMessage::LlmSetup(m) => llm_setup.update(m),
        OnboardingMessage::Strategy(m) => strategy_setup.update(m),
    }
}

/// Render the active onboarding step.
///
/// Dispatches to the appropriate step-specific renderer based on the
/// current `OnboardingStep`. The `Complete` step should never be rendered
/// (the app should have transitioned to `AppMode::Draft` before reaching
/// this point).
pub fn render(frame: &mut Frame, step: &OnboardingStep, state: &App) {
    // Split: content | help bar (1 line)
    let outer = Layout::vertical([
        Constraint::Min(0),    // content area
        Constraint::Length(1), // help bar
    ])
    .split(frame.area());

    let content_area = outer[0];

    match step {
        OnboardingStep::LlmSetup => {
            llm_setup::render(frame, content_area, &state.llm_setup);
        }
        OnboardingStep::StrategySetup => {
            strategy_setup::render(frame, content_area, &state.strategy_setup);
        }
        OnboardingStep::Complete => {
            // Should not reach here -- the app transitions to Draft mode
            // when onboarding completes. Render a fallback just in case.
            strategy_setup::render(frame, content_area, &state.strategy_setup);
        }
    }

    // --- Help bar: render the pre-synced keybind hints ---
    super::render_help_bar(frame, outer[1], state, &state.active_keybinds);
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;

    #[test]
    fn render_llm_setup_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = App::default();
        terminal
            .draw(|frame| render(frame, &OnboardingStep::LlmSetup, &state))
            .unwrap();
    }

    #[test]
    fn render_strategy_placeholder_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = App::default();
        terminal
            .draw(|frame| render(frame, &OnboardingStep::StrategySetup, &state))
            .unwrap();
    }

    #[test]
    fn render_complete_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = App::default();
        terminal
            .draw(|frame| render(frame, &OnboardingStep::Complete, &state))
            .unwrap();
    }
}
