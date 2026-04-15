// DailyStatsPanel: batting and pitching stat tables for a single scoring day.
//
// Renders two consecutive tables (batting then pitching) in a single scrollable
// view. Active players appear first, followed by a separator, then bench and IL
// players (dimmed). A TOTALS row at the bottom of each section shows aggregate
// stats in bold.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::matchup::{DailyPlayerRow, DailyTotals, ScoringDay, TeamDailyRoster};
use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};

// ---------------------------------------------------------------------------
// Column definitions
// ---------------------------------------------------------------------------

/// Fixed info columns (SLOT, Player, Team, Opp) that appear in every table.
const INFO_WIDTHS: &[(& str, usize)] = &[
    ("SLOT", 5),
    ("Player", 16),
    ("Team", 4),
    ("Opp", 5),
];

/// A dynamically-derived stat column built from the headers the extension sends.
struct StatCol {
    label: String,
    width: usize,
    precision: u8,
}

/// Derive display precision from a stat abbreviation.
/// Rate stats (AVG, OBP, SLG, OPS) get 3 decimals; ERA/WHIP/K9/BB9 get 2;
/// IP gets 1; everything else is an integer (0).
fn precision_for_stat(abbrev: &str) -> u8 {
    match abbrev {
        "AVG" | "OBP" | "SLG" | "OPS" => 3,
        "ERA" | "WHIP" | "K/9" | "BB/9" | "K/BB" => 2,
        "IP" => 1,
        _ => 0,
    }
}

