// Reusable confirmation dialog component (Elm Architecture).
//
// Extracts the shared pattern from quit_confirm and unsaved_changes_confirm
// into a configurable, self-contained component with its own state, update,
// view, and key-to-message mapping.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// A confirmation option: the key character, its display label, and its color.
#[derive(Debug, Clone)]
pub struct ConfirmOption {
    pub key: char,
    pub label: String,
    pub color: Color,
}

/// Result of a confirm dialog interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmResult {
    /// User chose an option (the char identifies which one).
    Confirmed(char),
    /// User cancelled (Esc).
    Cancelled,
}

/// Message type for the confirm dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmMessage {
    /// Open the dialog.
    Open,
    /// User pressed a key matching one of the options.
    Confirm(char),
    /// User cancelled (Esc).
    Cancel,
}

/// A reusable centered confirmation dialog.
///
/// Configurable title, dimensions, and options. Handles its own rendering
/// and key-to-message mapping.
#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub open: bool,
    title: String,
    prompt: String,
    width: u16,
    height: u16,
    options: Vec<ConfirmOption>,
}

impl ConfirmDialog {
    pub fn new(
        title: impl Into<String>,
        prompt: impl Into<String>,
        width: u16,
        height: u16,
        options: Vec<ConfirmOption>,
    ) -> Self {
        Self {
            open: false,
            title: title.into(),
            prompt: prompt.into(),
            width,
            height,
            options,
        }
    }

    /// Process a message. Returns `Some(ConfirmResult)` when a choice is made.
    pub fn update(&mut self, msg: ConfirmMessage) -> Option<ConfirmResult> {
        match msg {
            ConfirmMessage::Open => {
                self.open = true;
                None
            }
            ConfirmMessage::Confirm(ch) => {
                self.open = false;
                Some(ConfirmResult::Confirmed(ch))
            }
            ConfirmMessage::Cancel => {
                self.open = false;
                Some(ConfirmResult::Cancelled)
            }
        }
    }

    /// Convert a key event to a message, if relevant.
    ///
    /// Returns `None` if the dialog is closed or the key doesn't match any
    /// option or Esc.
    pub fn key_to_message(&self, key: KeyEvent) -> Option<ConfirmMessage> {
        if !self.open {
            return None;
        }
        match key.code {
            KeyCode::Esc => Some(ConfirmMessage::Cancel),
            KeyCode::Char(ch) => {
                // Match case-insensitively against option keys
                let lower = ch.to_ascii_lowercase();
                if self.options.iter().any(|o| o.key.to_ascii_lowercase() == lower) {
                    Some(ConfirmMessage::Confirm(lower))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Render the dialog centered on the given area.
    pub fn view(&self, frame: &mut Frame, area: Rect) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(self.width, self.height, area);
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));

        // Build prompt line: "  <prompt> (<key1>/<key2>/...)"
        let mut spans = vec![Span::raw(format!("  {} (", self.prompt))];
        for (i, opt) in self.options.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("/"));
            }
            spans.push(Span::styled(
                &opt.label,
                Style::default()
                    .fg(opt.color)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::raw(")"));

        let paragraph = Paragraph::new(Line::from(spans))
            .block(block)
            .style(Style::default().bg(Color::Black));

        frame.render_widget(paragraph, dialog_area);
    }
}

/// Convenience constructors for the two standard dialogs.
impl ConfirmDialog {
    /// Quit confirmation: "Really quit? (y/n)"
    pub fn quit() -> Self {
        Self::new(
            "Quit?",
            "Really quit?",
            28,
            5,
            vec![
                ConfirmOption { key: 'y', label: "y".into(), color: Color::Green },
                ConfirmOption { key: 'n', label: "n".into(), color: Color::Red },
            ],
        )
    }

    /// Unsaved changes confirmation: "Save changes? (y/n/Esc)"
    ///
    /// The Esc option is displayed but uses a sentinel key (`'\0'`) that will
    /// never match a `KeyCode::Char`. Esc is handled separately by
    /// `key_to_message` (via `KeyCode::Esc`), so the sentinel is purely for
    /// display purposes.
    pub fn unsaved_changes() -> Self {
        Self::new(
            "Unsaved Changes",
            "Save changes?",
            40,
            5,
            vec![
                ConfirmOption { key: 'y', label: "y".into(), color: Color::Green },
                ConfirmOption { key: 'n', label: "n".into(), color: Color::Red },
                ConfirmOption { key: '\0', label: "Esc".into(), color: Color::Cyan },
            ],
        )
    }
}

