use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::Id as WidgetId;
use iced::{Element, Length, Padding, Task};
use twui::{
    Colors, TextColor, TextSize, TextStyle,
    BoxStyle, StackGap, StackStyle,
    frame, h_stack, text, v_stack,
};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::draft::pick::Position;
use wyncast_baseball::valuation::zscore::PlayerValuation;

use crate::widgets::{
    data_table::{Column, DataTableStyle, ROW_HEIGHT, data_table},
    filter_input::filter_input,
};

// ---------------------------------------------------------------------------
// Pure filter helper (unit-tested below)
// ---------------------------------------------------------------------------

/// Filter players by position and text.
pub fn filter_players<'a>(
    players: &'a [PlayerValuation],
    position_filter: Option<Position>,
    filter_text: &str,
) -> Vec<&'a PlayerValuation> {
    let text_lower = filter_text.to_lowercase();
    players
        .iter()
        .filter(|p| {
            if let Some(pos) = position_filter {
                if !p.positions.contains(&pos) {
                    return false;
                }
            }
            if !text_lower.is_empty() && !p.name.to_lowercase().contains(&text_lower) {
                return false;
            }
            true
        })
        .collect()
}

/// Format a player's position list compactly (e.g. "1B/OF").
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
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum AvailableMessage {
    FilterChanged(String),
    FilterFocused(bool),
    PositionFilterOpened,
    PositionSelected(Option<Position>),
    PositionFilterClosed,
    #[allow(dead_code)]
    RowClicked(usize),
    ScrollBy(ScrollDirection),
    NominationActive(String),
    NominationCleared,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct AvailablePanel {
    // Public: parent sets these from UiUpdate
    pub available_players: Vec<PlayerValuation>,
    // Internal state
    filter_text: String,
    filter_focused: bool,
    pub position_filter: Option<Position>,
    scroll_id: WidgetId,
    highlighted_player_name: Option<String>,
    filter_input_id: WidgetId,
}

impl AvailablePanel {
    pub fn new() -> Self {
        Self {
            available_players: Vec::new(),
            filter_text: String::new(),
            filter_focused: false,
            position_filter: None,
            scroll_id: WidgetId::unique(),
            highlighted_player_name: None,
            filter_input_id: WidgetId::unique(),
        }
    }

    #[allow(dead_code)]
    pub fn filter_focused(&self) -> bool {
        self.filter_focused
    }

    pub fn update(&mut self, msg: AvailableMessage) -> Task<AvailableMessage> {
        match msg {
            AvailableMessage::FilterChanged(text) => {
                self.filter_text = text;
                Task::none()
            }
            AvailableMessage::FilterFocused(focused) => {
                self.filter_focused = focused;
                if focused {
                    operation::focus(self.filter_input_id.clone())
                } else {
                    Task::none()
                }
            }
            AvailableMessage::PositionFilterOpened => {
                // Modal open/close is tracked by ModalStack on DraftScreen.
                Task::none()
            }
            AvailableMessage::PositionSelected(pos) => {
                self.position_filter = pos;
                Task::none()
            }
            AvailableMessage::PositionFilterClosed => {
                Task::none()
            }
            AvailableMessage::RowClicked(_) => Task::none(),
            AvailableMessage::ScrollBy(dir) => {
                let (_, dy) = scroll_delta(dir);
                operation::scroll_by(
                    self.scroll_id.clone(),
                    AbsoluteOffset { x: 0.0, y: dy },
                )
            }
            AvailableMessage::NominationActive(player_name) => {
                self.highlighted_player_name = Some(player_name.clone());
                // Scroll nominated row into view
                let filtered = filter_players(
                    &self.available_players,
                    self.position_filter,
                    &self.filter_text,
                );
                if let Some(idx) = filtered.iter().position(|p| p.name == player_name) {
                    let target_y = idx as f32 * ROW_HEIGHT;
                    operation::scroll_to(
                        self.scroll_id.clone(),
                        AbsoluteOffset { x: 0.0, y: target_y },
                    )
                } else {
                    Task::none()
                }
            }
            AvailableMessage::NominationCleared => {
                self.highlighted_player_name = None;
                Task::none()
            }
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, AvailableMessage> {
        let filtered = filter_players(
            &self.available_players,
            self.position_filter,
            &self.filter_text,
        );
        let total = filtered.len();

        let highlighted_index = self
            .highlighted_player_name
            .as_deref()
            .and_then(|name| filtered.iter().position(|p| p.name == name));

        let rows = build_rows(&filtered);
        let table = data_table(
            columns(),
            rows,
            self.scroll_id.clone(),
            highlighted_index,
            DataTableStyle::default(),
            None,
        );

        let filter_bar = self.view_filter_bar(total);

        let table_area: Element<'a, AvailableMessage> = frame(
            table,
            BoxStyle {
                width: Length::Fill,
                height: Length::Fill,
                ..Default::default()
            },
        )
        .into();

        v_stack(
            vec![filter_bar, table_area],
            StackStyle {
                gap: StackGap::None,
                width: Length::Fill,
                height: Length::Fill,
                ..Default::default()
            },
        )
        .into()
    }

    fn view_filter_bar<'a>(&'a self, count: usize) -> Element<'a, AvailableMessage> {
        let input: Element<'a, AvailableMessage> = filter_input(
            &self.filter_text,
            AvailableMessage::FilterChanged,
            self.filter_input_id.clone(),
            "Filter players… (/)",
        );

        let pos_label = match self.position_filter {
            None => "All",
            Some(pos) => pos.display_str(),
        };
        let pos_btn_text = format!("[{pos_label}] p");
        let pos_button: Element<'a, AvailableMessage> = text(
            pos_btn_text,
            TextStyle {
                size: TextSize::Xs,
                color: TextColor::Dimmed,
                ..Default::default()
            },
        )
        .into();

        let count_label: Element<'a, AvailableMessage> = text(
            format!("{count} players"),
            TextStyle {
                size: TextSize::Xs,
                color: TextColor::Dimmed,
                ..Default::default()
            },
        )
        .into();

        h_stack(
            vec![input, pos_button, count_label],
            StackStyle {
                gap: StackGap::Sm,
                width: Length::Fill,
                padding: Padding::new(6.0),
                background: Some(Colors::BgElevated),
                ..Default::default()
            },
        )
        .into()
    }
}

