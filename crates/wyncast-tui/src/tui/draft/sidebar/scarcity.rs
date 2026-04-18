// Scarcity sidebar component: positional scarcity index heat map.
//
// One row per position with visual gauge/bar.
// Color: Red=Critical, Yellow=High, Blue=Medium, Green=Low
// Mark nominated player's position.
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
use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::widgets::focused_border_style;
use crate::valuation::scarcity::{ScarcityEntry, ScarcityUrgency};

/// Messages handled by the ScarcityPanel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScarcityPanelMessage {
    Scroll(ScrollDirection),
}

/// Stateful scarcity panel component.
pub struct ScarcityPanel {
    scroll: ScrollState,
}

impl ScarcityPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    pub fn update(&mut self, msg: ScarcityPanelMessage) -> Option<Action> {
        match msg {
            ScarcityPanelMessage::Scroll(dir) => {
                self.scroll.scroll(dir, Self::DEFAULT_PAGE_SIZE);
                None
            }
        }
    }

    /// Default page size for PageUp/PageDown scrolling.
    const DEFAULT_PAGE_SIZE: usize = 20;

    /// Convert a key event to a ScarcityPanelMessage.
    pub fn key_to_message(&self, key: KeyEvent) -> Option<ScarcityPanelMessage> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                Some(ScarcityPanelMessage::Scroll(ScrollDirection::Up))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                Some(ScarcityPanelMessage::Scroll(ScrollDirection::Down))
            }
            KeyCode::PageUp => Some(ScarcityPanelMessage::Scroll(ScrollDirection::PageUp)),
            KeyCode::PageDown => Some(ScarcityPanelMessage::Scroll(ScrollDirection::PageDown)),
            KeyCode::Home => Some(ScarcityPanelMessage::Scroll(ScrollDirection::Top)),
            KeyCode::End => Some(ScarcityPanelMessage::Scroll(ScrollDirection::Bottom)),
            _ => None,
        }
    }

    /// Render the scarcity panel.
    ///
    /// `nominated_position`: highlight entries matching this position (from current nomination).
    pub fn view(
        &self,
        frame: &mut Frame,
        area: Rect,
        scarcity: &[ScarcityEntry],
        nominated_position: Option<&Position>,
        focused: bool,
    ) {
        let border = focused_border_style(focused, Style::default());

        if scarcity.is_empty() {
            let paragraph = Paragraph::new("  No scarcity data.")
                .style(Style::default().fg(Color::DarkGray))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border)
                        .title("Scarcity"),
                );
            frame.render_widget(paragraph, area);
            return;
        }

        // Visible row count: subtract 2 for borders
        let visible_rows = (area.height as usize).saturating_sub(2);
        let total = scarcity.len();

        let scroll_offset = self.scroll.clamped_offset(total, visible_rows);

        let items: Vec<ListItem> = scarcity
            .iter()
            .skip(scroll_offset)
            .take(visible_rows.max(1))
            .map(|entry| {
                let is_nominated =
                    nominated_position.is_some_and(|pos| {
                        if entry.position == *pos {
                            return true;
                        }
                        // Combo-aware: concrete nominated pos highlights combo entries
                        if entry.position.is_combo_slot()
                            && entry.position.accepted_positions().contains(pos)
                        {
                            return true;
                        }
                        // Reverse: combo nominated pos highlights concrete entries
                        if pos.is_combo_slot()
                            && pos.accepted_positions().contains(&entry.position)
                        {
                            return true;
                        }
                        false
                    });
                format_scarcity_entry(entry, is_nominated)
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border)
                .title("Scarcity"),
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
impl ScarcityPanel {
    /// Test-only accessor for scroll offset.
    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }
}

