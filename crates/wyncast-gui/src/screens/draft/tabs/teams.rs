use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::Id as WidgetId;
use iced::{Element, Length, Task};
use twui::{
    BoxStyle, Icons, TextSize, TextStyle,
    empty_state, frame, text,
};
use wyncast_app::protocol::{ScrollDirection, TeamSnapshot};

use crate::widgets::data_table::{Column, DataTableStyle, ROW_HEIGHT, data_table};

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum TeamsMessage {
    ScrollBy(ScrollDirection),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct TeamsPanel {
    pub team_snapshots: Vec<TeamSnapshot>,
    pub salary_cap: u32,
    scroll_id: WidgetId,
}

impl TeamsPanel {
    pub fn new() -> Self {
        Self {
            team_snapshots: Vec::new(),
            salary_cap: 0,
            scroll_id: WidgetId::unique(),
        }
    }

    pub fn update(&mut self, msg: TeamsMessage) -> Task<TeamsMessage> {
        match msg {
            TeamsMessage::ScrollBy(dir) => {
                let (_, dy) = scroll_delta(dir);
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: 0.0, y: dy })
            }
        }
    }

    pub fn view(&self) -> Element<'_, TeamsMessage> {
        if self.team_snapshots.is_empty() {
            let empty: Element<'_, TeamsMessage> = empty_state(
                Icons::Grid2x2,
                "No team data yet",
                Some("Team summaries will appear here during the draft."),
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

        let rows = build_rows(&self.team_snapshots, self.salary_cap);

        let table = data_table(
            columns(),
            rows,
            self.scroll_id.clone(),
            None,
            DataTableStyle::default(),
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

impl Default for TeamsPanel {
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
        Column::new("Team", Length::FillPortion(3), TextAlign::Left),
        Column::new("Spent", Length::Fixed(80.0), TextAlign::Right),
        Column::new("Remaining", Length::Fixed(80.0), TextAlign::Right),
        Column::new("Slots Filled", Length::Fixed(90.0), TextAlign::Center),
        Column::new("Max Bid", Length::Fixed(80.0), TextAlign::Right),
    ]
}

fn build_rows(
    snapshots: &[TeamSnapshot],
    salary_cap: u32,
) -> Vec<Vec<Element<'static, TeamsMessage>>> {
    snapshots
        .iter()
        .map(|team| {
            let spent = salary_cap.saturating_sub(team.budget_remaining);
            let max_bid = compute_max_bid(team.budget_remaining, team.slots_filled, team.total_slots);

            vec![
                cell_text(team.name.clone()),
                cell_text(format!("${spent}")),
                cell_text(format!("${}", team.budget_remaining)),
                cell_text(format!("{}/{}", team.slots_filled, team.total_slots)),
                cell_text(format!("${max_bid}")),
            ]
        })
        .collect()
}

fn cell_text(content: String) -> Element<'static, TeamsMessage> {
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
// Pure helpers
// ---------------------------------------------------------------------------

/// Compute the maximum bid a team can make.
///
/// The team must keep at least $1 for each remaining empty slot (minus the
/// current bid slot itself), so:
///   max_bid = budget_remaining - max(empty_slots - 1, 0)
pub fn compute_max_bid(remaining: u32, slots_filled: usize, total_slots: usize) -> u32 {
    let empty_slots = total_slots.saturating_sub(slots_filled);
    remaining.saturating_sub(empty_slots.saturating_sub(1) as u32)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_max_bid_standard_case() {
        // remaining=200, filled=10, total=26 => empty=16, max_bid=200-(16-1)=185
        assert_eq!(compute_max_bid(200, 10, 26), 185);
    }

    #[test]
    fn compute_max_bid_one_empty_slot() {
        // remaining=10, filled=25, total=26 => empty=1, max_bid=10-(1-1)=10-0=10
        assert_eq!(compute_max_bid(10, 25, 26), 10);
    }

    #[test]
    fn compute_max_bid_saturating_underflow() {
        // remaining=1, filled=0, total=26 => empty=26, max_bid=1.saturating_sub(25)=0
        assert_eq!(compute_max_bid(1, 0, 26), 0);
    }

    #[test]
    fn compute_max_bid_fully_filled() {
        // remaining=5, filled=26, total=26 => empty=0, max_bid=5.saturating_sub(0)=5
        assert_eq!(compute_max_bid(5, 26, 26), 5);
    }

    #[test]
    fn new_starts_with_empty_state() {
        let panel = TeamsPanel::new();
        assert!(panel.team_snapshots.is_empty());
        assert_eq!(panel.salary_cap, 0);
    }

    #[test]
    fn default_matches_new() {
        let panel = TeamsPanel::default();
        assert!(panel.team_snapshots.is_empty());
    }
}