impl Default for AvailablePanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Column spec
// ---------------------------------------------------------------------------

fn columns() -> Vec<Column> {
    use twui::TextAlign;
    vec![
        Column::new("#", Length::Fixed(36.0), TextAlign::Right),
        Column::new("Name", Length::FillPortion(3), TextAlign::Left),
        Column::new("Pos", Length::Fixed(64.0), TextAlign::Left),
        Column::new("$Val", Length::Fixed(64.0), TextAlign::Right),
        Column::new("VOR", Length::Fixed(72.0), TextAlign::Right),
        Column::new("zTotal", Length::Fixed(72.0), TextAlign::Right),
    ]
}

fn build_rows<'a>(filtered: &[&'a PlayerValuation]) -> Vec<Vec<Element<'a, AvailableMessage>>> {
    filtered
        .iter()
        .enumerate()
        .map(|(i, p)| {
            vec![
                cell_text(format!("{}", i + 1)),
                cell_text(p.name.clone()),
                cell_text(format_positions(&p.positions)),
                cell_text(format!("${:.0}", p.dollar_value)),
                cell_text(format!("{:.1}", p.vor)),
                cell_text(format!("{:.2}", p.total_zscore)),
            ]
        })
        .collect()
}

fn cell_text(content: String) -> Element<'static, AvailableMessage> {
    text(
        content,
        TextStyle {
            size: TextSize::Sm,
            ..Default::default()
        },
    )
    .into()
}

