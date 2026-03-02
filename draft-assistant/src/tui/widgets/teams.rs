// Teams widget: overview of all teams' rosters and remaining budgets.
//
// Summary table of all teams: name, budget remaining, slots filled.

use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table};
use ratatui::Frame;

use crate::tui::ViewState;

/// Render the teams overview into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState) {
    let scroll_offset = state.scroll_offset.get("teams").copied().unwrap_or(0);

    // Visible row count: subtract 2 (borders) + 1 (header)
    let visible_rows = (area.height as usize).saturating_sub(3);

    let header = Row::new(vec![
        Cell::from("Team"),
        Cell::from("Budget"),
        Cell::from("Filled"),
        Cell::from("Remaining"),
    ])
    .style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(0);

    let total = state.team_summaries.len();

    let rows: Vec<Row> = if state.team_summaries.is_empty() {
        vec![Row::new(vec![Cell::from("  No team data available")])]
    } else {
        // Clamp scroll offset
        let max_offset = total.saturating_sub(visible_rows);
        let scroll_offset = scroll_offset.min(max_offset);

        state
            .team_summaries
            .iter()
            .skip(scroll_offset)
            .take(visible_rows.max(1))
            .map(|team| {
                let remaining_slots = team.total_slots.saturating_sub(team.slots_filled);
                Row::new(vec![
                    Cell::from(team.name.clone()),
                    Cell::from(format_budget(team.budget_remaining)),
                    Cell::from(format!("{}/{}", team.slots_filled, team.total_slots)),
                    Cell::from(format!("{}", remaining_slots)),
                ])
            })
            .collect()
    };

    let widths = [
        ratatui::layout::Constraint::Min(16),
        ratatui::layout::Constraint::Length(8),
        ratatui::layout::Constraint::Length(8),
        ratatui::layout::Constraint::Length(10),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Teams"),
    );
    frame.render_widget(table, area);

    // Render vertical scrollbar if content overflows
    if total > visible_rows {
        // Clamp scroll offset again for scrollbar state
        let max_offset = total.saturating_sub(visible_rows);
        let clamped_offset = scroll_offset.min(max_offset);
        let mut scrollbar_state = ScrollbarState::new(max_offset).position(clamped_offset);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }
}

/// Format a budget value for display.
pub fn format_budget(remaining: u32) -> String {
    format!("${}", remaining)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::TeamSummary;

    #[test]
    fn format_budget_basic() {
        assert_eq!(format_budget(260), "$260");
        assert_eq!(format_budget(0), "$0");
        assert_eq!(format_budget(135), "$135");
    }

    #[test]
    fn render_does_not_panic_empty() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_teams() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.team_summaries = vec![
            TeamSummary {
                name: "Team Alpha".to_string(),
                budget_remaining: 200,
                slots_filled: 5,
                total_slots: 26,
            },
            TeamSummary {
                name: "Team Beta".to_string(),
                budget_remaining: 180,
                slots_filled: 8,
                total_slots: 26,
            },
        ];
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
