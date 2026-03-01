// Quit confirmation overlay widget.
//
// Renders a centered modal dialog asking the user to confirm quitting.
// Displayed on top of the main layout when `ViewState::confirm_quit` is true.

use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Width and height of the confirmation dialog.
const DIALOG_WIDTH: u16 = 28;
const DIALOG_HEIGHT: u16 = 5;

/// Render the quit confirmation overlay centered on the screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let dialog_area = centered_rect(DIALOG_WIDTH, DIALOG_HEIGHT, area);

    // Clear the area behind the dialog so it renders cleanly on top
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " Quit? ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));

    let text = Line::from(vec![
        Span::raw("  Really quit? ("),
        Span::styled("y", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("/"),
        Span::styled("n", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::raw(")"),
    ]);

    let paragraph = Paragraph::new(text)
        .block(block)
        .style(Style::default().bg(Color::Black));

    frame.render_widget(paragraph, dialog_area);
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

    #[test]
    fn centered_rect_is_centered() {
        let area = Rect::new(0, 0, 80, 24);
        let result = centered_rect(DIALOG_WIDTH, DIALOG_HEIGHT, area);
        assert_eq!(result.width, DIALOG_WIDTH);
        assert_eq!(result.height, DIALOG_HEIGHT);
        // Should be roughly centered (within 1 pixel due to integer division)
        let center_x = area.width / 2;
        let center_y = area.height / 2;
        let result_center_x = result.x + result.width / 2;
        let result_center_y = result.y + result.height / 2;
        assert!(
            (result_center_x as i32 - center_x as i32).unsigned_abs() <= 1,
            "Dialog should be horizontally centered: dialog center {} vs area center {}",
            result_center_x,
            center_x,
        );
        assert!(
            (result_center_y as i32 - center_y as i32).unsigned_abs() <= 1,
            "Dialog should be vertically centered: dialog center {} vs area center {}",
            result_center_y,
            center_y,
        );
    }

    #[test]
    fn centered_rect_clamps_to_small_area() {
        let area = Rect::new(0, 0, 10, 3);
        let result = centered_rect(DIALOG_WIDTH, DIALOG_HEIGHT, area);
        assert!(result.width <= area.width);
        assert!(result.height <= area.height);
    }

    #[test]
    fn render_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area()))
            .unwrap();
    }
}
