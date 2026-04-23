use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::Id as WidgetId;
use iced::{Element, Length, Task};
use twui::{
    BoxStyle, Icons, TextAlign, TextColor, TextSize, TextStyle, TextWeight,
    empty_state, frame, text,
};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::matchup::{DailyPlayerRow, DailyTotals, ScoringDay, TeamSide};

use crate::widgets::data_table::{Column, DataTableStyle, ROW_HEIGHT, data_table};

#[derive(Debug, Clone)]
pub enum DailyStatsMessage {
    ScrollBy(ScrollDirection),
}

pub struct DailyStatsPanel {
    scroll_id: WidgetId,
}

impl DailyStatsPanel {
    pub fn new() -> Self {
        Self { scroll_id: WidgetId::unique() }
    }

    pub fn update(&mut self, msg: DailyStatsMessage) -> Task<DailyStatsMessage> {
        match msg {
            DailyStatsMessage::ScrollBy(dir) => {
                let dy = scroll_delta(dir);
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: 0.0, y: dy })
            }
        }
    }

    pub fn view<'a>(&self, day: Option<&'a ScoringDay>, home_name: &'a str, away_name: &'a str) -> Element<'a, DailyStatsMessage> {
        let Some(day) = day else {
            return frame(
                empty_state(Icons::Info, "No daily data", Some("Select a day with ←/→ keys.")),
                BoxStyle { width: Length::Fill, height: Length::Fill, ..Default::default() },
            )
            .into();
        };

        // Show batting section (home then away)
        let stat_cols = &day.batting_stat_columns;

        if stat_cols.is_empty() && day.home.batting_rows.is_empty() && day.away.batting_rows.is_empty() {
            return frame(
                empty_state(Icons::Info, "No stats for this day", None),
                BoxStyle { width: Length::Fill, height: Length::Fill, ..Default::default() },
            )
            .into();
        }

        let cols = build_columns(stat_cols);
        let rows = build_rows(
            &day.home.batting_rows,
            day.home.batting_totals.as_ref(),
            &day.away.batting_rows,
            day.away.batting_totals.as_ref(),
            stat_cols,
            home_name,
            away_name,
        );

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

impl Default for DailyStatsPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Column helpers
// ---------------------------------------------------------------------------

fn build_columns(stat_headers: &[String]) -> Vec<Column> {
    let mut cols = vec![
        Column::new("Side", Length::Fixed(64.0), TextAlign::Left),
        Column::new("Slot", Length::Fixed(50.0), TextAlign::Left),
        Column::new("Player", Length::FillPortion(3), TextAlign::Left),
        Column::new("Team", Length::Fixed(50.0), TextAlign::Left),
        Column::new("Opp", Length::Fixed(64.0), TextAlign::Left),
    ];

    for header in stat_headers {
        let width = stat_col_width(header);
        cols.push(Column::new(header.clone(), Length::Fixed(width), TextAlign::Right));
    }

    cols
}

fn stat_col_width(abbrev: &str) -> f32 {
    match abbrev {
        "AVG" | "OBP" | "SLG" | "OPS" => 64.0,
        "ERA" | "WHIP" => 64.0,
        "IP" => 56.0,
        _ => 52.0,
    }
}

// ---------------------------------------------------------------------------
// Row helpers
// ---------------------------------------------------------------------------

fn build_rows<'a>(
    home_batting: &'a [DailyPlayerRow],
    home_totals: Option<&'a DailyTotals>,
    away_batting: &'a [DailyPlayerRow],
    away_totals: Option<&'a DailyTotals>,
    stat_cols: &'a [String],
    home_name: &'a str,
    away_name: &'a str,
) -> Vec<Vec<Element<'a, DailyStatsMessage>>> {
    let mut rows: Vec<Vec<Element<DailyStatsMessage>>> = Vec::new();

    for player in home_batting {
        rows.push(player_row(TeamSide::Home, player, stat_cols));
    }
    if let Some(totals) = home_totals {
        rows.push(totals_row(home_name, stat_cols.len(), &totals.stats));
    }

    for player in away_batting {
        rows.push(player_row(TeamSide::Away, player, stat_cols));
    }
    if let Some(totals) = away_totals {
        rows.push(totals_row(away_name, stat_cols.len(), &totals.stats));
    }

    rows
}

