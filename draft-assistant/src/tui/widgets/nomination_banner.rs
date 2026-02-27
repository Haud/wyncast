// Nomination banner widget: displays current player on the block.
//
// 4-row layout when nomination active:
// Line 1: "NOW UP: {player} ({pos}) -- nom. by {team}"
// Line 2: "Bid: ${bid} | Value: ${value} | Adj: ${adjusted}"
// When no nomination: "Waiting for next nomination..." in dim

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::protocol::{InstantAnalysis, InstantVerdict, NominationInfo};
use crate::tui::ViewState;

/// Render the nomination banner into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState) {
    if let Some(ref nom) = state.current_nomination {
        let lines = build_nomination_lines(nom, state.instant_analysis.as_ref());
        let paragraph = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Nomination")
                .border_style(Style::default().fg(Color::Yellow)),
        );
        frame.render_widget(paragraph, area);
    } else {
        let paragraph = Paragraph::new(Line::from(Span::styled(
            "  Waiting for next nomination...",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Nomination"),
        );
        frame.render_widget(paragraph, area);
    }
}

/// Build the content lines of the nomination banner.
fn build_nomination_lines<'a>(
    nom: &NominationInfo,
    analysis: Option<&InstantAnalysis>,
) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    // Line 1: NOW UP
    lines.push(Line::from(vec![
        Span::styled(
            " NOW UP: ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} ({})", nom.player_name, nom.position),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" -- nom. by {}", nom.nominated_by),
            Style::default().fg(Color::Gray),
        ),
    ]));

    // Line 2: Bid / Value / Adjusted
    if let Some(analysis) = analysis {
        let spans = vec![
            Span::styled(" Bid: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_dollar(nom.current_bid),
                Style::default().fg(Color::White),
            ),
            Span::styled(" | Value: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_dollar_f64(analysis.dollar_value),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" | Adj: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_dollar_f64(analysis.adjusted_value),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" | ", Style::default().fg(Color::Gray)),
            Span::styled(
                verdict_label(analysis.verdict).to_string(),
                Style::default()
                    .fg(verdict_color(analysis.verdict))
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        lines.push(Line::from(spans));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" Bid: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_dollar(nom.current_bid),
                Style::default().fg(Color::White),
            ),
        ]));
    }

    lines
}

/// Format a u32 dollar value as "$X".
pub fn format_dollar(value: u32) -> String {
    format!("${}", value)
}

/// Format an f64 dollar value as "$X".
pub fn format_dollar_f64(value: f64) -> String {
    format!("${:.0}", value)
}

/// Return the color for a verdict badge.
pub fn verdict_color(verdict: InstantVerdict) -> Color {
    match verdict {
        InstantVerdict::StrongTarget => Color::Green,
        InstantVerdict::ConditionalTarget => Color::Yellow,
        InstantVerdict::Pass => Color::DarkGray,
    }
}

/// Return the label for a verdict badge.
pub fn verdict_label(verdict: InstantVerdict) -> &'static str {
    match verdict {
        InstantVerdict::StrongTarget => "STRONG TARGET",
        InstantVerdict::ConditionalTarget => "CONDITIONAL",
        InstantVerdict::Pass => "PASS",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_dollar_basic() {
        assert_eq!(format_dollar(45), "$45");
        assert_eq!(format_dollar(0), "$0");
        assert_eq!(format_dollar(260), "$260");
    }

    #[test]
    fn format_dollar_f64_basic() {
        assert_eq!(format_dollar_f64(32.9), "$33");
        assert_eq!(format_dollar_f64(0.0), "$0");
        assert_eq!(format_dollar_f64(1.5), "$2");
    }

    #[test]
    fn verdict_colors() {
        assert_eq!(verdict_color(InstantVerdict::StrongTarget), Color::Green);
        assert_eq!(
            verdict_color(InstantVerdict::ConditionalTarget),
            Color::Yellow
        );
        assert_eq!(verdict_color(InstantVerdict::Pass), Color::DarkGray);
    }

    #[test]
    fn verdict_labels() {
        assert_eq!(verdict_label(InstantVerdict::StrongTarget), "STRONG TARGET");
        assert_eq!(
            verdict_label(InstantVerdict::ConditionalTarget),
            "CONDITIONAL"
        );
        assert_eq!(verdict_label(InstantVerdict::Pass), "PASS");
    }

    #[test]
    fn build_nomination_lines_without_analysis() {
        let nom = NominationInfo {
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            nominated_by: "Team Alpha".to_string(),
            current_bid: 45,
            current_bidder: Some("Team Beta".to_string()),
            time_remaining: Some(30),
            eligible_slots: vec![],
        };
        let lines = build_nomination_lines(&nom, None);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn build_nomination_lines_with_analysis() {
        let nom = NominationInfo {
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            nominated_by: "Team Alpha".to_string(),
            current_bid: 45,
            current_bidder: Some("Team Beta".to_string()),
            time_remaining: Some(30),
            eligible_slots: vec![],
        };
        let analysis = InstantAnalysis {
            player_name: "Mike Trout".to_string(),
            dollar_value: 42.0,
            adjusted_value: 45.5,
            verdict: InstantVerdict::StrongTarget,
        };
        let lines = build_nomination_lines(&nom, Some(&analysis));
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn render_does_not_panic_with_defaults() {
        let backend = ratatui::backend::TestBackend::new(80, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_nomination() {
        let backend = ratatui::backend::TestBackend::new(80, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.current_nomination = Some(NominationInfo {
            player_name: "Aaron Judge".to_string(),
            position: "OF".to_string(),
            nominated_by: "Team Beta".to_string(),
            current_bid: 55,
            current_bidder: None,
            time_remaining: None,
            eligible_slots: vec![],
        });
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
