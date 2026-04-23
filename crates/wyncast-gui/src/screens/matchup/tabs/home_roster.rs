use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::Id as WidgetId;
use iced::{Element, Length, Task};
use twui::{BoxStyle, Icons, TextAlign, TextSize, TextStyle, empty_state, frame, text};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::matchup::{DailyPlayerRow, TeamDailyRoster};

use crate::widgets::data_table::{Column, DataTableStyle, ROW_HEIGHT, data_table};

#[derive(Debug, Clone)]
pub enum HomeRosterMessage {
    ScrollBy(ScrollDirection),
}

pub struct HomeRosterPanel {
    scroll_id: WidgetId,
}

impl HomeRosterPanel {
    pub fn new() -> Self {
        Self { scroll_id: WidgetId::unique() }
    }

    pub fn update(&mut self, msg: HomeRosterMessage) -> Task<HomeRosterMessage> {
        match msg {
            HomeRosterMessage::ScrollBy(dir) => {
                let dy = scroll_delta(dir);
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: 0.0, y: dy })
            }
        }
    }

    pub fn view<'a>(&self, roster: Option<&'a TeamDailyRoster>, team_name: &'a str) -> Element<'a, HomeRosterMessage> {
        match roster {
            None => empty_panel(team_name),
            Some(r) if r.batting_rows.is_empty() && r.pitching_rows.is_empty() => empty_panel(team_name),
            Some(r) => self.filled_panel(r),
        }
    }

    fn filled_panel<'a>(&self, roster: &'a TeamDailyRoster) -> Element<'a, HomeRosterMessage> {
        let cols = roster_columns();
        let rows = build_roster_rows(&roster.batting_rows, &roster.pitching_rows);

        let table = data_table(
            cols,
            rows,
            self.scroll_id.clone(),
            None,
            DataTableStyle::default(),
            None,
        );

        frame(
            table,
            BoxStyle { width: Length::Fill, height: Length::Fill, ..Default::default() },
        )
        .into()
    }
}

impl Default for HomeRosterPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn empty_panel<'a, Message: Clone + 'a>(team_name: &str) -> Element<'a, Message> {
    let msg = format!("No roster data for {team_name}");
    frame(
        empty_state(Icons::Info, &msg, Some("Waiting for matchup data.")),
        BoxStyle { width: Length::Fill, height: Length::Fill, ..Default::default() },
    )
    .into()
}

fn roster_columns() -> Vec<Column> {
    vec![
        Column::new("Slot", Length::Fixed(56.0), TextAlign::Left),
        Column::new("Player", Length::FillPortion(3), TextAlign::Left),
        Column::new("Team", Length::Fixed(56.0), TextAlign::Left),
        Column::new("Pos", Length::Fixed(64.0), TextAlign::Left),
        Column::new("Opp", Length::Fixed(80.0), TextAlign::Left),
        Column::new("Status", Length::Fixed(72.0), TextAlign::Left),
    ]
}

fn build_roster_rows<'a>(
    batting: &'a [DailyPlayerRow],
    pitching: &'a [DailyPlayerRow],
) -> Vec<Vec<Element<'a, HomeRosterMessage>>> {
    batting
        .iter()
        .chain(pitching.iter())
        .map(|row| {
            let pos = row.positions.join("/");
            let opp = row.opponent.as_deref().unwrap_or("—");
            let status = row.game_status.as_deref().unwrap_or("");

            vec![
                cell(&row.slot),
                cell(&row.player_name),
                cell(&row.team),
                cell(&pos),
                cell(opp),
                cell(status),
            ]
        })
        .collect()
}

fn cell(content: &str) -> Element<'static, HomeRosterMessage> {
    text(
        content.to_owned(),
        TextStyle { size: TextSize::Sm, ..Default::default() },
    )
    .into()
}

fn scroll_delta(dir: ScrollDirection) -> f32 {
    match dir {
        ScrollDirection::Up => -(ROW_HEIGHT * 3.0),
        ScrollDirection::Down => ROW_HEIGHT * 3.0,
        ScrollDirection::PageUp => -(ROW_HEIGHT * 10.0),
        ScrollDirection::PageDown => ROW_HEIGHT * 10.0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_panel_is_default() {
        let _ = HomeRosterPanel::new();
    }

    #[test]
    fn roster_columns_count() {
        assert_eq!(roster_columns().len(), 6);
    }

    #[test]
    fn build_roster_rows_empty() {
        let rows = build_roster_rows(&[], &[]);
        assert!(rows.is_empty());
    }

    #[test]
    fn build_roster_rows_combines_batting_pitching() {
        let bat = DailyPlayerRow {
            slot: "C".to_string(),
            player_name: "Ben Rice".to_string(),
            team: "NYY".to_string(),
            positions: vec!["C".to_string()],
            opponent: Some("@BOS".to_string()),
            game_status: None,
            stats: vec![],
        };
        let pit = DailyPlayerRow {
            slot: "SP".to_string(),
            player_name: "Gerrit Cole".to_string(),
            team: "NYY".to_string(),
            positions: vec!["SP".to_string()],
            opponent: None,
            game_status: Some("P".to_string()),
            stats: vec![],
        };
        let batting = [bat];
        let pitching = [pit];
        let rows = build_roster_rows(&batting, &pitching);
        assert_eq!(rows.len(), 2);
    }
}
