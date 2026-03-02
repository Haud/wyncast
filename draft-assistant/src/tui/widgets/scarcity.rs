// Scarcity widget: positional scarcity index heat map.
//
// One row per position with visual gauge/bar.
// Color: Red=Critical, Yellow=High, Blue=Medium, Green=Low
// Mark nominated player's position.
// Scrollable via Tab-focus and arrow keys.

use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::draft::pick::Position;
use crate::valuation::scarcity::{ScarcityEntry, ScarcityUrgency};
use crate::tui::ViewState;
use super::focused_border_style;

/// Render the scarcity gauges into the given area.
///
/// When `focused` is true, the border is highlighted in cyan to indicate this
/// panel has keyboard focus for scroll routing.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState, focused: bool) {
    let border = focused_border_style(focused, Style::default());

    if state.positional_scarcity.is_empty() {
        let paragraph = Paragraph::new("  No scarcity data.")
            .style(Style::default().fg(Color::DarkGray))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border)
                    .title("Scarcity"),
            );
        frame.render_widget(paragraph, area);
        return;
    }

    // Determine the nominated player's position for marking
    let nominated_position = state
        .current_nomination
        .as_ref()
        .and_then(|n| Position::from_str_pos(&n.position));

    let scroll_offset = state.scroll_offset.get("scarcity").copied().unwrap_or(0);

    // Visible row count: subtract 2 for borders
    let visible_rows = (area.height as usize).saturating_sub(2);
    let total = state.positional_scarcity.len();

    // Clamp scroll offset
    let max_offset = total.saturating_sub(visible_rows);
    let scroll_offset = scroll_offset.min(max_offset);

    let items: Vec<ListItem> = state
        .positional_scarcity
        .iter()
        .skip(scroll_offset)
        .take(visible_rows.max(1))
        .map(|entry| {
            let is_nominated = nominated_position.map_or(false, |pos| entry.position == pos);
            format_scarcity_entry(entry, is_nominated)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title("Scarcity"),
    );
    frame.render_widget(list, area);

    // Render vertical scrollbar if content overflows
    if total > visible_rows {
        let mut scrollbar_state = ScrollbarState::new(total.saturating_sub(visible_rows))
            .position(scroll_offset);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }
}

/// Format a scarcity entry as a ListItem with a visual gauge.
fn format_scarcity_entry<'a>(entry: &ScarcityEntry, is_nominated: bool) -> ListItem<'a> {
    let pos_label = entry.position.display_str();
    let urgency_label = format_urgency(entry.urgency);
    let color = urgency_color(entry.urgency);
    let bar = urgency_bar(entry.players_above_replacement);

    let marker = if is_nominated { ">" } else { " " };

    let spans = vec![
        Span::styled(
            format!("{}{:>3} ", marker, pos_label),
            if is_nominated {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        ),
        Span::styled(bar, Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            format!("{:>8}", urgency_label),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" ({})", entry.players_above_replacement),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    ListItem::new(Line::from(spans))
}

/// Return the color for a scarcity urgency level.
pub fn urgency_color(urgency: ScarcityUrgency) -> Color {
    match urgency {
        ScarcityUrgency::Critical => Color::Red,
        ScarcityUrgency::High => Color::Yellow,
        ScarcityUrgency::Medium => Color::Blue,
        ScarcityUrgency::Low => Color::Green,
    }
}

/// Return a visual bar based on player count (more players = longer bar).
pub fn urgency_bar(players_above_replacement: usize) -> String {
    let max_bar = 8;
    let filled = players_above_replacement.min(max_bar);
    let empty = max_bar - filled;
    format!("[{}{}]", "#".repeat(filled), "-".repeat(empty))
}

/// Return a human-readable urgency label.
pub fn format_urgency(urgency: ScarcityUrgency) -> &'static str {
    match urgency {
        ScarcityUrgency::Critical => "CRITICAL",
        ScarcityUrgency::High => "HIGH",
        ScarcityUrgency::Medium => "MEDIUM",
        ScarcityUrgency::Low => "LOW",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urgency_color_values() {
        assert_eq!(urgency_color(ScarcityUrgency::Critical), Color::Red);
        assert_eq!(urgency_color(ScarcityUrgency::High), Color::Yellow);
        assert_eq!(urgency_color(ScarcityUrgency::Medium), Color::Blue);
        assert_eq!(urgency_color(ScarcityUrgency::Low), Color::Green);
    }

    #[test]
    fn urgency_bar_empty() {
        assert_eq!(urgency_bar(0), "[--------]");
    }

    #[test]
    fn urgency_bar_partial() {
        assert_eq!(urgency_bar(3), "[###-----]");
    }

    #[test]
    fn urgency_bar_full() {
        assert_eq!(urgency_bar(8), "[########]");
    }

    #[test]
    fn urgency_bar_overflow() {
        // More than max_bar should cap at max
        assert_eq!(urgency_bar(15), "[########]");
    }

    #[test]
    fn format_urgency_values() {
        assert_eq!(format_urgency(ScarcityUrgency::Critical), "CRITICAL");
        assert_eq!(format_urgency(ScarcityUrgency::High), "HIGH");
        assert_eq!(format_urgency(ScarcityUrgency::Medium), "MEDIUM");
        assert_eq!(format_urgency(ScarcityUrgency::Low), "LOW");
    }

    #[test]
    fn render_does_not_panic_empty() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state, false))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_data() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.positional_scarcity = vec![
            ScarcityEntry {
                position: Position::Catcher,
                players_above_replacement: 2,
                top_available_vor: 8.0,
                replacement_vor: 2.0,
                dropoff: 6.0,
                urgency: ScarcityUrgency::Critical,
            },
            ScarcityEntry {
                position: Position::FirstBase,
                players_above_replacement: 6,
                top_available_vor: 10.0,
                replacement_vor: 5.0,
                dropoff: 5.0,
                urgency: ScarcityUrgency::Medium,
            },
        ];
        terminal
            .draw(|frame| render(frame, frame.area(), &state, false))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state, true))
            .unwrap();
    }
}
