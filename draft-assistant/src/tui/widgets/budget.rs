// Budget widget: remaining budget, inflation factor, and spending pace.
//
// Key-value display:
// Spent, Remaining, Inflation, Max bid, Avg/slot
// Inflation > 1.0 = green (others overspending), < 1.0 = red

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::{BudgetStatus, ViewState};

/// Render the budget display into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState) {
    let lines = build_budget_lines(&state.budget);

    let scroll_offset = state.scroll_offset.get("sidebar").copied().unwrap_or(0);
    let total_lines = lines.len();
    let visible_rows = (area.height as usize).saturating_sub(2);
    let max_offset = total_lines.saturating_sub(visible_rows);
    let scroll = (scroll_offset.min(max_offset)) as u16;

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Budget"),
        )
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

/// Build the budget display lines.
fn build_budget_lines(budget: &BudgetStatus) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Spent
    lines.push(Line::from(vec![
        Span::styled(" Spent:     ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("${}", budget.spent),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!(" / ${}", budget.cap),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // Remaining
    lines.push(Line::from(vec![
        Span::styled(" Remaining: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("${}", budget.remaining),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Inflation
    let inflation_color = inflation_color(budget.inflation_rate);
    lines.push(Line::from(vec![
        Span::styled(" Inflation: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format_inflation(budget.inflation_rate),
            Style::default()
                .fg(inflation_color)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Max bid
    lines.push(Line::from(vec![
        Span::styled(" Max Bid:   ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("${}", budget.max_bid),
            Style::default().fg(Color::White),
        ),
    ]));

    // Avg per slot
    lines.push(Line::from(vec![
        Span::styled(" Avg/Slot:  ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("${:.1}", budget.avg_per_slot),
            Style::default().fg(Color::White),
        ),
    ]));

    lines
}

/// Return the color for the inflation rate.
///
/// Green if > 1.0 (others overspending, our values go up).
/// Red if < 1.0 (others underspending).
/// White at exactly 1.0.
pub fn inflation_color(rate: f64) -> Color {
    if rate > 1.0 {
        Color::Green
    } else if rate < 1.0 {
        Color::Red
    } else {
        Color::White
    }
}

/// Format the inflation rate for display.
pub fn format_inflation(rate: f64) -> String {
    format!("{:.3}x", rate)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflation_color_above_one() {
        assert_eq!(inflation_color(1.1), Color::Green);
        assert_eq!(inflation_color(1.5), Color::Green);
    }

    #[test]
    fn inflation_color_below_one() {
        assert_eq!(inflation_color(0.9), Color::Red);
        assert_eq!(inflation_color(0.5), Color::Red);
    }

    #[test]
    fn inflation_color_at_one() {
        assert_eq!(inflation_color(1.0), Color::White);
    }

    #[test]
    fn format_inflation_basic() {
        assert_eq!(format_inflation(1.0), "1.000x");
        assert_eq!(format_inflation(1.15), "1.150x");
        assert_eq!(format_inflation(0.85), "0.850x");
    }

    #[test]
    fn build_budget_lines_default() {
        let budget = BudgetStatus::default();
        let lines = build_budget_lines(&budget);
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn render_does_not_panic_with_defaults() {
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_data() {
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.budget = BudgetStatus {
            spent: 120,
            remaining: 140,
            cap: 260,
            inflation_rate: 1.15,
            max_bid: 115,
            avg_per_slot: 10.8,
        };
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
