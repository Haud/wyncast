// Position filter modal overlay widget.
//
// Renders a centered modal dialog that lets the user select a position filter
// for the Available Players tab.  The user can:
//   - Navigate the list with Up/Down arrow keys
//   - Type to incrementally search / narrow the list
//   - Press Enter to apply the highlighted selection
//   - Press Escape to cancel without changing the current filter
//
// The modal is shown when `ViewState::position_filter_modal.open` is true.

use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::tui::{PositionFilterModal, ViewState};

/// Width of the modal dialog.
const MODAL_WIDTH: u16 = 30;

/// Render the position filter modal centered on the screen.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState) {
    let modal = &state.position_filter_modal;
    let options = modal.filtered_options();

    // Height: border(2) + search bar(1) + separator(0) + items (up to 14, min 3)
    let item_count = options.len().max(1).min(14) as u16;
    let modal_height = 2 + 1 + item_count; // borders + search row + list items
    let modal_area = centered_rect(MODAL_WIDTH, modal_height, area);

    // Clear background behind the modal
    frame.render_widget(Clear, modal_area);

    // Outer block
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Filter by Position ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    if inner_area.height == 0 || inner_area.width == 0 {
        return;
    }

    // Split inner area: search bar (1 row) + list (remaining)
    let inner_chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner_area);

    let search_area = inner_chunks[0];
    let list_area = inner_chunks[1];

    // --- Search bar ---
    let search_spans = vec![
        Span::styled(
            ">",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            modal.search_text.value(),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "▎",
            Style::default().fg(Color::Cyan),
        ),
    ];
    let search_paragraph = Paragraph::new(Line::from(search_spans));
    frame.render_widget(search_paragraph, search_area);

    // --- Options list ---
    if list_area.height == 0 {
        return;
    }

    let items: Vec<ListItem> = options
        .iter()
        .map(|opt| {
            let label = PositionFilterModal::option_label(*opt);
            ListItem::new(Line::from(format!(" {} ", label)))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    // Clamp selected_index to the filtered list length
    let clamped_index = if options.is_empty() {
        0
    } else {
        modal.selected_index.min(options.len() - 1)
    };

    let mut list_state = ListState::default();
    if !options.is_empty() {
        list_state.select(Some(clamped_index));
    }

    frame.render_stateful_widget(list, list_area, &mut list_state);
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
        let result = centered_rect(MODAL_WIDTH, 10, area);
        assert_eq!(result.width, MODAL_WIDTH);
        assert_eq!(result.height, 10);
        let center_x = area.width / 2;
        let result_center_x = result.x + result.width / 2;
        assert!(
            (result_center_x as i32 - center_x as i32).unsigned_abs() <= 1,
            "Modal should be horizontally centered"
        );
    }

    #[test]
    fn centered_rect_clamps_to_small_area() {
        let area = Rect::new(0, 0, 10, 3);
        let result = centered_rect(MODAL_WIDTH, 10, area);
        assert!(result.width <= area.width);
        assert!(result.height <= area.height);
    }

    #[test]
    fn centered_rect_handles_zero_area() {
        let area = Rect::new(0, 0, 0, 0);
        let result = centered_rect(MODAL_WIDTH, 10, area);
        assert_eq!(result.width, 0);
        assert_eq!(result.height, 0);
    }

    #[test]
    fn render_does_not_panic_when_modal_open() {
        let mut state = crate::tui::ViewState::default();
        state.position_filter_modal.open = true;
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_search_text() {
        let mut state = crate::tui::ViewState::default();
        state.position_filter_modal.open = true;
        state.position_filter_modal.search_text.set_value("1");
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_no_matches() {
        let mut state = crate::tui::ViewState::default();
        state.position_filter_modal.open = true;
        state.position_filter_modal.search_text.set_value("ZZZZ");
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_on_small_terminal() {
        let mut state = crate::tui::ViewState::default();
        state.position_filter_modal.open = true;
        let backend = ratatui::backend::TestBackend::new(10, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
