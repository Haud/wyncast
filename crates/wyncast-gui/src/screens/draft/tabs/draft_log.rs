use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::Id as WidgetId;
use iced::{Color, Element, Length, Task};
use std::collections::HashMap;
use twui::{
    BoxStyle, Icons, TextSize, TextStyle,
    Colors, Opacity,
    empty_state, frame, text,
};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::valuation::zscore::PlayerValuation;
use wyncast_baseball::draft::pick::DraftPick;

use crate::widgets::data_table::{Column, DataTableStyle, ROW_HEIGHT, data_table};

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum DraftLogMessage {
    ScrollBy(ScrollDirection),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct DraftLogPanel {
    pub draft_log: Vec<DraftPick>,
    pub available_players: Vec<PlayerValuation>,
    scroll_id: WidgetId,
}

impl DraftLogPanel {
    pub fn new() -> Self {
        Self {
            draft_log: Vec::new(),
            available_players: Vec::new(),
            scroll_id: WidgetId::unique(),
        }
    }

    pub fn update(&mut self, msg: DraftLogMessage) -> Task<DraftLogMessage> {
        match msg {
            DraftLogMessage::ScrollBy(dir) => {
                let (_, dy) = scroll_delta(dir);
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: 0.0, y: dy })
            }
        }
    }

    pub fn view(&self) -> Element<'_, DraftLogMessage> {
        if self.draft_log.is_empty() {
            let empty: Element<'_, DraftLogMessage> = empty_state(
                Icons::Database,
                "No picks yet",
                Some("Picks will appear here as the draft progresses."),
            )
            .into();

            return frame(
                empty,
                BoxStyle {
                    width: Length::Fill,
                    height: Length::Fill,
                    ..Default::default()
                },
            )
            .into();
        }

        let value_map: HashMap<&str, f64> = self
            .available_players
            .iter()
            .map(|p| (p.name.as_str(), p.dollar_value))
            .collect();

        let picks_rev: Vec<&DraftPick> = self.draft_log.iter().rev().collect();

        let tints: Vec<Option<Color>> = picks_rev
            .iter()
            .map(|p| pick_tint(p.price, value_map.get(p.player_name.as_str()).copied()))
            .collect();

        let rows = build_rows(&picks_rev, &value_map);

        let style = DataTableStyle {
            alternate_rows: true,
            row_tint_fn: Some(Box::new(move |idx| tints.get(idx).copied().flatten())),
        };

        let table = data_table(
            columns(),
            rows,
            self.scroll_id.clone(),
            None,
            style,
            None,
        );

        frame(
            table,
            BoxStyle {
                width: Length::Fill,
                height: Length::Fill,
                ..Default::default()
            },
        )
        .into()
    }
}

impl Default for DraftLogPanel {
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
        Column::new("Time", Length::Fixed(60.0), TextAlign::Center),
        Column::new("Team", Length::FillPortion(2), TextAlign::Left),
        Column::new("Player", Length::FillPortion(3), TextAlign::Left),
        Column::new("Pos", Length::Fixed(50.0), TextAlign::Left),
        Column::new("Price", Length::Fixed(60.0), TextAlign::Right),
        Column::new("Value", Length::Fixed(60.0), TextAlign::Right),
        Column::new("Δ", Length::Fixed(60.0), TextAlign::Right),
    ]
}

fn build_rows<'a>(
    picks_rev: &[&'a DraftPick],
    value_map: &HashMap<&str, f64>,
) -> Vec<Vec<Element<'static, DraftLogMessage>>> {
    picks_rev
        .iter()
        .map(|pick| {
            let value_opt = value_map.get(pick.player_name.as_str()).copied();
            let value_str = value_opt
                .map(|v| format!("${v:.0}"))
                .unwrap_or_else(|| "--".to_string());
            let delta_str = match value_opt {
                Some(val) => format!("{:+.0}", pick.price as f64 - val),
                None => "--".to_string(),
            };

            vec![
                cell_text(format!("{}", pick.pick_number)),
                cell_text("--".to_string()),
                cell_text(pick.team_name.clone()),
                cell_text(pick.player_name.clone()),
                cell_text(pick.position.clone()),
                cell_text(format!("${}", pick.price)),
                cell_text(value_str),
                cell_text(delta_str),
            ]
        })
        .collect()
}

fn cell_text(content: String) -> Element<'static, DraftLogMessage> {
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
// Pure tint helper
// ---------------------------------------------------------------------------

/// Compute the background tint color for a pick row based on price vs value.
///
/// Green if price < value (bargain).
/// Red if price > 1.2 * value (overpay).
/// None otherwise (fair price or no value available).
pub fn pick_tint(price: u32, value: Option<f64>) -> Option<Color> {
    match value {
        Some(val) if val > 0.0 => {
            let ratio = price as f64 / val;
            if ratio < 1.0 {
                Some(Colors::Success.rgba(Opacity::O20))
            } else if ratio > 1.2 {
                Some(Colors::Destructive.rgba(Opacity::O20))
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_tint_bargain_returns_green() {
        let result = pick_tint(20, Some(30.0));
        assert_eq!(result, Some(Colors::Success.rgba(Opacity::O20)));
    }

    #[test]
    fn pick_tint_overpay_returns_red() {
        let result = pick_tint(40, Some(30.0));
        assert_eq!(result, Some(Colors::Destructive.rgba(Opacity::O20)));
    }

    #[test]
    fn pick_tint_at_value_returns_none() {
        // ratio = 1.0, not < 1.0 and not > 1.2
        let result = pick_tint(30, Some(30.0));
        assert_eq!(result, None);
    }

    #[test]
    fn pick_tint_at_1_2x_value_returns_none() {
        // ratio = 1.2 exactly, NOT > 1.2
        let result = pick_tint(36, Some(30.0));
        assert_eq!(result, None);
    }

    #[test]
    fn pick_tint_just_above_1_2x_value_returns_red() {
        // price=37, value=30 => ratio = 37/30 ≈ 1.233 > 1.2
        let result = pick_tint(37, Some(30.0));
        assert_eq!(result, Some(Colors::Destructive.rgba(Opacity::O20)));
    }

    #[test]
    fn pick_tint_no_value_returns_none() {
        let result = pick_tint(30, None);
        assert_eq!(result, None);
    }

    #[test]
    fn pick_tint_zero_value_returns_none() {
        let result = pick_tint(30, Some(0.0));
        assert_eq!(result, None);
    }

    #[test]
    fn new_starts_with_empty_state() {
        let panel = DraftLogPanel::new();
        assert!(panel.draft_log.is_empty());
        assert!(panel.available_players.is_empty());
    }

    #[test]
    fn default_matches_new() {
        let panel = DraftLogPanel::default();
        assert!(panel.draft_log.is_empty());
        assert!(panel.available_players.is_empty());
    }
}
