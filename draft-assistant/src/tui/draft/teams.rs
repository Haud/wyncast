use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{
    Block, Borders, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};
use ratatui::Frame;

use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::widgets::focused_border_style;
use crate::tui::TeamSummary;

/// Messages handled by the TeamsPanel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamsMessage {
    Scroll(ScrollDirection),
}

/// Stateful teams overview panel component.
pub struct TeamsPanel {
    pub scroll: ScrollState,
}

impl TeamsPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    /// Update scroll viewport dimensions so PageUp/PageDown jump by the
    /// correct amount. Call this from the parent when the layout changes.
    pub fn set_viewport(&mut self, content_height: usize, viewport_height: usize) {
        self.scroll.set_viewport(content_height, viewport_height);
    }

    pub fn update(&mut self, msg: TeamsMessage) -> Option<Action> {
        match msg {
            TeamsMessage::Scroll(dir) => {
                self.scroll.scroll(dir);
                None
            }
        }
    }

    /// Convert a key event to a TeamsMessage.
    pub fn key_to_message(&self, key: KeyEvent) -> Option<TeamsMessage> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                Some(TeamsMessage::Scroll(ScrollDirection::Up))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                Some(TeamsMessage::Scroll(ScrollDirection::Down))
            }
            KeyCode::PageUp => Some(TeamsMessage::Scroll(ScrollDirection::PageUp)),
            KeyCode::PageDown => Some(TeamsMessage::Scroll(ScrollDirection::PageDown)),
            KeyCode::Home => Some(TeamsMessage::Scroll(ScrollDirection::Top)),
            KeyCode::End => Some(TeamsMessage::Scroll(ScrollDirection::Bottom)),
            _ => None,
        }
    }

    pub fn view(&self, frame: &mut Frame, area: Rect, teams: &[TeamSummary], focused: bool) {
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

        let total = teams.len();

        // Clamp scroll offset locally for rendering (scroll bounds are enforced
        // by ScrollState::scroll(); we just need a safe value here without
        // mutating self).
        let max_offset = total.saturating_sub(visible_rows);
        let scroll_offset = self.scroll.offset.min(max_offset);

        let rows: Vec<Row> = if teams.is_empty() {
            vec![Row::new(vec![Cell::from("  No team data available")])]
        } else {
            teams
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
            Constraint::Min(16),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(10),
        ];

        let focus_border = focused_border_style(focused, Style::default());

        let table = Table::new(rows, widths).header(header).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(focus_border)
                .title("Teams"),
        );
        frame.render_widget(table, area);

        // Render vertical scrollbar whenever content overflows
        if total > visible_rows {
            let mut scrollbar_state = ScrollbarState::new(max_offset).position(scroll_offset);
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

impl Default for TeamsPanel {
    fn default() -> Self {
        Self::new()
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
        let panel = TeamsPanel::new();
        assert_eq!(panel.scroll.offset, 0);
        assert_eq!(panel.scroll.content_height, 0);
        assert_eq!(panel.scroll.viewport_height, 0);
    }

    #[test]
    fn default_starts_with_zero_scroll() {
        let panel = TeamsPanel::default();
        assert_eq!(panel.scroll.offset, 0);
    }

    // -- Update --

    #[test]
    fn update_scroll_down_changes_offset() {
        let mut panel = TeamsPanel::new();
        panel.scroll.set_viewport(100, 10);
        let result = panel.update(TeamsMessage::Scroll(ScrollDirection::Down));
        assert!(result.is_none());
        assert_eq!(panel.scroll.offset, 1);
    }

    #[test]
    fn update_scroll_up_changes_offset() {
        let mut panel = TeamsPanel::new();
        panel.scroll.set_viewport(100, 10);
        panel.scroll.offset = 5;
        let result = panel.update(TeamsMessage::Scroll(ScrollDirection::Up));
        assert!(result.is_none());
        assert_eq!(panel.scroll.offset, 4);
    }

    #[test]
    fn update_returns_none() {
        let mut panel = TeamsPanel::new();
        panel.scroll.set_viewport(100, 10);
        assert!(panel
            .update(TeamsMessage::Scroll(ScrollDirection::Down))
            .is_none());
    }

    // -- key_to_message --

    #[test]
    fn key_to_message_up_arrow() {
        let panel = TeamsPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Up)),
            Some(TeamsMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_down_arrow() {
        let panel = TeamsPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Down)),
            Some(TeamsMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_k() {
        let panel = TeamsPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('k'))),
            Some(TeamsMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_j() {
        let panel = TeamsPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('j'))),
            Some(TeamsMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_page_up() {
        let panel = TeamsPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageUp)),
            Some(TeamsMessage::Scroll(ScrollDirection::PageUp))
        );
    }

    #[test]
    fn key_to_message_page_down() {
        let panel = TeamsPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageDown)),
            Some(TeamsMessage::Scroll(ScrollDirection::PageDown))
        );
    }

    #[test]
    fn key_to_message_home() {
        let panel = TeamsPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Home)),
            Some(TeamsMessage::Scroll(ScrollDirection::Top))
        );
    }

    #[test]
    fn key_to_message_end() {
        let panel = TeamsPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::End)),
            Some(TeamsMessage::Scroll(ScrollDirection::Bottom))
        );
    }

    #[test]
    fn key_to_message_irrelevant_returns_none() {
        let panel = TeamsPanel::new();
        assert_eq!(panel.key_to_message(key(KeyCode::Char('x'))), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Enter)), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Tab)), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Esc)), None);
    }

    // -- format_budget --

    #[test]
    fn format_budget_basic() {
        assert_eq!(format_budget(260), "$260");
        assert_eq!(format_budget(0), "$0");
        assert_eq!(format_budget(135), "$135");
    }

    // -- view() rendering --

    #[test]
    fn view_does_not_panic_empty() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = TeamsPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_teams() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = TeamsPanel::new();
        let teams = vec![
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
            .draw(|frame| panel.view(frame, frame.area(), &teams, false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = TeamsPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], true))
            .unwrap();
    }
}