impl Default for ScarcityPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a scarcity entry as a ListItem with a visual gauge.
fn format_scarcity_entry<'a>(entry: &ScarcityEntry, is_nominated: bool) -> ListItem<'a> {
    let pos_label = entry.position.display_str();
    let urgency_label = format_urgency(entry.urgency);
    let color = urgency_color(entry.urgency);
    let bar = urgency_bar(entry.players_above_replacement);

    let marker = if is_nominated { ">" } else { " " };

    let spans = vec![
        Span::styled(
            format!("{}{:>3} ", marker, pos_label),
            if is_nominated {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        ),
        Span::styled(bar, Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            format!("{:>8}", urgency_label),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" ({})", entry.players_above_replacement),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    ListItem::new(Line::from(spans))
}

/// Return the color for a scarcity urgency level.
pub fn urgency_color(urgency: ScarcityUrgency) -> Color {
    match urgency {
        ScarcityUrgency::Critical => Color::Red,
        ScarcityUrgency::High => Color::Yellow,
        ScarcityUrgency::Medium => Color::Blue,
        ScarcityUrgency::Low => Color::Green,
    }
}

/// Return a visual bar based on player count (more players = longer bar).
pub fn urgency_bar(players_above_replacement: usize) -> String {
    let max_bar = 8;
    let filled = players_above_replacement.min(max_bar);
    let empty = max_bar - filled;
    format!("[{}{}]", "#".repeat(filled), "-".repeat(empty))
}

/// Return a human-readable urgency label.
pub fn format_urgency(urgency: ScarcityUrgency) -> &'static str {
    match urgency {
        ScarcityUrgency::Critical => "CRITICAL",
        ScarcityUrgency::High => "HIGH",
        ScarcityUrgency::Medium => "MEDIUM",
        ScarcityUrgency::Low => "LOW",
    }
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
        let panel = ScarcityPanel::new();
        assert_eq!(panel.scroll.offset(), 0);
    }

    #[test]
    fn default_starts_with_zero_scroll() {
        let panel = ScarcityPanel::default();
        assert_eq!(panel.scroll.offset(), 0);
    }

    // -- Update --

    #[test]
    fn update_scroll_down_changes_offset() {
        let mut panel = ScarcityPanel::new();
        let result = panel.update(ScarcityPanelMessage::Scroll(ScrollDirection::Down));
        assert!(result.is_none());
        assert_eq!(panel.scroll.offset(), 1);
    }

    #[test]
    fn update_scroll_up_changes_offset() {
        let mut panel = ScarcityPanel::new();
        for _ in 0..5 {
            panel.update(ScarcityPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = panel.update(ScarcityPanelMessage::Scroll(ScrollDirection::Up));
        assert!(result.is_none());
        assert_eq!(panel.scroll.offset(), 4);
    }

    #[test]
    fn update_returns_none() {
        let mut panel = ScarcityPanel::new();
        assert!(panel
            .update(ScarcityPanelMessage::Scroll(ScrollDirection::Down))
            .is_none());
    }

    // -- key_to_message --

    #[test]
    fn key_to_message_up_arrow() {
        let panel = ScarcityPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Up)),
            Some(ScarcityPanelMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_down_arrow() {
        let panel = ScarcityPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Down)),
            Some(ScarcityPanelMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_k() {
        let panel = ScarcityPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('k'))),
            Some(ScarcityPanelMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_j() {
        let panel = ScarcityPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('j'))),
            Some(ScarcityPanelMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_page_up() {
        let panel = ScarcityPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageUp)),
            Some(ScarcityPanelMessage::Scroll(ScrollDirection::PageUp))
        );
    }

    #[test]
    fn key_to_message_page_down() {
        let panel = ScarcityPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageDown)),
            Some(ScarcityPanelMessage::Scroll(ScrollDirection::PageDown))
        );
    }

    #[test]
    fn key_to_message_home() {
        let panel = ScarcityPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Home)),
            Some(ScarcityPanelMessage::Scroll(ScrollDirection::Top))
        );
    }

    #[test]
    fn key_to_message_end() {
        let panel = ScarcityPanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::End)),
            Some(ScarcityPanelMessage::Scroll(ScrollDirection::Bottom))
        );
    }

    #[test]
    fn key_to_message_irrelevant_returns_none() {
        let panel = ScarcityPanel::new();
        assert_eq!(panel.key_to_message(key(KeyCode::Char('x'))), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Enter)), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Tab)), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Esc)), None);
    }

    // -- Helper functions --

    #[test]
    fn urgency_color_values() {
        assert_eq!(urgency_color(ScarcityUrgency::Critical), Color::Red);
        assert_eq!(urgency_color(ScarcityUrgency::High), Color::Yellow);
        assert_eq!(urgency_color(ScarcityUrgency::Medium), Color::Blue);
        assert_eq!(urgency_color(ScarcityUrgency::Low), Color::Green);
    }

    #[test]
    fn urgency_bar_empty() {
        assert_eq!(urgency_bar(0), "[--------]");
    }

    #[test]
    fn urgency_bar_partial() {
        assert_eq!(urgency_bar(3), "[###-----]");
    }

    #[test]
    fn urgency_bar_full() {
        assert_eq!(urgency_bar(8), "[########]");
    }

    #[test]
    fn urgency_bar_overflow() {
        // More than max_bar should cap at max
        assert_eq!(urgency_bar(15), "[########]");
    }

    #[test]
    fn format_urgency_values() {
        assert_eq!(format_urgency(ScarcityUrgency::Critical), "CRITICAL");
        assert_eq!(format_urgency(ScarcityUrgency::High), "HIGH");
        assert_eq!(format_urgency(ScarcityUrgency::Medium), "MEDIUM");
        assert_eq!(format_urgency(ScarcityUrgency::Low), "LOW");
    }

    // -- view() rendering --

    #[test]
    fn view_does_not_panic_empty() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = ScarcityPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_data() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = ScarcityPanel::new();
        let data = vec![
            ScarcityEntry {
                position: Position::Catcher,
                players_above_replacement: 2,
                top_available_vor: 8.0,
                replacement_vor: 2.0,
                dropoff: 6.0,
                urgency: ScarcityUrgency::Critical,
            },
            ScarcityEntry {
                position: Position::FirstBase,
                players_above_replacement: 6,
                top_available_vor: 10.0,
                replacement_vor: 5.0,
                dropoff: 5.0,
                urgency: ScarcityUrgency::Medium,
            },
        ];
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &data, None, false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = ScarcityPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, true))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_nomination_highlight() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = ScarcityPanel::new();
        let data = vec![
            ScarcityEntry {
                position: Position::Catcher,
                players_above_replacement: 2,
                top_available_vor: 8.0,
                replacement_vor: 2.0,
                dropoff: 6.0,
                urgency: ScarcityUrgency::Critical,
            },
        ];
        let pos = Position::Catcher;
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &data, Some(&pos), false))
            .unwrap();
    }
}
