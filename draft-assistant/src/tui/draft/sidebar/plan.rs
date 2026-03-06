// PlanPanel component: wraps LlmStreamState with nomination plan chrome.
//
// Renders Claude's nomination plan with:
// - Title with status indicator (Idle/Streaming/Complete/Error with colors)
// - Auto-scroll to bottom while streaming
// - User-controlled scroll when not streaming
// - Word wrap, scrollbar when content overflows
// - Status-dependent border color (yellow=streaming, red=error, cyan=focused)
// - Placeholder text when empty

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;

use crate::protocol::LlmStatus;
use crate::tui::action::Action;
use crate::tui::llm_stream::{LlmStreamMessage, LlmStreamState};
use crate::tui::scroll::ScrollDirection;
use crate::tui::widgets::focused_border_style;

/// Messages that can be sent to the PlanPanel component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanPanelMessage {
    Stream(LlmStreamMessage),
    Scroll(ScrollDirection),
}

/// PlanPanel component: LLM nomination plan rendering with status chrome.
pub struct PlanPanel {
    stream: LlmStreamState,
}

/// Page size for PageUp/PageDown scrolling (matches TUI input convention).
const PAGE_SIZE: usize = 20;

impl PlanPanel {
    pub fn new() -> Self {
        Self {
            stream: LlmStreamState::new(),
        }
    }

    pub fn update(&mut self, msg: PlanPanelMessage) -> Option<Action> {
        match msg {
            PlanPanelMessage::Stream(stream_msg) => self.stream.update(stream_msg),
            PlanPanelMessage::Scroll(dir) => {
                self.stream.scroll(dir, PAGE_SIZE);
                None
            }
        }
    }

    /// Map a key event to a PlanPanelMessage, if applicable.
    pub fn key_to_message(&self, key: KeyEvent) -> Option<PlanPanelMessage> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                Some(PlanPanelMessage::Scroll(ScrollDirection::Up))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                Some(PlanPanelMessage::Scroll(ScrollDirection::Down))
            }
            KeyCode::PageUp => Some(PlanPanelMessage::Scroll(ScrollDirection::PageUp)),
            KeyCode::PageDown => Some(PlanPanelMessage::Scroll(ScrollDirection::PageDown)),
            KeyCode::Home => Some(PlanPanelMessage::Scroll(ScrollDirection::Top)),
            KeyCode::End => Some(PlanPanelMessage::Scroll(ScrollDirection::Bottom)),
            _ => None,
        }
    }

    /// Access the stream text (for parent to read if needed).
    pub fn text(&self) -> &str {
        &self.stream.text
    }

    /// Access the stream status.
    pub fn status(&self) -> LlmStatus {
        self.stream.status
    }

    /// Raw scroll offset (for testing/inspection).
    pub fn scroll_offset(&self) -> usize {
        self.stream.scroll_offset()
    }

    /// Render the plan panel into the given area.
    pub fn view(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let title_line = build_title(self.stream.status);

        let content = if self.stream.text.is_empty() {
            placeholder_text(self.stream.status)
        } else {
            self.stream.text.clone()
        };

        // Compute scroll: auto-scroll to bottom while streaming
        let inner_height = area.height.saturating_sub(2) as usize; // subtract border
        let line_count = content.lines().count();
        let scroll = if self.stream.status == LlmStatus::Streaming && line_count > inner_height {
            (line_count - inner_height) as u16
        } else {
            self.stream.scroll_offset_clamped(inner_height) as u16
        };

        let effective_border = focused_border_style(focused, border_style(self.stream.status));

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

        // Render vertical scrollbar whenever content overflows
        if line_count > inner_height {
            let mut scrollbar_state = ScrollbarState::new(line_count.saturating_sub(inner_height))
                .position(scroll as usize);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area.inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }
    }
}

impl Default for PlanPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the title line with status indicator.
fn build_title(status: LlmStatus) -> Line<'static> {
    let (status_text, status_color) = status_indicator(status);
    Line::from(vec![
        Span::styled(
            "Nomination Plan",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(" -- ", Style::default().fg(Color::DarkGray)),
        Span::styled(status_text, Style::default().fg(status_color)),
    ])
}