fn player_row<'a>(
    side: TeamSide,
    row: &'a DailyPlayerRow,
    stat_cols: &'a [String],
) -> Vec<Element<'a, DailyStatsMessage>> {
    let side_label = match side {
        TeamSide::Home => "Home",
        TeamSide::Away => "Away",
    };

    let mut cells: Vec<Element<DailyStatsMessage>> = vec![
        cell(side_label),
        cell(&row.slot),
        cell(&row.player_name),
        cell(&row.team),
        cell(row.opponent.as_deref().unwrap_or("—")),
    ];

    for (i, _header) in stat_cols.iter().enumerate() {
        let val_str = row
            .stats
            .get(i)
            .and_then(|v| *v)
            .map(|v| format_stat(v, _header))
            .unwrap_or_else(|| "—".to_string());
        cells.push(cell_right(&val_str));
    }

    cells
}

fn totals_row<'a>(
    team_name: &'a str,
    num_stat_cols: usize,
    stats: &'a [Option<f64>],
) -> Vec<Element<'a, DailyStatsMessage>> {
    let mut cells: Vec<Element<DailyStatsMessage>> = vec![
        bold_cell(team_name),
        bold_cell("TOTAL"),
        iced::widget::Space::new().into(), // player col spacer
        iced::widget::Space::new().into(), // team
        iced::widget::Space::new().into(), // opp
    ];

    for i in 0..num_stat_cols {
        let val_str = stats
            .get(i)
            .and_then(|v| *v)
            .map(|v| format!("{v:.0}"))
            .unwrap_or_else(|| "—".to_string());
        cells.push(bold_cell_right(&val_str));
    }

    cells
}

fn format_stat(val: f64, header: &str) -> String {
    match header {
        "AVG" | "OBP" | "SLG" | "OPS" => format!("{val:.3}"),
        "ERA" | "WHIP" | "K/9" | "BB/9" | "K/BB" => format!("{val:.2}"),
        "IP" => format!("{val:.1}"),
        _ => format!("{}", val as i64),
    }
}

fn cell(content: &str) -> Element<'static, DailyStatsMessage> {
    text(content.to_owned(), TextStyle { size: TextSize::Sm, ..Default::default() }).into()
}

fn cell_right(content: &str) -> Element<'static, DailyStatsMessage> {
    text(
        content.to_owned(),
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Default,
            ..Default::default()
        },
    )
    .into()
}

fn bold_cell(content: &str) -> Element<'static, DailyStatsMessage> {
    text(
        content.to_owned(),
        TextStyle {
            size: TextSize::Sm,
            weight: TextWeight::Semibold,
            ..Default::default()
        },
    )
    .into()
}

fn bold_cell_right(content: &str) -> Element<'static, DailyStatsMessage> {
    text(
        content.to_owned(),
        TextStyle {
            size: TextSize::Sm,
            weight: TextWeight::Semibold,
            ..Default::default()
        },
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
    fn build_columns_no_stats() {
        let cols = build_columns(&[]);
        assert_eq!(cols.len(), 5);
    }

    #[test]
    fn build_columns_with_stats() {
        let headers = vec!["R".to_string(), "HR".to_string(), "AVG".to_string()];
        let cols = build_columns(&headers);
        assert_eq!(cols.len(), 8);
    }

    #[test]
    fn format_stat_avg() {
        assert_eq!(format_stat(0.283, "AVG"), "0.283");
    }

    #[test]
    fn format_stat_counting() {
        assert_eq!(format_stat(5.0, "R"), "5");
    }

    #[test]
    fn format_stat_era() {
        assert_eq!(format_stat(3.45, "ERA"), "3.45");
    }

    #[test]
    fn build_rows_empty() {
        let rows = build_rows(&[], None, &[], None, &[], "Home", "Away");
        assert!(rows.is_empty());
    }

    #[test]
    fn build_rows_includes_totals_row() {
        let player = DailyPlayerRow {
            slot: "C".to_string(),
            player_name: "Ben Rice".to_string(),
            team: "NYY".to_string(),
            positions: vec!["C".to_string()],
            opponent: Some("@BOS".to_string()),
            game_status: None,
            stats: vec![Some(4.0)],
        };
        let totals = DailyTotals { stats: vec![Some(29.0)] };
        let stat_cols = vec!["AB".to_string()];
        let home_batting = [player];
        let rows = build_rows(&home_batting, Some(&totals), &[], None, &stat_cols, "Home", "Away");
        // 1 player + 1 totals = 2 rows
        assert_eq!(rows.len(), 2);
    }
}