/// Build `StatCol` entries from the header strings provided by the extension.
fn stat_cols_from_headers(headers: &[String]) -> Vec<StatCol> {
    headers
        .iter()
        .map(|h| {
            let precision = precision_for_stat(h);
            // Width: enough for the label + typical formatted values
            let min_val_width = if precision == 0 { 3 } else { precision as usize + 2 };
            let width = (h.len() + 1).max(min_val_width + 1).max(4);
            StatCol {
                label: h.clone(),
                width,
                precision,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// DailyStatsPanel
// ---------------------------------------------------------------------------

/// Displays batting and pitching stat tables for one day.
pub struct DailyStatsPanel {
    scroll: ScrollState,
}

/// Messages handled by the daily stats panel.
#[derive(Debug, Clone)]
pub enum DailyStatsPanelMessage {
    Scroll(ScrollDirection),
}

impl DailyStatsPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    pub fn update(&mut self, msg: DailyStatsPanelMessage) -> Option<Action> {
        match msg {
            DailyStatsPanelMessage::Scroll(dir) => {
                self.scroll.scroll(dir, 20);
                None
            }
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Render the daily stats for `day` into `area`.
    pub fn view(&self, frame: &mut Frame, area: Rect, day: &ScoringDay, _focused: bool) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        // Build all lines for the scrollable view.
        let lines = build_all_lines(day, area.width as usize);
        let content_height = lines.len();
        let viewport_height = area.height as usize;

        let offset = self.scroll.clamped_offset(content_height, viewport_height);

        // Render visible slice.
        for (i, line) in lines.iter().skip(offset).take(viewport_height).enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }
            let render_area = Rect::new(area.x, y, area.width, 1);
            frame.render_widget(line.clone(), render_area);
        }
    }

    /// Render a placeholder when no scoring day data is available.
    pub fn view_placeholder(&self, frame: &mut Frame, area: Rect) {
        use ratatui::widgets::{Block, Borders, Paragraph};
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Daily Stats ")
            .border_style(Style::default().fg(Color::DarkGray));
        let text = Paragraph::new(Line::from("Daily stats (waiting for data...)"))
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(text, area);
    }
}

impl Default for DailyStatsPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Line building
// ---------------------------------------------------------------------------

/// Build all output lines for the daily stats view.
///
/// Renders the away team first, then the home team. Each team gets a batting
/// and a pitching section with the same dynamic-column layout, separated by
/// blank lines.
fn build_all_lines(day: &ScoringDay, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let batting_cols = stat_cols_from_headers(&day.batting_stat_columns);
    let pitching_cols = stat_cols_from_headers(&day.pitching_stat_columns);

    build_team_sections(&mut lines, "Away", &day.label, &day.away, &batting_cols, &pitching_cols, width);
    lines.push(Line::default());
    build_team_sections(&mut lines, "Home", &day.label, &day.home, &batting_cols, &pitching_cols, width);

    lines
}

/// Build the batting + pitching sections for a single team.
#[allow(clippy::too_many_arguments)]
fn build_team_sections(
    lines: &mut Vec<Line<'static>>,
    team_label: &str,
    day_label: &str,
    roster: &TeamDailyRoster,
    batting_cols: &[StatCol],
    pitching_cols: &[StatCol],
    width: usize,
) {
    build_section(
        lines,
        &format!("{} {} Batting", team_label, day_label),
        &roster.batting_rows,
        roster.batting_totals.as_ref(),
        batting_cols,
        width,
    );

    // Gap between batting and pitching within a team block.
    lines.push(Line::default());

    build_section(
        lines,
        &format!("{} {} Pitching", team_label, day_label),
        &roster.pitching_rows,
        roster.pitching_totals.as_ref(),
        pitching_cols,
        width,
    );
}

/// Build lines for one section (batting or pitching).
fn build_section(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    rows: &[DailyPlayerRow],
    totals: Option<&DailyTotals>,
    stat_cols: &[StatCol],
    width: usize,
) {
    let header_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);

    // Section header: "── March 26 Batting ──────────..."
    let prefix = format!("── {} ", title);
    let fill_len = width.saturating_sub(prefix.len());
    let header_text = format!("{}{}", prefix, "─".repeat(fill_len));
    lines.push(Line::from(Span::styled(header_text, header_style)));

    // Column header row
    lines.push(build_header_line(stat_cols));

    // Partition into active vs bench/IL
    let (active, inactive): (Vec<_>, Vec<_>) = rows.iter().partition(|r| !is_inactive(r));

    // Active rows
    for row in &active {
        lines.push(build_player_line(row, stat_cols, false));
    }

    // Separator before bench/IL (if any)
    if !inactive.is_empty() {
        let sep_len = total_row_width(stat_cols);
        let sep = "─".repeat(sep_len.min(width));
        lines.push(Line::from(Span::styled(
            sep,
            Style::default().fg(Color::DarkGray),
        )));

        for row in &inactive {
            lines.push(build_player_line(row, stat_cols, true));
        }
    }

    // TOTALS row
    if let Some(t) = totals {
        lines.push(build_totals_line(t, stat_cols));
    }
}

/// Whether a player is bench or IL based on slot name.
fn is_inactive(row: &DailyPlayerRow) -> bool {
    let slot_upper = row.slot.to_uppercase();
    slot_upper == "BENCH" || slot_upper == "IL" || slot_upper.starts_with("IL")
}

/// Total width consumed by info + stat columns.
fn total_row_width(stat_cols: &[StatCol]) -> usize {
    let info_w: usize = INFO_WIDTHS.iter().map(|(_, w)| w + 1).sum();
    let stat_w: usize = stat_cols.iter().map(|c| c.width + 1).sum();
    info_w + stat_w
}

// ---------------------------------------------------------------------------
// Header line
// ---------------------------------------------------------------------------

fn build_header_line(stat_cols: &[StatCol]) -> Line<'static> {
    let style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    let mut parts = Vec::new();

    for &(label, width) in INFO_WIDTHS {
        parts.push(Span::styled(pad_left(label, width), style));
        parts.push(Span::raw(" "));
    }
    for col in stat_cols {
        parts.push(Span::styled(pad_right_align(&col.label, col.width), style));
        parts.push(Span::raw(" "));
    }

    Line::from(parts)
}

// ---------------------------------------------------------------------------
// Player line
// ---------------------------------------------------------------------------

