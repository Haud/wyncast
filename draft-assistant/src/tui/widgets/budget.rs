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

use crate::tui::BudgetStatus;
use super::focused_border_style;

/// Render the budget display into the given area.
///
/// When `focused` is true, the border is highlighted in cyan to indicate this
/// panel has keyboard focus for scroll routing.
pub fn render(frame: &mut Frame, area: Rect, budget: &BudgetStatus, scroll_offset: usize, focused: bool) {
    let lines = build_budget_lines(budget);
    let total_lines = lines.len();
    let visible_rows = (area.height as usize).saturating_sub(2);
    let max_offset = total_lines.saturating_sub(visible_rows);
    let scroll = (scroll_offset.min(max_offset)) as u16;

    let border = focused_border_style(focused, Style::default());

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border)
                .title("Budget"),
        )
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

/// Build the budget display lines.
fn build_budget_lines(budget: &BudgetStatus) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Spent
    let mut spent_spans = vec![
        Span::styled(" Spent:     ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("${}", budget.spent),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!(" / ${}", budget.cap),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    // Append hitter/pitcher split if targets are non-zero (config is loaded)
    if budget.hitting_target > 0 {
        let hit_pct = (budget.hitting_spent as f64 / budget.hitting_target as f64 * 100.0).round() as u32;
        let pit_pct = if budget.pitching_target > 0 {
            (budget.pitching_spent as f64 / budget.pitching_target as f64 * 100.0).round() as u32
        } else {
            0
        };

        spent_spans.push(Span::styled("    ", Style::default()));
        spent_spans.push(Span::styled(
            format!("Hit ${}/{} ({}%)", budget.hitting_spent, budget.hitting_target, hit_pct),
            Style::default().fg(split_color(hit_pct)),
        ));
        spent_spans.push(Span::styled("  ", Style::default()));
        spent_spans.push(Span::styled(
            format!("Pit ${}/{} ({}%)", budget.pitching_spent, budget.pitching_target, pit_pct),
            Style::default().fg(split_color(pit_pct)),
        ));
    }

    lines.push(Line::from(spent_spans));

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

/// Return color for hitter/pitcher budget percentage.
/// Green < 80%, Yellow 80-100%, Red > 100%.
fn split_color(pct: u32) -> Color {
    if pct > 100 {
        Color::Red
    } else if pct >= 80 {
        Color::Yellow
    } else {
        Color::Green
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
        let budget = BudgetStatus::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &budget, 0, false))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_data() {
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let budget = BudgetStatus {
            spent: 120,
            remaining: 140,
            cap: 260,
            inflation_rate: 1.15,
            max_bid: 115,
            avg_per_slot: 10.8,
            hitting_spent: 0,
            hitting_target: 0,
            pitching_spent: 0,
            pitching_target: 0,
        };
        terminal
            .draw(|frame| render(frame, frame.area(), &budget, 0, false))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let budget = BudgetStatus::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &budget, 0, true))
            .unwrap();
    }

    #[test]
    fn split_color_below_80() {
        assert_eq!(split_color(0), Color::Green);
        assert_eq!(split_color(50), Color::Green);
        assert_eq!(split_color(79), Color::Green);
    }

    #[test]
    fn split_color_at_80_to_100() {
        assert_eq!(split_color(80), Color::Yellow);
        assert_eq!(split_color(90), Color::Yellow);
        assert_eq!(split_color(100), Color::Yellow);
    }

    #[test]
    fn split_color_above_100() {
        assert_eq!(split_color(101), Color::Red);
        assert_eq!(split_color(150), Color::Red);
    }

    #[test]
    fn build_budget_lines_with_split_targets_still_five_lines() {
        let budget = BudgetStatus {
            spent: 120,
            remaining: 140,
            cap: 260,
            inflation_rate: 1.0,
            max_bid: 115,
            avg_per_slot: 10.0,
            hitting_spent: 85,
            hitting_target: 169,
            pitching_spent: 35,
            pitching_target: 91,
        };
        let lines = build_budget_lines(&budget);
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn render_does_not_panic_with_budget_split() {
        let backend = ratatui::backend::TestBackend::new(80, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let budget = BudgetStatus {
            spent: 120,
            remaining: 140,
            cap: 260,
            inflation_rate: 1.15,
            max_bid: 115,
            avg_per_slot: 10.8,
            hitting_spent: 85,
            hitting_target: 169,
            pitching_spent: 35,
            pitching_target: 91,
        };
        terminal
            .draw(|frame| render(frame, frame.area(), &budget, 0, false))
            .unwrap();
    }
}
