// Draft log widget: chronological list of completed picks.
//
// Reverse chronological list.
// Each: "#{pick} {team}: {player} ({pos}) -- ${price}"
// Color: green if price < value (bargain), red if price > value (overpay), white if close

use std::collections::HashMap;

use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::draft::pick::DraftPick;
use crate::tui::ViewState;
use crate::valuation::zscore::PlayerValuation;

/// Render the draft log into the given area.
///
/// When `focused` is true, the border is highlighted to indicate this panel
/// has keyboard focus for scroll routing.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState, focused: bool) {
    let focus_border = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    if state.draft_log.is_empty() {
        let paragraph = Paragraph::new("  No picks yet.")
            .style(Style::default().fg(Color::DarkGray))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(focus_border)
                    .title("Draft Log"),
            );
        frame.render_widget(paragraph, area);
        return;
    }

    // Build a name->value lookup map once to avoid O(n*m) scanning
    let value_map = build_value_map(&state.available_players);

    let scroll_offset = state.scroll_offset.get("draft_log").copied().unwrap_or(0);

    // Visible row count: subtract 2 for borders
    let visible_rows = (area.height as usize).saturating_sub(2);

    // All picks in reverse chronological order
    let all_picks: Vec<_> = state.draft_log.iter().rev().collect();
    let total = all_picks.len();

    // Clamp scroll offset
    let max_offset = total.saturating_sub(visible_rows);
    let scroll_offset = scroll_offset.min(max_offset);

    // Build list items for visible slice only
    let items: Vec<ListItem> = all_picks
        .into_iter()
        .skip(scroll_offset)
        .take(visible_rows.max(1))
        .map(|pick| {
            let value = value_map.get(pick.player_name.as_str()).copied();
            let color = pick_color(pick.price, value);
            let text = format_pick(pick);
            ListItem::new(Line::from(Span::styled(text, Style::default().fg(color))))
        })
        .collect();

    let title = format!("Draft Log ({})", state.draft_log.len());

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(focus_border)
            .title(title),
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

/// Build a HashMap of player name -> dollar value for O(1) lookups.
fn build_value_map(players: &[PlayerValuation]) -> HashMap<&str, f64> {
    players
        .iter()
        .map(|p| (p.name.as_str(), p.dollar_value))
        .collect()
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
            eligible_slots: vec![],
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
            .draw(|frame| render(frame, frame.area(), &state, false))
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
                eligible_slots: vec![],
            },
            DraftPick {
                pick_number: 2,
                team_id: "team_2".to_string(),
                team_name: "Team B".to_string(),
                player_name: "Player 2".to_string(),
                position: "C".to_string(),
                price: 15,
                espn_player_id: None,
                eligible_slots: vec![],
            },
        ];
        terminal
            .draw(|frame| render(frame, frame.area(), &state, false))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state, true))
            .unwrap();
    }
}
