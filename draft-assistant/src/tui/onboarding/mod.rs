// Onboarding screen dispatcher: routes rendering to the correct step screen.

pub mod llm_setup;
pub mod strategy_setup;

use ratatui::layout::{Constraint, Layout};
use ratatui::Frame;

use crate::onboarding::OnboardingStep;
use crate::tui::ViewState;

/// Render the active onboarding step.
///
/// Dispatches to the appropriate step-specific renderer based on the
/// current `OnboardingStep`. The `Complete` step should never be rendered
/// (the app should have transitioned to `AppMode::Draft` before reaching
/// this point).
pub fn render(frame: &mut Frame, step: &OnboardingStep, state: &ViewState) {
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
    use crate::tui::ViewState;

    #[test]
    fn render_llm_setup_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, &OnboardingStep::LlmSetup, &state))
            .unwrap();
    }

    #[test]
    fn render_strategy_placeholder_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, &OnboardingStep::StrategySetup, &state))
            .unwrap();
    }

    #[test]
    fn render_complete_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, &OnboardingStep::Complete, &state))
            .unwrap();
    }
}
