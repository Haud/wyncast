use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::draft::pick::DraftPick;
use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::widgets::focused_border_style;
use crate::valuation::zscore::PlayerValuation;

/// Messages handled by the DraftLogPanel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftLogMessage {
    Scroll(ScrollDirection),
}

const PAGE_SIZE: usize = 20;

/// Stateful draft log panel component.
pub struct DraftLogPanel {
    scroll: ScrollState,
}

impl DraftLogPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    pub fn update(&mut self, msg: DraftLogMessage) -> Option<Action> {
        match msg {
            DraftLogMessage::Scroll(dir) => {
                self.scroll.scroll(dir, PAGE_SIZE);
                None
            }
        }
    }

    /// Convert a key event to a DraftLogMessage.
    pub fn key_to_message(&self, key: KeyEvent) -> Option<DraftLogMessage> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                Some(DraftLogMessage::Scroll(ScrollDirection::Up))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                Some(DraftLogMessage::Scroll(ScrollDirection::Down))
            }
            KeyCode::PageUp => Some(DraftLogMessage::Scroll(ScrollDirection::PageUp)),
            KeyCode::PageDown => Some(DraftLogMessage::Scroll(ScrollDirection::PageDown)),
            KeyCode::Home => Some(DraftLogMessage::Scroll(ScrollDirection::Top)),
            KeyCode::End => Some(DraftLogMessage::Scroll(ScrollDirection::Bottom)),
            _ => None,
        }
    }

    pub fn view(&self, frame: &mut Frame, area: Rect, picks: &[DraftPick], available_players: &[PlayerValuation], focused: bool) {
        let focus_border = focused_border_style(focused, Style::default());

        if picks.is_empty() {
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

        let value_map = build_value_map(available_players);

        let visible_rows = (area.height as usize).saturating_sub(2);
        let all_picks: Vec<_> = picks.iter().rev().collect();
        let total = all_picks.len();

        let scroll_offset = self.scroll.clamped_offset(total, visible_rows);

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

        let title = format!("Draft Log ({})", picks.len());

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(focus_border)
                .title(title),
        );
        frame.render_widget(list, area);

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

impl Default for DraftLogPanel {
    fn default() -> Self {
        Self::new()
    }
}

// Keep these as public functions -- they're useful utilities
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
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_pick(number: u32, name: &str, position: &str, price: u32) -> DraftPick {
        DraftPick {
            pick_number: number,
            team_id: "team_1".to_string(),
            team_name: "Team A".to_string(),
            player_name: name.to_string(),
            position: position.to_string(),
            price,
            espn_player_id: None,
            eligible_slots: vec![],
            assigned_slot: None,
        }
    }

    // -- Construction --

    #[test]
    fn new_starts_with_zero_scroll() {
        let panel = DraftLogPanel::new();
        assert_eq!(panel.scroll.offset(), 0);
    }

    #[test]
    fn default_starts_with_zero_scroll() {
        let panel = DraftLogPanel::default();
        assert_eq!(panel.scroll.offset(), 0);
    }

    // -- Update --

    #[test]
    fn update_scroll_down_changes_offset() {
        let mut panel = DraftLogPanel::new();
        let result = panel.update(DraftLogMessage::Scroll(ScrollDirection::Down));
        assert!(result.is_none());
        assert_eq!(panel.scroll.offset(), 1);
    }

    #[test]
    fn update_scroll_up_changes_offset() {
        let mut panel = DraftLogPanel::new();
        // Scroll down a few times to set offset
        for _ in 0..5 {
            panel.update(DraftLogMessage::Scroll(ScrollDirection::Down));
        }
        let result = panel.update(DraftLogMessage::Scroll(ScrollDirection::Up));
        assert!(result.is_none());
        assert_eq!(panel.scroll.offset(), 4);
    }

    #[test]
    fn update_returns_none() {
        let mut panel = DraftLogPanel::new();
        assert!(panel.update(DraftLogMessage::Scroll(ScrollDirection::Down)).is_none());
    }

    // -- key_to_message --

    #[test]
    fn key_to_message_up_arrow() {
        let panel = DraftLogPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Up)),
            Some(DraftLogMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_down_arrow() {
        let panel = DraftLogPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Down)),
            Some(DraftLogMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_k() {
        let panel = DraftLogPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('k'))),
            Some(DraftLogMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_j() {
        let panel = DraftLogPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('j'))),
            Some(DraftLogMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_page_up() {
        let panel = DraftLogPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageUp)),
            Some(DraftLogMessage::Scroll(ScrollDirection::PageUp))
        );
    }

    #[test]
    fn key_to_message_page_down() {
        let panel = DraftLogPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageDown)),
            Some(DraftLogMessage::Scroll(ScrollDirection::PageDown))
        );
    }

    #[test]
    fn key_to_message_home() {
        let panel = DraftLogPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Home)),
            Some(DraftLogMessage::Scroll(ScrollDirection::Top))
        );
    }

    #[test]
    fn key_to_message_end() {
        let panel = DraftLogPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::End)),
            Some(DraftLogMessage::Scroll(ScrollDirection::Bottom))
        );
    }

    #[test]
    fn key_to_message_irrelevant_returns_none() {
        let panel = DraftLogPanel::new();
        assert_eq!(panel.key_to_message(key(KeyCode::Char('x'))), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Enter)), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Tab)), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Esc)), None);
    }

    // -- format_pick --

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
            assigned_slot: None,
        };
        assert_eq!(format_pick(&pick), "#42 Vorticists: Mike Trout (CF) -- $45");
    }

    // -- pick_color --

    #[test]
    fn pick_color_bargain() {
        assert_eq!(pick_color(20, Some(30.0)), Color::Green);
    }

    #[test]
    fn pick_color_overpay() {
        assert_eq!(pick_color(40, Some(30.0)), Color::Red);
    }

    #[test]
    fn pick_color_fair() {
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

    // -- view() rendering --

    #[test]
    fn view_does_not_panic_empty() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = DraftLogPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], &[], false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_picks() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = DraftLogPanel::new();
        let picks = vec![
            make_pick(1, "Player 1", "SP", 30),
            make_pick(2, "Player 2", "C", 15),
        ];
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &picks, &[], false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = DraftLogPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], &[], true))
            .unwrap();
    }
}