/// Compute a centered rectangle of the given size within `area`.
///
/// If the area is too small, the dialog is clamped to the available space.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let clamped_width = width.min(area.width);
    let clamped_height = height.min(area.height);

    let vertical = Layout::vertical([Constraint::Length(clamped_height)])
        .flex(Flex::Center)
        .split(area);

    let horizontal = Layout::horizontal([Constraint::Length(clamped_width)])
        .flex(Flex::Center)
        .split(vertical[0]);

    horizontal[0]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    // -- Constructor tests --

    #[test]
    fn new_starts_closed() {
        let dialog = ConfirmDialog::new("T", "P", 30, 5, vec![]);
        assert!(!dialog.open);
    }

    #[test]
    fn quit_constructor() {
        let d = ConfirmDialog::quit();
        assert_eq!(d.title, "Quit?");
        assert_eq!(d.prompt, "Really quit?");
        assert_eq!(d.width, 28);
        assert_eq!(d.height, 5);
        assert_eq!(d.options.len(), 2);
        assert!(!d.open);
    }

    #[test]
    fn unsaved_changes_constructor() {
        let d = ConfirmDialog::unsaved_changes();
        assert_eq!(d.title, "Unsaved Changes");
        assert_eq!(d.prompt, "Save changes?");
        assert_eq!(d.width, 40);
        assert_eq!(d.height, 5);
        assert_eq!(d.options.len(), 3); // y, n, Esc (display-only)
        assert!(!d.open);
    }

    // -- Update tests --

    #[test]
    fn update_open_sets_open() {
        let mut d = ConfirmDialog::quit();
        let result = d.update(ConfirmMessage::Open);
        assert!(d.open);
        assert!(result.is_none());
    }

    #[test]
    fn update_confirm_closes_and_returns_confirmed() {
        let mut d = ConfirmDialog::quit();
        d.open = true;
        let result = d.update(ConfirmMessage::Confirm('y'));
        assert!(!d.open);
        assert_eq!(result, Some(ConfirmResult::Confirmed('y')));
    }

    #[test]
    fn update_cancel_closes_and_returns_cancelled() {
        let mut d = ConfirmDialog::quit();
        d.open = true;
        let result = d.update(ConfirmMessage::Cancel);
        assert!(!d.open);
        assert_eq!(result, Some(ConfirmResult::Cancelled));
    }

    // -- key_to_message tests --

    #[test]
    fn key_to_message_when_closed_returns_none() {
        let d = ConfirmDialog::quit();
        assert!(!d.open);
        assert!(d.key_to_message(key(KeyCode::Char('y'))).is_none());
    }

    #[test]
    fn key_to_message_matching_option() {
        let mut d = ConfirmDialog::quit();
        d.open = true;
        assert_eq!(
            d.key_to_message(key(KeyCode::Char('y'))),
            Some(ConfirmMessage::Confirm('y'))
        );
    }

    #[test]
    fn key_to_message_case_insensitive() {
        let mut d = ConfirmDialog::quit();
        d.open = true;
        assert_eq!(
            d.key_to_message(key(KeyCode::Char('Y'))),
            Some(ConfirmMessage::Confirm('y'))
        );
    }

    #[test]
    fn key_to_message_esc_returns_cancel() {
        let mut d = ConfirmDialog::quit();
        d.open = true;
        assert_eq!(
            d.key_to_message(key(KeyCode::Esc)),
            Some(ConfirmMessage::Cancel)
        );
    }

    #[test]
    fn key_to_message_unrecognized_returns_none() {
        let mut d = ConfirmDialog::quit();
        d.open = true;
        assert!(d.key_to_message(key(KeyCode::Char('z'))).is_none());
        assert!(d.key_to_message(key(KeyCode::Enter)).is_none());
    }

    // -- centered_rect tests --

    #[test]
    fn centered_rect_is_centered() {
        let area = Rect::new(0, 0, 80, 24);
        let result = centered_rect(28, 5, area);
        assert_eq!(result.width, 28);
        assert_eq!(result.height, 5);
        let center_x = area.width / 2;
        let center_y = area.height / 2;
        let result_center_x = result.x + result.width / 2;
        let result_center_y = result.y + result.height / 2;
        assert!(
            (result_center_x as i32 - center_x as i32).unsigned_abs() <= 1,
            "Dialog should be horizontally centered: {} vs {}",
            result_center_x,
            center_x,
        );
        assert!(
            (result_center_y as i32 - center_y as i32).unsigned_abs() <= 1,
            "Dialog should be vertically centered: {} vs {}",
            result_center_y,
            center_y,
        );
    }

    #[test]
    fn centered_rect_clamps_to_small_area() {
        let area = Rect::new(0, 0, 10, 3);
        let result = centered_rect(28, 5, area);
        assert!(result.width <= area.width);
        assert!(result.height <= area.height);
    }

    #[test]
    fn centered_rect_handles_zero_area() {
        let area = Rect::new(0, 0, 0, 0);
        let result = centered_rect(28, 5, area);
        assert_eq!(result.width, 0);
        assert_eq!(result.height, 0);
    }

    // -- View tests --

    #[test]
    fn view_does_not_panic_when_open() {
        let mut d = ConfirmDialog::quit();
        d.open = true;
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| d.view(frame, frame.area())).unwrap();
    }

    #[test]
    fn view_when_closed_is_noop() {
        let d = ConfirmDialog::quit();
        assert!(!d.open);
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        // Should not panic and should not render anything
        terminal.draw(|frame| d.view(frame, frame.area())).unwrap();
    }
}