fn build_player_line(
    row: &DailyPlayerRow,
    stat_cols: &[StatCol],
    dim: bool,
) -> Line<'static> {
    let has_game = row.opponent.is_some();
    let is_il = row.slot.to_uppercase() == "IL" || row.slot.to_uppercase().starts_with("IL");

    // Style: dim for bench/IL/no-game
    let text_style = if dim || !has_game {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };

    let mut parts = Vec::new();

    // Slot: show IL tag in red if applicable
    let slot_width = INFO_WIDTHS[0].1;
    if is_il {
        parts.push(Span::styled(
            pad_left(&row.slot, slot_width),
            Style::default().fg(Color::Red),
        ));
    } else {
        parts.push(Span::styled(
            pad_left(&row.slot, slot_width),
            text_style,
        ));
    }
    parts.push(Span::raw(" "));

    // Player name (truncate to column width)
    let name_width = INFO_WIDTHS[1].1;
    let name = truncate(&row.player_name, name_width);
    parts.push(Span::styled(
        pad_left(&name, name_width),
        text_style,
    ));
    parts.push(Span::raw(" "));

    // Team
    let team_width = INFO_WIDTHS[2].1;
    parts.push(Span::styled(
        pad_left(&row.team, team_width),
        text_style,
    ));
    parts.push(Span::raw(" "));

    // Opponent
    let opp_width = INFO_WIDTHS[3].1;
    let opp_display = row
        .opponent
        .as_deref()
        .unwrap_or("--");
    parts.push(Span::styled(
        pad_left(opp_display, opp_width),
        text_style,
    ));
    parts.push(Span::raw(" "));

    // Stats
    for (i, col) in stat_cols.iter().enumerate() {
        let val = row.stats.get(i).copied().flatten();
        let display = if !has_game {
            "--".to_string()
        } else {
            format_stat(val, col.precision)
        };
        parts.push(Span::styled(
            pad_right_align(&display, col.width),
            text_style,
        ));
        parts.push(Span::raw(" "));
    }

    Line::from(parts)
}

// ---------------------------------------------------------------------------
// Totals line
// ---------------------------------------------------------------------------

fn build_totals_line(
    totals: &DailyTotals,
    stat_cols: &[StatCol],
) -> Line<'static> {
    let bold = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);

    let mut parts = Vec::new();

    // "TOTALS" spans the info columns
    let info_width: usize = INFO_WIDTHS.iter().map(|(_, w)| w + 1).sum();
    let label = pad_left("TOTALS", info_width.saturating_sub(1));
    parts.push(Span::styled(label, bold));
    parts.push(Span::raw(" "));

    // Stat values
    for (i, col) in stat_cols.iter().enumerate() {
        let val = totals.stats.get(i).copied().flatten();
        let display = format_stat(val, col.precision);
        parts.push(Span::styled(
            pad_right_align(&display, col.width),
            bold,
        ));
        parts.push(Span::raw(" "));
    }

    Line::from(parts)
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Format a stat value using the column's display precision.
fn format_stat(val: Option<f64>, precision: u8) -> String {
    match val {
        None => "--".to_string(),
        Some(v) => {
            if precision == 0 {
                format!("{}", v as i64)
            } else {
                format!("{:.prec$}", v, prec = precision as usize)
            }
        }
    }
}

/// Left-align text in a field of given width (pad right with spaces).
fn pad_left(text: &str, width: usize) -> String {
    if text.len() >= width {
        text[..width].to_string()
    } else {
        format!("{:<width$}", text, width = width)
    }
}

/// Right-align text in a field of given width (pad left with spaces).
fn pad_right_align(text: &str, width: usize) -> String {
    if text.len() >= width {
        text[..width].to_string()
    } else {
        format!("{:>width$}", text, width = width)
    }
}

