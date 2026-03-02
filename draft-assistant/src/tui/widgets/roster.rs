// Roster widget: displays my team's current roster with slot assignments.
//
// Position slots with filled/empty status.
// "C: [empty]" or "1B: Pete Alonso ($28)"
// Highlight positions matching nominated player.
// Scrollable with [ and ] keys (sidebar scroll).

use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::draft::pick::Position;
use crate::draft::roster::RosterSlot;
use crate::tui::ViewState;

/// Render the roster sidebar into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState) {
    if state.my_roster.is_empty() {
        let paragraph = Paragraph::new("  No roster data.")
            .style(Style::default().fg(Color::DarkGray))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("My Roster"),
            );
        frame.render_widget(paragraph, area);
        return;
    }

    // Determine the nominated player's position for highlighting
    let nominated_position = state
        .current_nomination
        .as_ref()
        .and_then(|n| Position::from_str_pos(&n.position));

    let scroll_offset = state.scroll_offset.get("sidebar").copied().unwrap_or(0);

    // Visible row count: subtract 2 for borders
    let visible_rows = (area.height as usize).saturating_sub(2);
    let total = state.my_roster.len();

    // Clamp scroll offset
    let max_offset = total.saturating_sub(visible_rows);
    let scroll_offset = scroll_offset.min(max_offset);

    let items: Vec<ListItem> = state
        .my_roster
        .iter()
        .skip(scroll_offset)
        .take(visible_rows.max(1))
        .map(|slot| {
            let is_highlight =
                nominated_position.map_or(false, |pos| slot.position == pos);
            format_slot(slot, is_highlight)
        })
        .collect();

    let filled = state.my_roster.iter().filter(|s| s.player.is_some()).count();
    let title = format!("My Roster ({}/{})", filled, total);

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
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

/// Format a single roster slot as a ListItem.
fn format_slot<'a>(slot: &RosterSlot, highlight: bool) -> ListItem<'a> {
    let pos_label = slot.position.display_str();

    let (content, style) = if let Some(ref player) = slot.player {
        let text = format!(" {}: {} (${})", pos_label, player.name, player.price);
        let style = if highlight {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        (text, style)
    } else {
        let text = format!(" {}: [empty]", pos_label);
        let style = if highlight {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        (text, style)
    };

    ListItem::new(Line::from(Span::styled(content, style)))
}

/// Format a roster slot as a plain string (for testing).
pub fn format_slot_text(slot: &RosterSlot) -> String {
    let pos_label = slot.position.display_str();
    if let Some(ref player) = slot.player {
        format!("{}: {} (${})", pos_label, player.name, player.price)
    } else {
        format!("{}: [empty]", pos_label)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::roster::RosteredPlayer;

    #[test]
    fn format_slot_text_empty() {
        let slot = RosterSlot {
            position: Position::Catcher,
            player: None,
        };
        assert_eq!(format_slot_text(&slot), "C: [empty]");
    }

    #[test]
    fn format_slot_text_filled() {
        let slot = RosterSlot {
            position: Position::FirstBase,
            player: Some(RosteredPlayer {
                name: "Pete Alonso".to_string(),
                price: 28,
                position: Position::FirstBase,
                eligible_slots: vec![],
                espn_player_id: None,
            }),
        };
        assert_eq!(format_slot_text(&slot), "1B: Pete Alonso ($28)");
    }

    #[test]
    fn format_slot_text_pitcher() {
        let slot = RosterSlot {
            position: Position::StartingPitcher,
            player: Some(RosteredPlayer {
                name: "Gerrit Cole".to_string(),
                price: 35,
                position: Position::StartingPitcher,
                eligible_slots: vec![],
                espn_player_id: None,
            }),
        };
        assert_eq!(format_slot_text(&slot), "SP: Gerrit Cole ($35)");
    }

    #[test]
    fn render_does_not_panic_empty() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_roster() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.my_roster = vec![
            RosterSlot {
                position: Position::Catcher,
                player: Some(RosteredPlayer {
                    name: "Salvador Perez".to_string(),
                    price: 12,
                    position: Position::Catcher,
                    eligible_slots: vec![],
                    espn_player_id: None,
                }),
            },
            RosterSlot {
                position: Position::FirstBase,
                player: None,
            },
            RosterSlot {
                position: Position::SecondBase,
                player: None,
            },
        ];
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
