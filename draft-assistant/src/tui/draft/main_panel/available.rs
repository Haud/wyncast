// AvailablePanel component: filterable, scrollable table of undrafted players.
//
// Owns filter state (text filter, filter mode, position filter) and scroll
// state internally. The parent passes in the player data and nominated player
// name; the component handles filtering, rendering, and input routing.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};
use ratatui::Frame;

use crate::draft::pick::Position;
use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::subscription::{
    Subscription, SubscriptionId,
    keybinding::{
        exact, KeyBindingRecipe, KeybindHint, KeybindManager, KeyTrigger, PRIORITY_CAPTURE,
    },
};
use crate::tui::text_input::TextInput;
use crate::tui::widgets::focused_border_style;
use crate::valuation::zscore::PlayerValuation;

/// Page size for PageUp/PageDown scrolling (matches TUI input convention).
const PAGE_SIZE: usize = 20;

/// Messages that can be sent to the AvailablePanel component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvailablePanelMessage {
    Scroll(ScrollDirection),
    ToggleFilterMode,
    ExitFilterMode { clear: bool },
    FilterKeyPress(KeyEvent),
    SetPositionFilter(Option<Position>),
    ClearFilters,
}

/// AvailablePanel component: available players table with integrated filtering.
pub struct AvailablePanel {
    scroll: ScrollState,
    filter_text: TextInput,
    filter_mode: bool,
    position_filter: Option<Position>,
    sub_id: SubscriptionId,
}

