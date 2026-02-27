// Available players widget: sortable/filterable table of undrafted players.
//
// Scrollable table: Rank, Name, Pos, $Value, $Adj, VOR, z-score total
// Filter by position_filter and filter_text from ViewState
// Highlight nominated player row
// Column headers bold

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::draft::pick::Position;
use crate::tui::ViewState;
use crate::valuation::zscore::PlayerValuation;

/// Render the available players table into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState) {
    let filtered = filter_players(
        &state.available_players,
        state.position_filter.as_ref(),
        &state.filter_text,
    );

    let nominated_name = state
        .current_nomination
        .as_ref()
        .map(|n| n.player_name.as_str());

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

    let rows: Vec<Row> = filtered
        .iter()
        .enumerate()
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

    let title = build_title(state, filtered.len());

    let widths = [
        ratatui::layout::Constraint::Length(4),
        ratatui::layout::Constraint::Min(16),
        ratatui::layout::Constraint::Length(8),
        ratatui::layout::Constraint::Length(6),
        ratatui::layout::Constraint::Length(6),
        ratatui::layout::Constraint::Length(7),
    ];

    let _scroll_offset = state.scroll_offset.get("available").copied().unwrap_or(0);

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">> ");

    frame.render_widget(table, area);
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

/// Build the title with filter info and pre-computed count.
fn build_title(state: &ViewState, filtered_count: usize) -> Line<'static> {
    let mut title = String::from("Available Players");
    if let Some(ref pos) = state.position_filter {
        title.push_str(&format!(" [{}]", pos.display_str()));
    }
    if !state.filter_text.is_empty() {
        title.push_str(&format!(" \"{}\"", state.filter_text));
    }
    title.push_str(&format!(" ({})", filtered_count));
    Line::from(title)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::valuation::zscore::{
        CategoryZScores, HitterZScores, PlayerProjectionData,
    };

    fn make_test_player(name: &str, positions: Vec<Position>, dollar: f64) -> PlayerValuation {
        PlayerValuation {
            name: name.to_string(),
            team: "TST".to_string(),
            positions,
            is_pitcher: false,
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
            make_test_player("Player C", vec![Position::Catcher, Position::FirstBase], 10.0),
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

    #[test]
    fn format_positions_basic() {
        assert_eq!(
            format_positions(&[Position::Catcher]),
            "C"
        );
        assert_eq!(
            format_positions(&[Position::FirstBase, Position::ThirdBase]),
            "1B/3B"
        );
    }

    #[test]
    fn format_positions_empty() {
        assert_eq!(format_positions(&[]), "--");
    }

    #[test]
    fn render_does_not_panic_with_defaults() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_with_players() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = ViewState::default();
        state.available_players = vec![
            make_test_player("Player A", vec![Position::Catcher], 20.0),
            make_test_player("Player B", vec![Position::FirstBase], 15.0),
        ];
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
