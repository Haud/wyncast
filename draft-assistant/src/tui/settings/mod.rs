// Settings screen: tabbed pane reusing onboarding widgets for LLM and strategy config.
//
// Accessible from draft mode via the `,` keybind. Renders a tabbed layout that
// delegates to the same render functions used by the onboarding wizard, avoiding
// code duplication.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::protocol::SettingsSection;
use crate::tui::ViewState;

/// Render the settings screen into the given frame.
///
/// Draws a tabbed layout with LLM and Strategy tabs, delegating content
/// rendering to the existing onboarding widgets. The active tab is determined
/// by `view_state.settings_tab`.
pub fn render(frame: &mut Frame, state: &ViewState) {
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
    use crate::tui::ViewState;

    #[test]
    fn render_llm_tab_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.settings_tab = SettingsSection::LlmConfig;
        terminal
            .draw(|frame| render(frame, &state))
            .unwrap();
    }

    #[test]
    fn render_strategy_tab_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.settings_tab = SettingsSection::StrategyConfig;
        terminal
            .draw(|frame| render(frame, &state))
            .unwrap();
    }

    #[test]
    fn render_small_terminal_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.settings_tab = SettingsSection::LlmConfig;
        terminal
            .draw(|frame| render(frame, &state))
            .unwrap();
    }

    #[test]
    fn render_with_editing_state_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.settings_tab = SettingsSection::LlmConfig;
        state.llm_setup.api_key_editing = true;
        terminal
            .draw(|frame| render(frame, &state))
            .unwrap();
    }
}