/// Return status text and color for the plan status.
pub fn status_indicator(status: LlmStatus) -> (&'static str, Color) {
    match status {
        LlmStatus::Idle => ("not yet computed", Color::DarkGray),
        LlmStatus::Streaming => ("streaming...", Color::Yellow),
        LlmStatus::Complete => ("ready", Color::Green),
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

/// Placeholder text when plan text is empty.
fn placeholder_text(status: LlmStatus) -> String {
    match status {
        LlmStatus::Idle => "No nomination plan yet.".to_string(),
        LlmStatus::Streaming => "Streaming...".to_string(),
        LlmStatus::Complete => "Plan complete (empty).".to_string(),
        LlmStatus::Error => "Plan error.".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // -- Construction --

    #[test]
    fn new_starts_with_idle_status_and_empty_text() {
        let panel = PlanPanel::new();
        assert_eq!(panel.status(), LlmStatus::Idle);
        assert_eq!(panel.text(), "");
    }

    #[test]
    fn default_starts_with_idle_status_and_empty_text() {
        let panel = PlanPanel::default();
        assert_eq!(panel.status(), LlmStatus::Idle);
        assert_eq!(panel.text(), "");
    }

    // -- Stream updates --

    #[test]
    fn update_stream_token_appends_text() {
        let mut panel = PlanPanel::new();
        let result = panel.update(PlanPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("hello ".into()),
        ));
        assert_eq!(result, None);
        assert_eq!(panel.text(), "hello ");
        assert_eq!(panel.status(), LlmStatus::Streaming);

        panel.update(PlanPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("world".into()),
        ));
        assert_eq!(panel.text(), "hello world");
    }

    #[test]
    fn update_stream_complete_sets_final_text() {
        let mut panel = PlanPanel::new();
        panel.update(PlanPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("partial".into()),
        ));
        let result = panel.update(PlanPanelMessage::Stream(LlmStreamMessage::Complete(
            "final text".into(),
        )));
        assert_eq!(result, None);
        assert_eq!(panel.text(), "final text");
        assert_eq!(panel.status(), LlmStatus::Complete);
    }

    #[test]
    fn update_stream_error_sets_error() {
        let mut panel = PlanPanel::new();
        panel.update(PlanPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("data".into()),
        ));
        let result = panel.update(PlanPanelMessage::Stream(LlmStreamMessage::Error(
            "timeout".into(),
        )));
        assert_eq!(result, None);
        assert_eq!(panel.text(), "[Error: timeout]");
        assert_eq!(panel.status(), LlmStatus::Error);
    }

    #[test]
    fn update_stream_clear_resets() {
        let mut panel = PlanPanel::new();
        panel.update(PlanPanelMessage::Stream(
            LlmStreamMessage::TokenReceived("data".into()),
        ));
        let result = panel.update(PlanPanelMessage::Stream(LlmStreamMessage::Clear));
        assert_eq!(result, None);
        assert_eq!(panel.text(), "");
        assert_eq!(panel.status(), LlmStatus::Idle);
    }

    // -- Scroll --

    #[test]
    fn update_scroll_down_scrolls() {
        let mut panel = PlanPanel::new();
        panel.update(PlanPanelMessage::Scroll(ScrollDirection::Down));
        assert_eq!(panel.scroll_offset(), 1);
    }

    #[test]
    fn update_scroll_returns_none() {
        let mut panel = PlanPanel::new();
        let result = panel.update(PlanPanelMessage::Scroll(ScrollDirection::Down));
        assert_eq!(result, None);
    }

    // -- key_to_message --

    #[test]
    fn key_to_message_up() {
        let panel = PlanPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Up)),
            Some(PlanPanelMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_down() {
        let panel = PlanPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Down)),
            Some(PlanPanelMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_k_scrolls_up() {
        let panel = PlanPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('k'))),
            Some(PlanPanelMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_j_scrolls_down() {
        let panel = PlanPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('j'))),
            Some(PlanPanelMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_page_up() {
        let panel = PlanPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageUp)),
            Some(PlanPanelMessage::Scroll(ScrollDirection::PageUp))
        );
    }

    #[test]
    fn key_to_message_page_down() {
        let panel = PlanPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageDown)),
            Some(PlanPanelMessage::Scroll(ScrollDirection::PageDown))
        );
    }

    #[test]
    fn key_to_message_home() {
        let panel = PlanPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Home)),
            Some(PlanPanelMessage::Scroll(ScrollDirection::Top))
        );
    }

    #[test]
    fn key_to_message_end() {
        let panel = PlanPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::End)),
            Some(PlanPanelMessage::Scroll(ScrollDirection::Bottom))
        );
    }

    #[test]
    fn key_to_message_unrecognized_returns_none() {
        let panel = PlanPanel::new();
        assert_eq!(panel.key_to_message(key(KeyCode::Char('x'))), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Enter)), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Esc)), None);
    }

    // -- Status indicator --

    #[test]
    fn status_indicator_values() {
        assert_eq!(status_indicator(LlmStatus::Idle).0, "not yet computed");
        assert_eq!(status_indicator(LlmStatus::Streaming).0, "streaming...");
        assert_eq!(status_indicator(LlmStatus::Complete).0, "ready");
        assert_eq!(status_indicator(LlmStatus::Error).0, "error");
    }

    #[test]
    fn status_indicator_colors() {
        assert_eq!(status_indicator(LlmStatus::Idle).1, Color::DarkGray);
        assert_eq!(status_indicator(LlmStatus::Streaming).1, Color::Yellow);
        assert_eq!(status_indicator(LlmStatus::Complete).1, Color::Green);
        assert_eq!(status_indicator(LlmStatus::Error).1, Color::Red);
    }

    // -- Placeholder text --

    #[test]
    fn placeholder_text_values() {
        assert_eq!(
            placeholder_text(LlmStatus::Idle),
            "No nomination plan yet."
        );
        assert_eq!(placeholder_text(LlmStatus::Streaming), "Streaming...");
        assert_eq!(
            placeholder_text(LlmStatus::Complete),
            "Plan complete (empty)."
        );
        assert_eq!(placeholder_text(LlmStatus::Error), "Plan error.");
    }

    // -- View (render) doesn't panic --

    #[test]
    fn view_does_not_panic_with_defaults() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = PlanPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_text() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = PlanPanel::new();
        panel.update(PlanPanelMessage::Stream(LlmStreamMessage::Complete(
            "Nominate Player X at $15.\nAlternative: Player Y at $12.".into(),
        )));
        terminal
            .draw(|frame| panel.view(frame, frame.area(), false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_streaming() {
        let backend = ratatui::backend::TestBackend::new(80, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = PlanPanel::new();
        // Create enough text to trigger auto-scroll
        let long_text = (0..50)
            .map(|i| format!("Line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        for chunk in long_text.as_bytes().chunks(20) {
            panel.update(PlanPanelMessage::Stream(
                LlmStreamMessage::TokenReceived(String::from_utf8_lossy(chunk).into_owned()),
            ));
        }
        terminal
            .draw(|frame| panel.view(frame, frame.area(), false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = PlanPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), true))
            .unwrap();
    }
}
