// Matchup status bar: matchup period, team names, day counter, navigation hint.
//
// Single row display:
// | Matchup 1 (Mar 25 - Apr 5)  |  Bob Dole Exp. vs Certified!  |  Day 2 of 12  |  <- -> Days |

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::matchup::MatchupInfo;

/// Render the matchup status bar into the given 1-row area.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    matchup_info: Option<&MatchupInfo>,
    selected_day: usize,
    total_days: usize,
) {
    let line = match matchup_info {
        Some(info) => build_info_line(info, selected_day, total_days),
        None => build_waiting_line(),
    };

    let paragraph = Paragraph::new(line).style(
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray),
    );
    frame.render_widget(paragraph, area);
}

/// Build the status line when matchup data is available.
fn build_info_line(info: &MatchupInfo, selected_day: usize, total_days: usize) -> Line<'static> {
    let sep = Style::default().fg(Color::Gray);
    let normal = Style::default().fg(Color::White);
    let hint = Style::default().fg(Color::DarkGray);

    let day_display = selected_day + 1;

    let spans = vec![
        Span::styled(
            format!(" Matchup {} ({} - {})", info.matchup_period, info.start_date, info.end_date),
            normal,
        ),
        Span::styled("  \u{2502}  ", sep),
        Span::styled(
            format!("{} vs {}", info.my_team_name, info.opp_team_name),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  \u{2502}  ", sep),
        Span::styled(
            format!("Day {} of {}", day_display, total_days),
            normal,
        ),
        Span::styled("  \u{2502}  ", sep),
        Span::styled("\u{2190} \u{2192} Days", hint),
    ];

    Line::from(spans)
}

/// Build the status line when waiting for data.
fn build_waiting_line() -> Line<'static> {
    Line::from(Span::styled(
        " Matchup (waiting for data...)",
        Style::default().fg(Color::DarkGray),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matchup::TeamRecord;

    fn make_info() -> MatchupInfo {
        MatchupInfo {
            matchup_period: 1,
            start_date: "Mar 25".to_string(),
            end_date: "Apr 5".to_string(),
            my_team_name: "Bob Dole Exp.".to_string(),
            opp_team_name: "Certified!".to_string(),
            my_record: TeamRecord { wins: 1, losses: 0, ties: 0 },
            opp_record: TeamRecord { wins: 0, losses: 1, ties: 0 },
        }
    }

    #[test]
    fn render_does_not_panic_with_info() {
        let backend = ratatui::backend::TestBackend::new(120, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let info = make_info();
        terminal
            .draw(|frame| render(frame, frame.area(), Some(&info), 1, 12))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_without_info() {
        let backend = ratatui::backend::TestBackend::new(120, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), None, 0, 0))
            .unwrap();
    }

    #[test]
    fn info_line_contains_team_names() {
        let info = make_info();
        let line = build_info_line(&info, 1, 12);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Bob Dole Exp."));
        assert!(text.contains("Certified!"));
    }

    #[test]
    fn info_line_contains_day_counter() {
        let info = make_info();
        let line = build_info_line(&info, 1, 12);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Day 2 of 12"));
    }

    #[test]
    fn info_line_contains_matchup_period() {
        let info = make_info();
        let line = build_info_line(&info, 0, 12);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Matchup 1"));
    }

    #[test]
    fn info_line_contains_navigation_hint() {
        let info = make_info();
        let line = build_info_line(&info, 0, 12);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Days"));
    }

    #[test]
    fn waiting_line_shows_placeholder() {
        let line = build_waiting_line();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("waiting for data"));
    }
}
