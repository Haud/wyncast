// LLM analysis widget: streams Claude's analysis of the current nomination.
//
// Header: "LLM Analysis -- streaming.../complete/algorithmic only"
// Body: view_state.analysis_text with word wrap
// Auto-scroll to bottom while streaming

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::protocol::LlmStatus;
use crate::tui::ViewState;
use super::focused_border_style;

/// Render the LLM analysis panel into the given area.
///
/// When `focused` is true, the border is highlighted to indicate this panel
/// has keyboard focus for scroll routing.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState, focused: bool) {
    let title_line = build_title(state.analysis_status);

    let content = if state.analysis_text.is_empty() {
        placeholder_text(state.analysis_status)
    } else {
        state.analysis_text.clone()
    };

    // Compute scroll: auto-scroll to bottom while streaming
    let inner_height = area.height.saturating_sub(2) as usize; // subtract border
    let line_count = content.lines().count();
    let scroll = if state.analysis_status == LlmStatus::Streaming && line_count > inner_height {
        (line_count - inner_height) as u16
    } else {
        let offset = state.scroll_offset.get("analysis").copied().unwrap_or(0);
        offset as u16
    };

    let effective_border = focused_border_style(focused, border_style(state.analysis_status));

    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title_line)
                .border_style(effective_border),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

/// Build the title line with status indicator.
fn build_title(status: LlmStatus) -> Line<'static> {
    let (status_text, status_color) = status_indicator(status);
    Line::from(vec![
        Span::styled(
            "LLM Analysis",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(" -- ", Style::default().fg(Color::DarkGray)),
        Span::styled(status_text, Style::default().fg(status_color)),
    ])
}

/// Return status text and color for the LLM status.
pub fn status_indicator(status: LlmStatus) -> (&'static str, Color) {
    match status {
        LlmStatus::Idle => ("algorithmic only", Color::DarkGray),
        LlmStatus::Streaming => ("streaming...", Color::Yellow),
        LlmStatus::Complete => ("complete", Color::Green),
        LlmStatus::Error => ("error", Color::Red),
    }
}

/// Border style varies by status.
fn border_style(status: LlmStatus) -> Style {
    match status {
        LlmStatus::Streaming => Style::default().fg(Color::Yellow),
        LlmStatus::Error => Style::default().fg(Color::Red),
        _ => Style::default(),
    }
}

/// Placeholder text when analysis_text is empty.
fn placeholder_text(status: LlmStatus) -> String {
    match status {
        LlmStatus::Idle => "Waiting for nomination...".to_string(),
        LlmStatus::Streaming => "Streaming...".to_string(),
        LlmStatus::Complete => "Analysis complete (empty).".to_string(),
        LlmStatus::Error => "Analysis error.".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_indicator_values() {
        assert_eq!(status_indicator(LlmStatus::Idle).0, "algorithmic only");
        assert_eq!(status_indicator(LlmStatus::Streaming).0, "streaming...");
        assert_eq!(status_indicator(LlmStatus::Complete).0, "complete");
        assert_eq!(status_indicator(LlmStatus::Error).0, "error");
    }

    #[test]
    fn status_indicator_colors() {
        assert_eq!(status_indicator(LlmStatus::Idle).1, Color::DarkGray);
        assert_eq!(status_indicator(LlmStatus::Streaming).1, Color::Yellow);
        assert_eq!(status_indicator(LlmStatus::Complete).1, Color::Green);
        assert_eq!(status_indicator(LlmStatus::Error).1, Color::Red);
    }

    #[test]
    fn placeholder_text_values() {
        assert_eq!(placeholder_text(LlmStatus::Idle), "Waiting for nomination...");
        assert_eq!(placeholder_text(LlmStatus::Streaming), "Streaming...");
        assert_eq!(
            placeholder_text(LlmStatus::Complete),
            "Analysis complete (empty)."
        );
        assert_eq!(placeholder_text(LlmStatus::Error), "Analysis error.");
    }

    #[test]
    fn render_does_not_panic_with_defaults() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state, false))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_text() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.analysis_text = "This is a test analysis.\nSecond line.".to_string();
        state.analysis_status = LlmStatus::Complete;
        terminal
            .draw(|frame| render(frame, frame.area(), &state, false))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_streaming() {
        let backend = ratatui::backend::TestBackend::new(80, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        // Create enough text to trigger auto-scroll
        state.analysis_text = (0..50).map(|i| format!("Line {}", i)).collect::<Vec<_>>().join("\n");
        state.analysis_status = LlmStatus::Streaming;
        terminal
            .draw(|frame| render(frame, frame.area(), &state, false))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state, true))
            .unwrap();
    }
}
