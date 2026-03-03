// Onboarding screen dispatcher: routes rendering to the correct step screen.

pub mod llm_setup;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
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
    match step {
        OnboardingStep::LlmSetup => {
            llm_setup::render(frame, frame.area(), &state.llm_setup);
        }
        OnboardingStep::StrategySetup => {
            // Placeholder until Task 5 implements the strategy setup screen
            render_strategy_placeholder(frame, frame.area());
        }
        OnboardingStep::Complete => {
            // Should not reach here -- the app transitions to Draft mode
            // when onboarding completes. Render a fallback just in case.
            render_strategy_placeholder(frame, frame.area());
        }
    }
}

/// Placeholder renderer for the strategy setup step (Task 5).
fn render_strategy_placeholder(frame: &mut Frame, area: Rect) {
    let outer = Layout::vertical([
        Constraint::Percentage(40),
        Constraint::Length(3),
        Constraint::Percentage(40),
        Constraint::Length(1),
    ])
    .split(area);

    let message = Paragraph::new(Line::from(vec![
        Span::styled(
            "Strategy Configuration (coming soon)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(Color::Black));

    frame.render_widget(message, outer[1]);

    let help = Paragraph::new(Line::from(vec![
        Span::styled(" n:next", Style::default().fg(Color::Gray)),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc:back", Style::default().fg(Color::Gray)),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled("q:quit", Style::default().fg(Color::Gray)),
    ]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(Color::Black));

    frame.render_widget(help, outer[3]);
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