fn scroll_delta(dir: ScrollDirection) -> (f32, f32) {
    match dir {
        ScrollDirection::Up => (0.0, -40.0),
        ScrollDirection::Down => (0.0, 40.0),
        ScrollDirection::PageUp => (0.0, -(ROW_HEIGHT * 10.0)),
        ScrollDirection::PageDown => (0.0, ROW_HEIGHT * 10.0),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wyncast_baseball::valuation::zscore::{CategoryZScores, ProjectionData};
    use wyncast_baseball::test_utils::test_registry;
    use wyncast_core::stats::CategoryValues;

    fn make_player(name: &str, positions: Vec<Position>, dollar: f64) -> PlayerValuation {
        PlayerValuation {
            name: name.to_string(),
            team: "TST".to_string(),
            positions,
            is_pitcher: false,
            is_two_way: false,
            pitcher_type: None,
            projection: ProjectionData {
                values: HashMap::from([
                    ("pa".into(), 600.0),
                    ("hr".into(), 20.0),
                    ("r".into(), 80.0),
                    ("rbi".into(), 80.0),
                    ("sb".into(), 10.0),
                    ("avg".into(), 0.270),
                ]),
            },
            total_zscore: 2.5,
            category_zscores: CategoryZScores::hitter(
                CategoryValues::zeros(test_registry().len()),
                2.5,
            ),
            vor: 4.0,
            initial_vor: 4.0,
            best_position: None,
            dollar_value: dollar,
        }
    }

    // -- filter_players --

    #[test]
    fn filter_no_filters_returns_all() {
        let players = vec![
            make_player("Mike Trout", vec![Position::CenterField], 50.0),
            make_player("Aaron Judge", vec![Position::RightField], 45.0),
        ];
        let result = filter_players(&players, None, "");
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_text_case_insensitive() {
        let players = vec![
            make_player("Mike Trout", vec![Position::CenterField], 50.0),
            make_player("Aaron Judge", vec![Position::RightField], 45.0),
        ];
        let result = filter_players(&players, None, "MIKE");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "Mike Trout");
    }

    #[test]
    fn filter_by_position_only() {
        let players = vec![
            make_player("A", vec![Position::Catcher], 20.0),
            make_player("B", vec![Position::FirstBase], 15.0),
            make_player("C", vec![Position::Catcher, Position::FirstBase], 10.0),
        ];
        let result = filter_players(&players, Some(Position::Catcher), "");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "A");
        assert_eq!(result[1].name, "C");
    }

    #[test]
    fn filter_by_text_and_position() {
        let players = vec![
            make_player("Mike Trout", vec![Position::CenterField], 50.0),
            make_player("Mike Zunino", vec![Position::Catcher], 5.0),
            make_player("Aaron Judge", vec![Position::RightField], 45.0),
        ];
        let result = filter_players(&players, Some(Position::Catcher), "mike");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "Mike Zunino");
    }

    #[test]
    fn filter_empty_list_returns_empty() {
        let players: Vec<PlayerValuation> = Vec::new();
        let result = filter_players(&players, None, "test");
        assert!(result.is_empty());
    }

    #[test]
    fn filter_no_match_returns_empty() {
        let players = vec![make_player("Mike Trout", vec![Position::CenterField], 50.0)];
        let result = filter_players(&players, None, "zzznomatch");
        assert!(result.is_empty());
    }

    // -- format_positions --

    #[test]
    fn format_positions_empty() {
        assert_eq!(format_positions(&[]), "--");
    }

    #[test]
    fn format_positions_single() {
        assert_eq!(format_positions(&[Position::Catcher]), "C");
    }

    #[test]
    fn format_positions_multi() {
        assert_eq!(
            format_positions(&[Position::FirstBase, Position::ThirdBase]),
            "1B/3B"
        );
    }

    // -- AvailablePanel update --

    #[test]
    fn new_starts_with_empty_state() {
        let panel = AvailablePanel::new();
        assert!(panel.filter_text.is_empty());
        assert!(!panel.filter_focused);
        assert!(panel.position_filter.is_none());
        assert!(panel.highlighted_player_name.is_none());
        assert!(panel.available_players.is_empty());
    }

    #[test]
    fn update_filter_changed_updates_text() {
        let mut panel = AvailablePanel::new();
        let _ = panel.update(AvailableMessage::FilterChanged("test".to_string()));
        assert_eq!(panel.filter_text, "test");
    }

    #[test]
    fn update_filter_focused_sets_flag() {
        let mut panel = AvailablePanel::new();
        let _ = panel.update(AvailableMessage::FilterFocused(true));
        assert!(panel.filter_focused);
        let _ = panel.update(AvailableMessage::FilterFocused(false));
        assert!(!panel.filter_focused);
    }

    #[test]
    fn update_position_filter_opened_is_noop_on_panel() {
        let mut panel = AvailablePanel::new();
        let _ = panel.update(AvailableMessage::PositionFilterOpened);
        // ModalStack on DraftScreen owns open/close; panel state unchanged.
        assert!(panel.position_filter.is_none());
    }

    #[test]
    fn update_position_selected_sets_filter() {
        let mut panel = AvailablePanel::new();
        let _ = panel.update(AvailableMessage::PositionSelected(Some(Position::Catcher)));
        assert_eq!(panel.position_filter, Some(Position::Catcher));
    }

    #[test]
    fn update_position_selected_none_clears_filter() {
        let mut panel = AvailablePanel::new();
        let _ = panel.update(AvailableMessage::PositionSelected(Some(Position::Catcher)));
        let _ = panel.update(AvailableMessage::PositionSelected(None));
        assert!(panel.position_filter.is_none());
    }

    #[test]
    fn update_position_filter_closed_is_noop_on_panel() {
        let mut panel = AvailablePanel::new();
        let _ = panel.update(AvailableMessage::PositionFilterClosed);
        assert!(panel.position_filter.is_none());
    }

    #[test]
    fn update_row_clicked_is_noop() {
        let mut panel = AvailablePanel::new();
        let _ = panel.update(AvailableMessage::RowClicked(3));
        // No state change
        assert!(panel.filter_text.is_empty());
    }

    #[test]
    fn update_nomination_active_sets_highlighted() {
        let mut panel = AvailablePanel::new();
        let _ = panel.update(AvailableMessage::NominationActive("Mike Trout".to_string()));
        assert_eq!(panel.highlighted_player_name, Some("Mike Trout".to_string()));
    }

    #[test]
    fn update_nomination_cleared_clears_highlighted() {
        let mut panel = AvailablePanel::new();
        let _ = panel.update(AvailableMessage::NominationActive("Mike Trout".to_string()));
        let _ = panel.update(AvailableMessage::NominationCleared);
        assert!(panel.highlighted_player_name.is_none());
    }

    #[test]
    fn filter_players_combined_text_and_position() {
        let players = vec![
            make_player("Fred Catcher", vec![Position::Catcher], 10.0),
            make_player("Fred Pitcher", vec![Position::StartingPitcher], 10.0),
            make_player("Bob Catcher", vec![Position::Catcher], 8.0),
        ];
        let result = filter_players(&players, Some(Position::Catcher), "fred");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "Fred Catcher");
    }
}