/// Truncate text to max_len characters.
fn truncate(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        text[..max_len].to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matchup::{DailyPlayerRow, DailyTotals, ScoringDay, TeamDailyRoster};

    fn make_active_batter(slot: &str, name: &str, opponent: Option<&str>) -> DailyPlayerRow {
        DailyPlayerRow {
            slot: slot.to_string(),
            player_name: name.to_string(),
            team: "NYY".to_string(),
            positions: vec!["C".to_string()],
            opponent: opponent.map(|s| s.to_string()),
            game_status: None,
            stats: vec![
                Some(4.0),  // AB
                Some(1.0),  // H
                Some(0.0),  // R
                Some(0.0),  // HR
                Some(1.0),  // RBI
                Some(0.0),  // BB
                Some(0.0),  // SB
                Some(0.250), // AVG
            ],
        }
    }

    fn make_bench_batter(name: &str, has_game: bool) -> DailyPlayerRow {
        DailyPlayerRow {
            slot: "BENCH".to_string(),
            player_name: name.to_string(),
            team: "MIL".to_string(),
            positions: vec!["LF".to_string()],
            opponent: if has_game {
                Some("@PIT".to_string())
            } else {
                None
            },
            game_status: None,
            stats: if has_game {
                vec![
                    Some(3.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.000),
                ]
            } else {
                vec![None; 8]
            },
        }
    }

    fn make_il_batter(name: &str) -> DailyPlayerRow {
        DailyPlayerRow {
            slot: "IL".to_string(),
            player_name: name.to_string(),
            team: "CHC".to_string(),
            positions: vec!["SS".to_string()],
            opponent: None,
            game_status: Some("60-day IL".to_string()),
            stats: vec![None; 8],
        }
    }

    fn make_active_pitcher(slot: &str, name: &str, opponent: &str) -> DailyPlayerRow {
        DailyPlayerRow {
            slot: slot.to_string(),
            player_name: name.to_string(),
            team: "HOU".to_string(),
            positions: vec!["SP".to_string()],
            opponent: Some(opponent.to_string()),
            game_status: None,
            stats: vec![
                Some(7.0),  // IP
                Some(4.0),  // H
                Some(2.0),  // ER
                Some(1.0),  // BB
                Some(8.0),  // K
                Some(1.0),  // W
                Some(0.0),  // SV
                Some(0.0),  // HD
            ],
        }
    }

    fn batting_headers() -> Vec<String> {
        ["AB", "H", "R", "HR", "RBI", "BB", "SB", "AVG"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn pitching_headers() -> Vec<String> {
        ["IP", "H", "ER", "BB", "K", "W", "SV", "HD"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn make_test_scoring_day() -> ScoringDay {
        // Home team: the full fixture with bench/IL players covering every
        // rendering branch. The content_height test anchors to this block.
        let home = TeamDailyRoster {
            batting_rows: vec![
                make_active_batter("C", "B. Rice", Some("@BOS")),
                make_active_batter("1B", "F. Freeman", Some("SD")),
                make_active_batter("3B", "A. Riley", None), // no game
                make_bench_batter("C. Yelich", false),       // bench, no game
                make_bench_batter("T. Grisham", true),       // bench, has game
                make_il_batter("D. Swanson"),                // IL
            ],
            pitching_rows: vec![
                make_active_pitcher("SP", "F. Valdez", "@TEX"),
                make_active_pitcher("RP", "L. Weaver", "@BOS"),
            ],
            batting_totals: Some(DailyTotals {
                stats: vec![
                    Some(29.0),
                    Some(8.0),
                    Some(5.0),
                    Some(2.0),
                    Some(6.0),
                    Some(5.0),
                    Some(1.0),
                    Some(0.276),
                ],
            }),
            pitching_totals: Some(DailyTotals {
                stats: vec![
                    Some(15.0),
                    Some(8.0),
                    Some(3.0),
                    Some(3.0),
                    Some(20.0),
                    Some(1.0),
                    Some(1.0),
                    Some(1.0),
                ],
            }),
        };
        // Away: a simpler one-batter-one-pitcher fixture, distinct from home.
        let away = TeamDailyRoster {
            batting_rows: vec![make_active_batter("1B", "P. Alonso", Some("@PHI"))],
            pitching_rows: vec![make_active_pitcher("SP", "C. Burnes", "LAD")],
            batting_totals: Some(DailyTotals {
                stats: vec![
                    Some(4.0), Some(2.0), Some(1.0), Some(1.0),
                    Some(2.0), Some(1.0), Some(0.0), Some(0.500),
                ],
            }),
            pitching_totals: Some(DailyTotals {
                stats: vec![
                    Some(6.0), Some(4.0), Some(2.0), Some(2.0),
                    Some(7.0), Some(1.0), Some(0.0), Some(0.0),
                ],
            }),
        };
        // Mirror the home roster into the legacy top-level fields while
        // both representations coexist. The rendering paths under test read
        // from `home`/`away`; the clones exist only so the `ScoringDay`
        // initializer is still complete.
        let batting_rows = home.batting_rows.clone();
        let pitching_rows = home.pitching_rows.clone();
        let batting_totals = home.batting_totals.clone();
        let pitching_totals = home.pitching_totals.clone();
        ScoringDay {
            date: "2026-03-26".to_string(),
            label: "March 26".to_string(),
            batting_stat_columns: batting_headers(),
            pitching_stat_columns: pitching_headers(),
            batting_rows,
            pitching_rows,
            batting_totals,
            pitching_totals,
            home,
            away,
        }
    }

    // -- Rendering tests --

    #[test]
    fn view_renders_without_panic() {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = DailyStatsPanel::new();
        let day = make_test_scoring_day();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &day, false))
            .unwrap();
    }

    #[test]
    fn view_renders_narrow_terminal() {
        let backend = ratatui::backend::TestBackend::new(60, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = DailyStatsPanel::new();
        let day = make_test_scoring_day();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &day, false))
            .unwrap();
    }

    #[test]
    fn view_renders_empty_day() {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = DailyStatsPanel::new();
        let day = ScoringDay {
            date: "2026-03-26".to_string(),
            label: "March 26".to_string(),
            batting_stat_columns: vec![],
            pitching_stat_columns: vec![],
            batting_rows: Vec::new(),
            pitching_rows: Vec::new(),
            batting_totals: None,
            pitching_totals: None,
            home: TeamDailyRoster::default(),
            away: TeamDailyRoster::default(),
        };
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &day, false))
            .unwrap();
    }

    // -- Content height and scroll --

    #[test]
    fn content_height_matches_expected_lines() {
        let day = make_test_scoring_day();
        let lines = build_all_lines(&day, 120);
        // Away block (one-row batter + one-row pitcher, each with totals):
        //   Batting: header(1) + col_header(1) + active(1) + totals(1) = 4
        //   Gap: 1
        //   Pitching: header(1) + col_header(1) + active(1) + totals(1) = 4
        // Gap between teams: 1
        // Home block:
        //   Batting: header(1) + col_header(1) + active(3) + separator(1) + bench(2) + IL(1) + totals(1) = 10
        //   Gap: 1
        //   Pitching: header(1) + col_header(1) + active(2) + totals(1) = 5
        // Total: (4 + 1 + 4) + 1 + (10 + 1 + 5) = 9 + 1 + 16 = 26
        assert_eq!(lines.len(), 26);
    }

    #[test]
    fn scroll_bounds_clamped() {
        let mut panel = DailyStatsPanel::new();
        // Scroll well past content
        for _ in 0..50 {
            panel.update(DailyStatsPanelMessage::Scroll(ScrollDirection::Down));
        }
        let day = make_test_scoring_day();
        let lines = build_all_lines(&day, 120);
        let content_height = lines.len();
        let viewport = 10_usize;
        let clamped = panel.scroll.clamped_offset(content_height, viewport);
        assert!(clamped <= content_height.saturating_sub(viewport));
    }

    // -- Formatting tests --

    #[test]
    fn format_stat_rate_precision_3() {
        assert_eq!(format_stat(Some(0.276), 3), "0.276");
        assert_eq!(format_stat(Some(0.0), 3), "0.000");
        assert_eq!(format_stat(Some(1.0), 3), "1.000");
    }

    #[test]
    fn format_stat_precision_1() {
        assert_eq!(format_stat(Some(7.0), 1), "7.0");
        assert_eq!(format_stat(Some(0.1), 1), "0.1");
    }

    #[test]
    fn format_stat_counting_integer() {
        assert_eq!(format_stat(Some(4.0), 0), "4");
        assert_eq!(format_stat(Some(0.0), 0), "0");
        assert_eq!(format_stat(Some(8.0), 0), "8");
    }

    #[test]
    fn format_stat_none_shows_dashes() {
        assert_eq!(format_stat(None, 0), "--");
        assert_eq!(format_stat(None, 3), "--");
        assert_eq!(format_stat(None, 1), "--");
    }

    #[test]
    fn precision_for_known_stats() {
        assert_eq!(precision_for_stat("AVG"), 3);
        assert_eq!(precision_for_stat("OBP"), 3);
        assert_eq!(precision_for_stat("ERA"), 2);
        assert_eq!(precision_for_stat("WHIP"), 2);
        assert_eq!(precision_for_stat("IP"), 1);
        assert_eq!(precision_for_stat("HR"), 0);
        assert_eq!(precision_for_stat("K"), 0);
        assert_eq!(precision_for_stat("AB"), 0);
    }

    // -- Inactive detection --

    #[test]
    fn bench_player_detected_as_inactive() {
        let row = make_bench_batter("Test", false);
        assert!(is_inactive(&row));
    }

    #[test]
    fn il_player_detected_as_inactive() {
        let row = make_il_batter("Test");
        assert!(is_inactive(&row));
    }

    #[test]
    fn active_player_not_inactive() {
        let row = make_active_batter("C", "Test", Some("@BOS"));
        assert!(!is_inactive(&row));
    }

    // -- No-game player shows dashes --

    #[test]
    fn no_game_player_shows_dashes_in_line() {
        let row = make_active_batter("3B", "A. Riley", None);
        let cols = stat_cols_from_headers(&batting_headers());
        let line = build_player_line(&row, &cols, false);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // Should contain "--" for opponent and all stat columns
        assert!(text.contains("--"));
    }

    // -- Bench player stats with game show values --

    #[test]
    fn bench_player_with_game_shows_stats() {
        let row = make_bench_batter("T. Grisham", true);
        let cols = stat_cols_from_headers(&batting_headers());
        let line = build_player_line(&row, &cols, true);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // Should contain stat values, not "--" for stats
        assert!(text.contains("3")); // AB = 3
        assert!(text.contains("0.000")); // AVG = .000
    }

    // -- Dynamic column tests --

    #[test]
    fn stat_cols_from_headers_computes_widths() {
        let cols = stat_cols_from_headers(&["AB".to_string(), "AVG".to_string()]);
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].precision, 0);
        assert_eq!(cols[1].precision, 3);
        assert!(cols[0].width >= 4); // minimum width for counting stat
        assert!(cols[1].width >= 5); // wider for rate stat
    }

    #[test]
    fn empty_stat_columns_renders_without_panic() {
        let day = ScoringDay {
            date: "2026-03-26".to_string(),
            label: "March 26".to_string(),
            batting_stat_columns: vec![],
            pitching_stat_columns: vec![],
            batting_rows: vec![make_active_batter("C", "B. Rice", Some("@BOS"))],
            pitching_rows: vec![],
            batting_totals: None,
            pitching_totals: None,
            home: TeamDailyRoster {
                batting_rows: vec![make_active_batter("C", "B. Rice", Some("@BOS"))],
                ..TeamDailyRoster::default()
            },
            away: TeamDailyRoster::default(),
        };
        let lines = build_all_lines(&day, 120);
        // Should still have section headers for both teams + col headers + player row + gaps
        assert!(lines.len() >= 4);
    }
}
