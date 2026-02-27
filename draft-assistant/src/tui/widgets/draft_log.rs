// Draft log widget: chronological list of completed picks.
//
// Reverse chronological list.
// Each: "#{pick} {team}: {player} ({pos}) -- ${price}"
// Color: green if price < value (bargain), red if price > value (overpay), white if close

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::draft::pick::DraftPick;
use crate::tui::ViewState;
use crate::valuation::zscore::PlayerValuation;

/// Render the draft log into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState) {
    if state.draft_log.is_empty() {
        let paragraph = Paragraph::new("  No picks yet.")
            .style(Style::default().fg(Color::DarkGray))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Draft Log"),
            );
        frame.render_widget(paragraph, area);
        return;
    }

    // Build list items in reverse chronological order
    let items: Vec<ListItem> = state
        .draft_log
        .iter()
        .rev()
        .map(|pick| {
            let value = find_player_value(&state.available_players, &pick.player_name);
            let color = pick_color(pick.price, value);
            let text = format_pick(pick);
            ListItem::new(Line::from(Span::styled(text, Style::default().fg(color))))
        })
        .collect();

    let title = format!("Draft Log ({})", state.draft_log.len());

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title),
    );
    frame.render_widget(list, area);
}

/// Format a single draft pick for display.
pub fn format_pick(pick: &DraftPick) -> String {
    format!(
        "#{} {}: {} ({}) -- ${}",
        pick.pick_number, pick.team_name, pick.player_name, pick.position, pick.price
    )
}

/// Determine the color for a pick based on price vs value.
///
/// Green if price < 90% of value (bargain).
/// Red if price > 110% of value (overpay).
/// White otherwise (fair price).
pub fn pick_color(price: u32, value: Option<f64>) -> Color {
    match value {
        Some(val) if val > 0.0 => {
            let ratio = price as f64 / val;
            if ratio < 0.9 {
                Color::Green
            } else if ratio > 1.1 {
                Color::Red
            } else {
                Color::White
            }
        }
        _ => Color::White,
    }
}

/// Look up a player's dollar value from the available players list.
fn find_player_value(players: &[PlayerValuation], name: &str) -> Option<f64> {
    players
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.dollar_value)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_pick_basic() {
        let pick = DraftPick {
            pick_number: 42,
            team_id: "team_1".to_string(),
            team_name: "Vorticists".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
        };
        assert_eq!(
            format_pick(&pick),
            "#42 Vorticists: Mike Trout (CF) -- $45"
        );
    }

    #[test]
    fn pick_color_bargain() {
        // Price 20, value 30 -> ratio 0.67 -> green (bargain)
        assert_eq!(pick_color(20, Some(30.0)), Color::Green);
    }

    #[test]
    fn pick_color_overpay() {
        // Price 40, value 30 -> ratio 1.33 -> red (overpay)
        assert_eq!(pick_color(40, Some(30.0)), Color::Red);
    }

    #[test]
    fn pick_color_fair() {
        // Price 30, value 30 -> ratio 1.0 -> white (fair)
        assert_eq!(pick_color(30, Some(30.0)), Color::White);
    }

    #[test]
    fn pick_color_no_value() {
        assert_eq!(pick_color(30, None), Color::White);
    }

    #[test]
    fn pick_color_zero_value() {
        assert_eq!(pick_color(30, Some(0.0)), Color::White);
    }

    #[test]
    fn pick_color_edge_cases() {
        // Exactly at 90% boundary -> white (not bargain)
        assert_eq!(pick_color(27, Some(30.0)), Color::White);
        // Just below 90% boundary -> green (bargain)
        assert_eq!(pick_color(26, Some(30.0)), Color::Green);
        // Just above 110% boundary -> red (overpay)
        assert_eq!(pick_color(34, Some(30.0)), Color::Red);
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
    fn render_does_not_panic_with_picks() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.draft_log = vec![
            DraftPick {
                pick_number: 1,
                team_id: "team_1".to_string(),
                team_name: "Team A".to_string(),
                player_name: "Player 1".to_string(),
                position: "SP".to_string(),
                price: 30,
                espn_player_id: None,
            },
            DraftPick {
                pick_number: 2,
                team_id: "team_2".to_string(),
                team_name: "Team B".to_string(),
                player_name: "Player 2".to_string(),
                position: "C".to_string(),
                price: 15,
                espn_player_id: None,
            },
        ];
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