impl AvailablePanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
            filter_text: TextInput::new(),
            filter_mode: false,
            position_filter: None,
            sub_id: SubscriptionId::unique(),
        }
    }

    /// Declare keybindings for the subscription system.
    ///
    /// When filter mode is active, returns a capturing
    /// `Subscription<AvailablePanelMessage>` at `PRIORITY_CAPTURE` that
    /// handles Esc (cancel), Enter (apply), and character input.
    /// When filter mode is inactive, returns `Subscription::none()`.
    pub fn subscription(&self, kb: &mut KeybindManager) -> Subscription<AvailablePanelMessage> {
        if !self.filter_mode {
            return Subscription::none();
        }

        let recipe = KeyBindingRecipe::new(self.sub_id)
            .priority(PRIORITY_CAPTURE)
            .capture()
            .bind(
                exact(KeyCode::Esc),
                |_| AvailablePanelMessage::ExitFilterMode { clear: true },
                KeybindHint::new("Esc", "Cancel filter"),
            )
            .bind(
                exact(KeyCode::Enter),
                |_| AvailablePanelMessage::ExitFilterMode { clear: false },
                KeybindHint::new("Enter", "Apply filter"),
            )
            .bind(
                KeyTrigger::AnyChar,
                |k| AvailablePanelMessage::FilterKeyPress(k),
                KeybindHint::new("a-z", "Type to filter"),
            );

        kb.subscribe(recipe)
    }

    pub fn update(&mut self, msg: AvailablePanelMessage) -> Option<Action> {
        match msg {
            AvailablePanelMessage::Scroll(dir) => {
                self.scroll.scroll(dir, PAGE_SIZE);
                None
            }
            AvailablePanelMessage::ToggleFilterMode => {
                self.filter_mode = true;
                None
            }
            AvailablePanelMessage::ExitFilterMode { clear } => {
                self.filter_mode = false;
                if clear {
                    self.filter_text.clear();
                }
                None
            }
            AvailablePanelMessage::FilterKeyPress(key) => {
                if let Some(msg) = TextInput::key_to_message(&key) {
                    self.filter_text.update(msg);
                }
                None
            }
            AvailablePanelMessage::SetPositionFilter(pos) => {
                self.position_filter = pos;
                None
            }
            AvailablePanelMessage::ClearFilters => {
                self.filter_text.clear();
                self.position_filter = None;
                self.scroll.reset();
                None
            }
        }
    }

    /// Map a key event to an AvailablePanelMessage, if applicable.
    pub fn key_to_message(&self, key: KeyEvent) -> Option<AvailablePanelMessage> {
        if self.filter_mode {
            match key.code {
                KeyCode::Esc => Some(AvailablePanelMessage::ExitFilterMode { clear: true }),
                KeyCode::Enter => Some(AvailablePanelMessage::ExitFilterMode { clear: false }),
                _ => Some(AvailablePanelMessage::FilterKeyPress(key)),
            }
        } else {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    Some(AvailablePanelMessage::Scroll(ScrollDirection::Up))
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    Some(AvailablePanelMessage::Scroll(ScrollDirection::Down))
                }
                KeyCode::PageUp => Some(AvailablePanelMessage::Scroll(ScrollDirection::PageUp)),
                KeyCode::PageDown => {
                    Some(AvailablePanelMessage::Scroll(ScrollDirection::PageDown))
                }
                KeyCode::Home => Some(AvailablePanelMessage::Scroll(ScrollDirection::Top)),
                KeyCode::End => Some(AvailablePanelMessage::Scroll(ScrollDirection::Bottom)),
                _ => None,
            }
        }
    }

    /// Whether filter mode is currently active.
    pub fn filter_mode(&self) -> bool {
        self.filter_mode
    }

    /// Current position filter.
    pub fn position_filter(&self) -> Option<Position> {
        self.position_filter
    }

    /// Current filter text value.
    pub fn filter_text(&self) -> &TextInput {
        &self.filter_text
    }

    /// Raw scroll offset (for testing/inspection).
    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Render the available players table into the given area.
    pub fn view(
        &self,
        frame: &mut Frame,
        area: Rect,
        players: &[PlayerValuation],
        nominated_name: Option<&str>,
        focused: bool,
    ) {
        let filtered = filter_players(
            players,
            self.position_filter.as_ref(),
            self.filter_text.value(),
        );

        // Visible row count: subtract 2 (borders) + 1 (header) = 3
        let visible_rows = (area.height as usize).saturating_sub(3);

        // Use ScrollState's clamped offset for safe rendering
        let scroll_offset = self.scroll.clamped_offset(filtered.len(), visible_rows);

        let header = Row::new(vec![
            Cell::from("#"),
            Cell::from("Name"),
            Cell::from("Pos"),
            Cell::from("$Val"),
            Cell::from("VOR"),
            Cell::from("zTotal"),
        ])
        .style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(0);

        // Only render the visible slice of rows
        let visible_filtered: Vec<_> = filtered
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_rows.max(1))
            .collect();

        let rows: Vec<Row> = visible_filtered
            .iter()
            .map(|(i, p)| {
                let is_nominated = nominated_name.map_or(false, |name| name == p.name);
                let style = if is_nominated {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                Row::new(vec![
                    Cell::from(format!("{}", i + 1)),
                    Cell::from(p.name.clone()),
                    Cell::from(format_positions(&p.positions)),
                    Cell::from(format!("${:.0}", p.dollar_value)),
                    Cell::from(format!("{:.1}", p.vor)),
                    Cell::from(format!("{:.2}", p.total_zscore)),
                ])
                .style(style)
            })
            .collect();

        let title = self.build_title(filtered.len());

        let widths = [
            ratatui::layout::Constraint::Length(4),
            ratatui::layout::Constraint::Min(16),
            ratatui::layout::Constraint::Length(8),
            ratatui::layout::Constraint::Length(6),
            ratatui::layout::Constraint::Length(6),
            ratatui::layout::Constraint::Length(7),
        ];

        // Border style priority: filter mode > focus > default.
        let block = if self.filter_mode {
            Block::default()
                .borders(Borders::ALL)
                .border_style(focused_border_style(true, Style::default()))
                .title(title)
                .title_bottom(Line::from(vec![Span::styled(
                    " [FILTER MODE] ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )]))
        } else {
            Block::default()
                .borders(Borders::ALL)
                .border_style(focused_border_style(focused, Style::default()))
                .title(title)
        };

        let table = Table::new(rows, widths)
            .header(header)
            .block(block)
            .row_highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol(">> ");

        frame.render_widget(table, area);

        // Render vertical scrollbar whenever content overflows
        if filtered.len() > visible_rows {
            let mut scrollbar_state =
                ScrollbarState::new(filtered.len().saturating_sub(visible_rows))
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

    /// Build the title with filter info and pre-computed count.
    fn build_title(&self, filtered_count: usize) -> Line<'static> {
        let mut title = String::from("Available Players");
        if let Some(ref pos) = self.position_filter {
            title.push_str(&format!(" [{}]", pos.display_str()));
        }
        if !self.filter_text.is_empty() {
            title.push_str(&format!(" \"{}\"", self.filter_text.value()));
        }
        title.push_str(&format!(" ({})", filtered_count));
        Line::from(title)
    }
}

impl Default for AvailablePanel {
    fn default() -> Self {
        Self::new()
    }
}


/// Filter players by position and text search.
pub fn filter_players<'a>(
    players: &'a [PlayerValuation],
    position_filter: Option<&Position>,
    filter_text: &str,
) -> Vec<&'a PlayerValuation> {
    let text_lower = filter_text.to_lowercase();

    players
        .iter()
        .filter(|p| {
            // Position filter
            if let Some(pos) = position_filter {
                if !p.positions.contains(pos) {
                    return false;
                }
            }
            // Text filter (match on name)
            if !text_lower.is_empty() && !p.name.to_lowercase().contains(&text_lower) {
                return false;
            }
            true
        })
        .collect()
}

/// Format position list as a compact string (e.g., "1B/OF").
pub fn format_positions(positions: &[Position]) -> String {
    if positions.is_empty() {
        return "--".to_string();
    }
    positions
        .iter()
        .map(|p| p.display_str())
        .collect::<Vec<_>>()
        .join("/")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::valuation::zscore::{CategoryZScores, HitterZScores, PlayerProjectionData};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_test_player(name: &str, positions: Vec<Position>, dollar: f64) -> PlayerValuation {
        PlayerValuation {
            name: name.to_string(),
            team: "TST".to_string(),
            positions,
            is_pitcher: false,
            is_two_way: false,
            pitcher_type: None,
            projection: PlayerProjectionData::Hitter {
                pa: 600,
                ab: 550,
                h: 150,
                hr: 25,
                r: 80,
                rbi: 85,
                bb: 50,
                sb: 10,
                avg: 0.273,
            },
            total_zscore: 3.5,
            category_zscores: CategoryZScores::Hitter(HitterZScores {
                r: 0.5,
                hr: 0.3,
                rbi: 0.4,
                bb: 0.6,
                sb: 0.2,
                avg: 0.1,
                total: 3.5,
            }),
            vor: 5.0,
            best_position: None,
            dollar_value: dollar,
        }
    }

    // -- Construction --

    #[test]
    fn new_starts_empty() {
        let panel = AvailablePanel::new();
        assert!(!panel.filter_mode());
        assert!(panel.filter_text().is_empty());
        assert!(panel.position_filter().is_none());
        assert_eq!(panel.scroll_offset(), 0);
    }

    #[test]
    fn default_starts_empty() {
        let panel = AvailablePanel::default();
        assert!(!panel.filter_mode());
        assert!(panel.filter_text().is_empty());
        assert!(panel.position_filter().is_none());
    }

    // -- Update: ToggleFilterMode --

    #[test]
    fn toggle_filter_mode_activates() {
        let mut panel = AvailablePanel::new();
        let result = panel.update(AvailablePanelMessage::ToggleFilterMode);
        assert_eq!(result, None);
        assert!(panel.filter_mode());
    }

    // -- Update: ExitFilterMode --

    #[test]
    fn exit_filter_mode_clear_true_clears_text() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::ToggleFilterMode);
        panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char('a'))));
        panel.update(AvailablePanelMessage::ExitFilterMode { clear: true });
        assert!(!panel.filter_mode());
        assert!(panel.filter_text().is_empty());
    }

    #[test]
    fn exit_filter_mode_clear_false_keeps_text() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::ToggleFilterMode);
        panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char('a'))));
        panel.update(AvailablePanelMessage::ExitFilterMode { clear: false });
        assert!(!panel.filter_mode());
        assert_eq!(panel.filter_text().value(), "a");
    }

    // -- Update: FilterKeyPress --

    #[test]
    fn filter_key_press_appends_chars() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char('t'))));
        panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char('r'))));
        assert_eq!(panel.filter_text().value(), "tr");
    }

    #[test]
    fn filter_key_press_backspace() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char('a'))));
        panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char('b'))));
        panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Backspace)));
        assert_eq!(panel.filter_text().value(), "a");
    }

    // -- Update: SetPositionFilter --

    #[test]
    fn set_position_filter() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::SetPositionFilter(Some(
            Position::Catcher,
        )));
        assert_eq!(panel.position_filter(), Some(Position::Catcher));
    }

    #[test]
    fn set_position_filter_none_clears() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::SetPositionFilter(Some(
            Position::Catcher,
        )));
        panel.update(AvailablePanelMessage::SetPositionFilter(None));
        assert!(panel.position_filter().is_none());
    }

    // -- Update: ClearFilters --

    #[test]
    fn clear_filters_resets_all() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char('x'))));
        panel.update(AvailablePanelMessage::SetPositionFilter(Some(
            Position::Catcher,
        )));
        panel.update(AvailablePanelMessage::Scroll(ScrollDirection::Down));
        panel.update(AvailablePanelMessage::ClearFilters);
        assert!(panel.filter_text().is_empty());
        assert!(panel.position_filter().is_none());
        assert_eq!(panel.scroll_offset(), 0);
    }

    // -- Update: Scroll --

    #[test]
    fn scroll_down() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::Scroll(ScrollDirection::Down));
        assert_eq!(panel.scroll_offset(), 1);
    }

    #[test]
    fn scroll_up_at_zero() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::Scroll(ScrollDirection::Up));
        assert_eq!(panel.scroll_offset(), 0);
    }

    #[test]
    fn scroll_returns_none() {
        let mut panel = AvailablePanel::new();
        let result = panel.update(AvailablePanelMessage::Scroll(ScrollDirection::Down));
        assert_eq!(result, None);
    }

    // -- key_to_message: filter mode --

    #[test]
    fn key_to_message_filter_mode_esc() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::ToggleFilterMode);
        assert_eq!(
            panel.key_to_message(key(KeyCode::Esc)),
            Some(AvailablePanelMessage::ExitFilterMode { clear: true })
        );
    }

    #[test]
    fn key_to_message_filter_mode_enter() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::ToggleFilterMode);
        assert_eq!(
            panel.key_to_message(key(KeyCode::Enter)),
            Some(AvailablePanelMessage::ExitFilterMode { clear: false })
        );
    }

    #[test]
    fn key_to_message_filter_mode_char() {
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::ToggleFilterMode);
        let msg = panel.key_to_message(key(KeyCode::Char('x')));
        assert!(matches!(msg, Some(AvailablePanelMessage::FilterKeyPress(_))));
    }

    // -- key_to_message: normal mode --

    #[test]
    fn key_to_message_up() {
        let panel = AvailablePanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Up)),
            Some(AvailablePanelMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_down() {
        let panel = AvailablePanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Down)),
            Some(AvailablePanelMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_k_scrolls_up() {
        let panel = AvailablePanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('k'))),
            Some(AvailablePanelMessage::Scroll(ScrollDirection::Up))
        );
    }

    #[test]
    fn key_to_message_j_scrolls_down() {
        let panel = AvailablePanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Char('j'))),
            Some(AvailablePanelMessage::Scroll(ScrollDirection::Down))
        );
    }

    #[test]
    fn key_to_message_page_up() {
        let panel = AvailablePanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageUp)),
            Some(AvailablePanelMessage::Scroll(ScrollDirection::PageUp))
        );
    }

    #[test]
    fn key_to_message_page_down() {
        let panel = AvailablePanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::PageDown)),
            Some(AvailablePanelMessage::Scroll(ScrollDirection::PageDown))
        );
    }

    #[test]
    fn key_to_message_home() {
        let panel = AvailablePanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::Home)),
            Some(AvailablePanelMessage::Scroll(ScrollDirection::Top))
        );
    }

    #[test]
    fn key_to_message_end() {
        let panel = AvailablePanel::new();
        assert_eq!(
            panel.key_to_message(key(KeyCode::End)),
            Some(AvailablePanelMessage::Scroll(ScrollDirection::Bottom))
        );
    }

    #[test]
    fn key_to_message_unrecognized_returns_none() {
        let panel = AvailablePanel::new();
        assert_eq!(panel.key_to_message(key(KeyCode::Char('x'))), None);
        assert_eq!(panel.key_to_message(key(KeyCode::Enter)), None);
    }

    // -- filter_players --

    #[test]
    fn filter_no_filters() {
        let players = vec![
            make_test_player("Player A", vec![Position::Catcher], 20.0),
            make_test_player("Player B", vec![Position::FirstBase], 15.0),
        ];
        let result = filter_players(&players, None, "");
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_by_position() {
        let players = vec![
            make_test_player("Player A", vec![Position::Catcher], 20.0),
            make_test_player("Player B", vec![Position::FirstBase], 15.0),
            make_test_player(
                "Player C",
                vec![Position::Catcher, Position::FirstBase],
                10.0,
            ),
        ];
        let result = filter_players(&players, Some(&Position::Catcher), "");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "Player A");
        assert_eq!(result[1].name, "Player C");
    }

    #[test]
    fn filter_by_text() {
        let players = vec![
            make_test_player("Mike Trout", vec![Position::CenterField], 50.0),
            make_test_player("Aaron Judge", vec![Position::RightField], 45.0),
            make_test_player("Mike Yastrzemski", vec![Position::LeftField], 10.0),
        ];
        let result = filter_players(&players, None, "mike");
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_by_position_and_text() {
        let players = vec![
            make_test_player("Mike Trout", vec![Position::CenterField], 50.0),
            make_test_player("Mike Zunino", vec![Position::Catcher], 5.0),
            make_test_player("Aaron Judge", vec![Position::RightField], 45.0),
        ];
        let result = filter_players(&players, Some(&Position::Catcher), "mike");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "Mike Zunino");
    }

    #[test]
    fn filter_empty_players() {
        let players: Vec<PlayerValuation> = Vec::new();
        let result = filter_players(&players, None, "test");
        assert!(result.is_empty());
    }

    // -- format_positions --

    #[test]
    fn format_positions_basic() {
        assert_eq!(format_positions(&[Position::Catcher]), "C");
        assert_eq!(
            format_positions(&[Position::FirstBase, Position::ThirdBase]),
            "1B/3B"
        );
    }

    #[test]
    fn format_positions_empty() {
        assert_eq!(format_positions(&[]), "--");
    }

    // -- View (render) doesn't panic --

    #[test]
    fn view_does_not_panic_with_defaults() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = AvailablePanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_players() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = AvailablePanel::new();
        let players = vec![
            make_test_player("Player A", vec![Position::Catcher], 20.0),
            make_test_player("Player B", vec![Position::FirstBase], 15.0),
        ];
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &players, None, false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_when_focused() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = AvailablePanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, true))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_in_filter_mode() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = AvailablePanel::new();
        panel.update(AvailablePanelMessage::ToggleFilterMode);
        panel.update(AvailablePanelMessage::FilterKeyPress(key(KeyCode::Char(
            't',
        ))));
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_nominated_player() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = AvailablePanel::new();
        let players = vec![
            make_test_player("Player A", vec![Position::Catcher], 20.0),
            make_test_player("Player B", vec![Position::FirstBase], 15.0),
        ];
        terminal
            .draw(|frame| {
                panel.view(frame, frame.area(), &players, Some("Player A"), false)
            })
            .unwrap();
    }
}
