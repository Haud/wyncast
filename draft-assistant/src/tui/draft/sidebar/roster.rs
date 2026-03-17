// Roster sidebar component: displays my team's current roster with slot assignments.
//
// Position slots with filled/empty status.
// "C: [empty]" or "1B: Pete Alonso ($28)"
// Highlight positions matching nominated player.
// Scrollable via Tab-focus and arrow keys.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::draft::pick::Position;
use crate::draft::roster::RosterSlot;
use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::widgets::focused_border_style;

/// Messages handled by the RosterPanel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterMessage {
    Scroll(ScrollDirection),
}

/// Stateful roster panel component.
pub struct RosterPanel {
    scroll: ScrollState,
}

impl RosterPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    pub fn update(&mut self, msg: RosterMessage) -> Option<Action> {
        match msg {
            RosterMessage::Scroll(dir) => {
                // Use a default page size for PageUp/PageDown when viewport
                // dimensions aren't available at update time. The actual
                // offset is clamped in view() via clamped_offset().
                self.scroll.scroll(dir, Self::DEFAULT_PAGE_SIZE);
                None
            }
        }
    }

    /// Default page size for PageUp/PageDown scrolling.
    const DEFAULT_PAGE_SIZE: usize = 20;

    /// Convert a key event to a RosterMessage.
    pub fn key_to_message(&self, key: KeyEvent) -> Option<RosterMessage> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                Some(RosterMessage::Scroll(ScrollDirection::Up))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                Some(RosterMessage::Scroll(ScrollDirection::Down))
            }
            KeyCode::PageUp => Some(RosterMessage::Scroll(ScrollDirection::PageUp)),
            KeyCode::PageDown => Some(RosterMessage::Scroll(ScrollDirection::PageDown)),
            KeyCode::Home => Some(RosterMessage::Scroll(ScrollDirection::Top)),
            KeyCode::End => Some(RosterMessage::Scroll(ScrollDirection::Bottom)),
            _ => None,
        }
    }

    /// Render the roster panel.
    ///
    /// `nominated_position`: highlight slots matching this position (from current nomination).
    pub fn view(
        &self,
        frame: &mut Frame,
        area: Rect,
        roster: &[RosterSlot],
        nominated_position: Option<&Position>,
        focused: bool,
    ) {
        let border = focused_border_style(focused, Style::default());

        if roster.is_empty() {
            let paragraph = Paragraph::new("  No roster data.")
                .style(Style::default().fg(Color::DarkGray))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border)
                        .title("My Roster"),
                );
            frame.render_widget(paragraph, area);
            return;
        }

        // Visible row count: subtract 2 for borders
        let visible_rows = (area.height as usize).saturating_sub(2);
        let total = roster.len();

        let scroll_offset = self.scroll.clamped_offset(total, visible_rows);

        let items: Vec<ListItem> = roster
            .iter()
            .skip(scroll_offset)
            .take(visible_rows.max(1))
            .map(|slot| {
                let is_highlight =
                    nominated_position.map_or(false, |pos| {
                        // Exact match
                        if slot.position == *pos {
                            return true;
                        }
                        // Combo-aware: if nominated position is concrete (e.g. LF),
                        // also highlight combo slots that accept it (e.g. OF).
                        if slot.position.is_combo_slot()
                            && slot.position.accepted_positions().contains(pos)
                        {
                            return true;
                        }
                        // Reverse: if nominated position is combo (e.g. OF),
                        // also highlight concrete slots it accepts (e.g. LF/CF/RF).
                        if pos.is_combo_slot()
                            && pos.accepted_positions().contains(&slot.position)
                        {
                            return true;
                        }
                        false
                    });
                format_slot(slot, is_highlight)
            })
            .collect();

        let filled = roster.iter().filter(|s| s.player.is_some()).count();
        let title = format!("My Roster ({}/{})", filled, total);

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border)
                .title(title),
        );
        frame.render_widget(list, area);

        // Render vertical scrollbar whenever content overflows
        if total > visible_rows {
            let mut scrollbar_state = ScrollbarState::new(total.saturating_sub(visible_rows))
                .position(scroll_offset);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area.inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }
    }
}

#[cfg(test)]
impl RosterPanel {
    /// Test-only accessor for scroll offset.
    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }
}

impl Default for RosterPanel {
    fn default() -> Self {
        Self::new()
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
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // -- Construction --

    #[test]
    fn new_starts_with_zero_scroll() {
        let panel = RosterPanel::new();
        assert_eq!(panel.scroll.offset(), 0);
    }

    #[test]
    fn default_starts_with_zero_scroll() {
        let panel = RosterPanel::default();
        assert_eq!(panel.scroll.offset(), 0);
    }

    // -- Update --

    #[test]
    fn update_scroll_down_changes_offset() {
        let mut panel = RosterPanel::new();
        let result = panel.update(RosterMessage::Scroll(ScrollDirection::Down));
        assert!(result.is_none());
        assert_eq!(panel.scroll.offset(), 1);
    }

    #[test]
    fn update_scroll_up_changes_offset() {
        let mut panel = RosterPanel::new();
        for _ in 0..5 {
            panel.update(RosterMessage::Scroll(ScrollDirection::Down));
        }
        let result = panel.update(RosterMessage::Scroll(ScrollDirection::Up));
        assert!(result.is_none());
        assert_eq!(panel.scroll.offset(), 4);
    }

    #[test]
    fn update_returns_none() {
        let mut panel = RosterPanel::new();
        assert!(panel
            .update(RosterMessage::Scroll(ScrollDirection::Down))
            .is_none());
    }

    // -- key_to_message --

    #[test]
    fn key_to_message_up_arrow() {
        let panel = RosterPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Up)),
            Some(RosterMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_down_arrow() {
        let panel = RosterPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Down)),
            Some(RosterMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_k() {
        let panel = RosterPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('k'))),
            Some(RosterMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_j() {
        let panel = RosterPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('j'))),
            Some(RosterMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_page_up() {
        let panel = RosterPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageUp)),
            Some(RosterMessage::Scroll(ScrollDirection::PageUp))
        );
    }

    #[test]
    fn key_to_message_page_down() {
        let panel = RosterPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageDown)),
            Some(RosterMessage::Scroll(ScrollDirection::PageDown))
        );
    }

    #[test]
    fn key_to_message_home() {
        let panel = RosterPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Home)),
            Some(RosterMessage::Scroll(ScrollDirection::Top))
        );
    }

    #[test]
    fn key_to_message_end() {
        let panel = RosterPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::End)),
            Some(RosterMessage::Scroll(ScrollDirection::Bottom))
        );
    }

    #[test]
    fn key_to_message_irrelevant_returns_none() {
        let panel = RosterPanel::new();
        assert_eq!(panel.key_to_message(key(KeyCode::Char('x'))), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Enter)), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Tab)), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Esc)), None);
    }

    // -- format_slot_text --

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

    // -- view() rendering --

    #[test]
    fn view_does_not_panic_empty() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = RosterPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_roster() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = RosterPanel::new();
        let roster = vec![
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
            .draw(|frame| panel.view(frame, frame.area(), &roster, None, false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = RosterPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, true))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_nomination_highlight() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = RosterPanel::new();
        let roster = vec![
            RosterSlot {
                position: Position::Catcher,
                player: None,
            },
            RosterSlot {
                position: Position::FirstBase,
                player: None,
            },
        ];
        let pos = Position::Catcher;
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &roster, Some(&pos), false))
            .unwrap();
    }
}
