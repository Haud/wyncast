// RosterViewPanel: aggregated roster stats across the entire scoring period.
//
// Reused for both the Home Roster (Tab 3) and Away Roster (Tab 4) — the
// same component renders different data based on the `TeamSide` parameter.
// Aggregates counting stats across all ScoringDay entries and computes rate
// stats (AVG, ERA, WHIP) from components.

use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use std::collections::BTreeMap;

use crate::matchup::{ScoringDay, TeamSide};
use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::widgets::focused_border_style;

/// Page size for PageUp/PageDown scrolling.
const PAGE_SIZE: usize = 20;

// ---------------------------------------------------------------------------
// Dynamic stat header lookup
// ---------------------------------------------------------------------------

/// Resolves a stat name to its index within a header list sent by the extension.
/// Returns `None` if the stat is not present in the headers (case-insensitive).
fn find_header_index(headers: &[String], name: &str) -> Option<usize> {
    headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case(name))
}

/// Look up a stat value from a player's stats vec using the header-derived index.
fn stat_by_header(stats: &[Option<f64>], headers: &[String], name: &str) -> Option<f64> {
    find_header_index(headers, name)
        .and_then(|idx| stats.get(idx).copied().flatten())
}

// ---------------------------------------------------------------------------
// Aggregated player data
// ---------------------------------------------------------------------------

/// Aggregated batting stats for one player across the scoring period.
#[derive(Debug, Clone)]
pub struct AggregatedBatter {
    pub slot: String,
    pub player_name: String,
    pub team: String,
    pub positions: Vec<String>,
    pub gp: u16,
    pub ab: f64,
    pub h: f64,
    pub r: f64,
    pub hr: f64,
    pub rbi: f64,
    pub bb: f64,
    pub sb: f64,
}

impl AggregatedBatter {
    /// Batting average: H / AB (None if AB == 0).
    pub fn avg(&self) -> Option<f64> {
        if self.ab > 0.0 {
            Some(self.h / self.ab)
        } else {
            None
        }
    }
}

/// Aggregated pitching stats for one player across the scoring period.
#[derive(Debug, Clone)]
pub struct AggregatedPitcher {
    pub slot: String,
    pub player_name: String,
    pub team: String,
    pub positions: Vec<String>,
    pub gs: u16,
    pub ip: f64,
    pub h: f64,
    pub er: f64,
    pub bb: f64,
    pub k: f64,
    pub w: f64,
    pub sv: f64,
    pub hd: f64,
}

impl AggregatedPitcher {
    /// ERA: (ER / IP) * 9 (None if IP == 0).
    pub fn era(&self) -> Option<f64> {
        if self.ip > 0.0 {
            Some((self.er / self.ip) * 9.0)
        } else {
            None
        }
    }

    /// WHIP: (H + BB) / IP (None if IP == 0).
    pub fn whip(&self) -> Option<f64> {
        if self.ip > 0.0 {
            Some((self.h + self.bb) / self.ip)
        } else {
            None
        }
    }
}

/// Aggregated results for display.
pub struct AggregatedRoster {
    pub batters: Vec<AggregatedBatter>,
    pub pitchers: Vec<AggregatedPitcher>,
}

// ---------------------------------------------------------------------------
// Aggregation logic
// ---------------------------------------------------------------------------

/// Aggregate batting stats across all scoring days for the given team side.
///
/// Players are identified by name. The first occurrence sets slot/team/positions.
/// Counting stats are summed. GP counts days where the player had an opponent
/// and was not on the bench. Stat indices are resolved dynamically from the
/// `batting_stat_columns` sent by the extension.
pub fn aggregate_batters(days: &[ScoringDay], side: TeamSide) -> Vec<AggregatedBatter> {
    let mut order: Vec<String> = Vec::new();
    let mut map: BTreeMap<String, AggregatedBatter> = BTreeMap::new();

    for day in days {
        let headers = &day.batting_stat_columns;

        for row in &day.roster(side).batting_rows {
            let entry = map.entry(row.player_name.clone()).or_insert_with(|| {
                order.push(row.player_name.clone());
                AggregatedBatter {
                    slot: row.slot.clone(),
                    player_name: row.player_name.clone(),
                    team: row.team.clone(),
                    positions: row.positions.clone(),
                    gp: 0,
                    ab: 0.0,
                    h: 0.0,
                    r: 0.0,
                    hr: 0.0,
                    rbi: 0.0,
                    bb: 0.0,
                    sb: 0.0,
                }
            });

            // A player "played" if they had an opponent and were not on the bench
            let is_bench = row.slot.eq_ignore_ascii_case("bench")
                || row.slot.eq_ignore_ascii_case("il");
            let has_game = row.opponent.is_some() && !is_bench;

            if has_game {
                entry.gp += 1;
            }

            // Sum counting stats using header-driven indices
            if let Some(v) = stat_by_header(&row.stats, headers, "AB") {
                entry.ab += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "H") {
                entry.h += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "R") {
                entry.r += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "HR") {
                entry.hr += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "RBI") {
                entry.rbi += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "BB") {
                entry.bb += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "SB") {
                entry.sb += v;
            }
            // AVG is ignored; we recompute from H/AB.
        }
    }

    // Return in insertion order (preserves roster slot ordering from first day)
    order
        .iter()
        .filter_map(|name| map.remove(name))
        .collect()
}

/// Aggregate pitching stats across all scoring days for the given team side.
///
/// GS counts days where the pitcher's slot was "SP" and they had a game.
/// Stat indices are resolved dynamically from the `pitching_stat_columns`
/// sent by the extension.
pub fn aggregate_pitchers(days: &[ScoringDay], side: TeamSide) -> Vec<AggregatedPitcher> {
    let mut order: Vec<String> = Vec::new();
    let mut map: BTreeMap<String, AggregatedPitcher> = BTreeMap::new();

    for day in days {
        let headers = &day.pitching_stat_columns;

        for row in &day.roster(side).pitching_rows {
            let entry = map.entry(row.player_name.clone()).or_insert_with(|| {
                order.push(row.player_name.clone());
                AggregatedPitcher {
                    slot: row.slot.clone(),
                    player_name: row.player_name.clone(),
                    team: row.team.clone(),
                    positions: row.positions.clone(),
                    gs: 0,
                    ip: 0.0,
                    h: 0.0,
                    er: 0.0,
                    bb: 0.0,
                    k: 0.0,
                    w: 0.0,
                    sv: 0.0,
                    hd: 0.0,
                }
            });

            // GS: pitcher slot is SP and had a game (opponent present, not bench)
            let is_bench = row.slot.eq_ignore_ascii_case("bench")
                || row.slot.eq_ignore_ascii_case("il");
            let is_sp = row.slot.eq_ignore_ascii_case("sp");
            let has_game = row.opponent.is_some() && !is_bench;

            if is_sp && has_game {
                entry.gs += 1;
            }

            // Sum counting stats using header-driven indices
            if let Some(v) = stat_by_header(&row.stats, headers, "IP") {
                entry.ip += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "H") {
                entry.h += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "ER") {
                entry.er += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "BB") {
                entry.bb += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "K") {
                entry.k += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "W") {
                entry.w += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "SV") {
                entry.sv += v;
            }
            if let Some(v) = stat_by_header(&row.stats, headers, "HD") {
                entry.hd += v;
            }
        }
    }

    order
        .iter()
        .filter_map(|name| map.remove(name))
        .collect()
}

/// Build complete aggregated roster from scoring days for the given team side.
pub fn aggregate_roster(days: &[ScoringDay], side: TeamSide) -> AggregatedRoster {
    AggregatedRoster {
        batters: aggregate_batters(days, side),
        pitchers: aggregate_pitchers(days, side),
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Format a counting stat as an integer string.
fn fmt_count(v: f64) -> String {
    format!("{}", v as i64)
}

/// Format AVG as ".XXX" (3 decimal places, baseball convention drops leading zero).
fn fmt_avg(avg: Option<f64>) -> String {
    match avg {
        Some(v) if v >= 1.0 => format!("{:.3}", v),
        Some(v) => {
            let s = format!("{:.3}", v);
            // Strip leading "0" for values < 1.0 (e.g., "0.286" -> ".286")
            s.strip_prefix('0').unwrap_or(&s).to_string()
        }
        None => "--".to_string(),
    }
}

/// Format ERA as "X.XX" (2 decimal places).
fn fmt_era(era: Option<f64>) -> String {
    match era {
        Some(v) => format!("{:.2}", v),
        None => "--".to_string(),
    }
}

/// Format WHIP as "X.XX" (2 decimal places).
fn fmt_whip(whip: Option<f64>) -> String {
    match whip {
        Some(v) => format!("{:.2}", v),
        None => "--".to_string(),
    }
}

/// Format IP to one decimal (ESPN uses x.1 = 1/3, x.2 = 2/3 convention).
fn fmt_ip(ip: f64) -> String {
    if ip == 0.0 {
        "0.0".to_string()
    } else {
        format!("{:.1}", ip)
    }
}

/// Format positions as comma-separated string.
fn fmt_positions(positions: &[String]) -> String {
    if positions.is_empty() {
        "--".to_string()
    } else {
        positions.join(",")
    }
}

// ---------------------------------------------------------------------------
// RosterViewPanel
// ---------------------------------------------------------------------------

/// Message type for the roster view panel.
#[derive(Debug, Clone)]
pub enum RosterViewPanelMessage {
    Scroll(ScrollDirection),
}

/// Roster view panel: aggregated stats for an entire scoring period.
///
/// Reused for both the Home Roster and Away Roster tabs.
pub struct RosterViewPanel {
    scroll: ScrollState,
}

impl RosterViewPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    pub fn update(&mut self, msg: RosterViewPanelMessage) -> Option<Action> {
        match msg {
            RosterViewPanelMessage::Scroll(dir) => {
                self.scroll.scroll(dir, PAGE_SIZE);
                None
            }
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Render the roster view panel.
    ///
    /// `team_name` is displayed in the title.
    /// `days` contains all scoring period days to aggregate.
    /// `side` selects which team's rosters to aggregate (Home or Away).
    pub fn view(
        &self,
        frame: &mut Frame,
        area: Rect,
        team_name: &str,
        days: &[ScoringDay],
        side: TeamSide,
        focused: bool,
    ) {
        let roster = aggregate_roster(days, side);
        let lines = build_roster_lines(&roster);

        // Total content lines for scroll calculation
        let content_height = lines.len();
        // Usable viewport: area height minus 2 for borders
        let viewport_height = (area.height as usize).saturating_sub(2);
        let scroll_offset = self.scroll.clamped_offset(content_height, viewport_height);

        let title = format!(" {} - Full Roster ", team_name);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(focused_border_style(focused, Style::default()))
            .title(title);

        // Slice visible lines
        let visible: Vec<Line> = lines
            .into_iter()
            .skip(scroll_offset)
            .take(viewport_height.max(1))
            .collect();

        let paragraph = ratatui::widgets::Paragraph::new(visible).block(block);
        frame.render_widget(paragraph, area);

        // Scrollbar when content overflows
        if content_height > viewport_height {
            let mut scrollbar_state =
                ScrollbarState::new(content_height.saturating_sub(viewport_height))
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

impl Default for RosterViewPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Line building
// ---------------------------------------------------------------------------

/// Build all display lines for the roster (batting + pitching sections).
fn build_roster_lines(roster: &AggregatedRoster) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // -- Batting section --
    if !roster.batters.is_empty() {
        // Header
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {:<6}{:<17}{:<6}{:<12}{:>3}{:>5}{:>4}{:>4}{:>4}{:>5}{:>4}{:>4}{:>6}",
                "SLOT", "Player", "Team", "Pos", "GP", "AB", "H", "R", "HR", "RBI", "BB", "SB", "AVG"
            ),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]));

        // Separator
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {:<6}{:<17}{:<6}{:<12}{:>3}{:>5}{:>4}{:>4}{:>4}{:>5}{:>4}{:>4}{:>6}",
                "────", "──────", "────", "───", "──", "──", "──", "──", "──", "───", "──", "──", "────"
            ),
            Style::default().fg(Color::DarkGray),
        )]));

        // Player rows
        for b in &roster.batters {
            let line = format!(
                "  {:<6}{:<17}{:<6}{:<12}{:>3}{:>5}{:>4}{:>4}{:>4}{:>5}{:>4}{:>4}{:>6}",
                b.slot,
                truncate(&b.player_name, 16),
                &b.team,
                truncate(&fmt_positions(&b.positions), 11),
                b.gp,
                fmt_count(b.ab),
                fmt_count(b.h),
                fmt_count(b.r),
                fmt_count(b.hr),
                fmt_count(b.rbi),
                fmt_count(b.bb),
                fmt_count(b.sb),
                fmt_avg(b.avg()),
            );
            lines.push(Line::from(line));
        }

        // Totals row
        let totals = batting_totals(&roster.batters);
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {:<6}{:<17}{:<6}{:<12}{:>3}{:>5}{:>4}{:>4}{:>4}{:>5}{:>4}{:>4}{:>6}",
                "TOTAL", "", "", "",
                "",
                fmt_count(totals.ab),
                fmt_count(totals.h),
                fmt_count(totals.r),
                fmt_count(totals.hr),
                fmt_count(totals.rbi),
                fmt_count(totals.bb),
                fmt_count(totals.sb),
                fmt_avg(totals.avg()),
            ),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]));

        // Blank line between sections
        lines.push(Line::from(""));
    }

    // -- Pitching section --
    if !roster.pitchers.is_empty() {
        // Header
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {:<6}{:<17}{:<6}{:<12}{:>3}{:>6}{:>4}{:>4}{:>4}{:>4}{:>4}{:>4}{:>4}{:>6}{:>6}",
                "SLOT", "Player", "Team", "Pos", "GS", "IP", "H", "ER", "BB", "K", "W", "SV", "HD", "ERA", "WHIP"
            ),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]));

        // Separator
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {:<6}{:<17}{:<6}{:<12}{:>3}{:>6}{:>4}{:>4}{:>4}{:>4}{:>4}{:>4}{:>4}{:>6}{:>6}",
                "────", "──────", "────", "───", "──", "───", "──", "──", "──", "──", "──", "──", "──", "────", "────"
            ),
            Style::default().fg(Color::DarkGray),
        )]));

        // Player rows
        for p in &roster.pitchers {
            let line = format!(
                "  {:<6}{:<17}{:<6}{:<12}{:>3}{:>6}{:>4}{:>4}{:>4}{:>4}{:>4}{:>4}{:>4}{:>6}{:>6}",
                p.slot,
                truncate(&p.player_name, 16),
                &p.team,
                truncate(&fmt_positions(&p.positions), 11),
                p.gs,
                fmt_ip(p.ip),
                fmt_count(p.h),
                fmt_count(p.er),
                fmt_count(p.bb),
                fmt_count(p.k),
                fmt_count(p.w),
                fmt_count(p.sv),
                fmt_count(p.hd),
                fmt_era(p.era()),
                fmt_whip(p.whip()),
            );
            lines.push(Line::from(line));
        }

        // Totals row
        let totals = pitching_totals(&roster.pitchers);
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {:<6}{:<17}{:<6}{:<12}{:>3}{:>6}{:>4}{:>4}{:>4}{:>4}{:>4}{:>4}{:>4}{:>6}{:>6}",
                "TOTAL", "", "", "",
                "",
                fmt_ip(totals.ip),
                fmt_count(totals.h),
                fmt_count(totals.er),
                fmt_count(totals.bb),
                fmt_count(totals.k),
                fmt_count(totals.w),
                fmt_count(totals.sv),
                fmt_count(totals.hd),
                fmt_era(totals.era()),
                fmt_whip(totals.whip()),
            ),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No roster data available",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines
}

/// Compute batting totals across all batters.
fn batting_totals(batters: &[AggregatedBatter]) -> AggregatedBatter {
    let mut total = AggregatedBatter {
        slot: String::new(),
        player_name: String::new(),
        team: String::new(),
        positions: Vec::new(),
        gp: 0,
        ab: 0.0,
        h: 0.0,
        r: 0.0,
        hr: 0.0,
        rbi: 0.0,
        bb: 0.0,
        sb: 0.0,
    };
    for b in batters {
        total.ab += b.ab;
        total.h += b.h;
        total.r += b.r;
        total.hr += b.hr;
        total.rbi += b.rbi;
        total.bb += b.bb;
        total.sb += b.sb;
    }
    total
}

/// Compute pitching totals across all pitchers.
fn pitching_totals(pitchers: &[AggregatedPitcher]) -> AggregatedPitcher {
    let mut total = AggregatedPitcher {
        slot: String::new(),
        player_name: String::new(),
        team: String::new(),
        positions: Vec::new(),
        gs: 0,
        ip: 0.0,
        h: 0.0,
        er: 0.0,
        bb: 0.0,
        k: 0.0,
        w: 0.0,
        sv: 0.0,
        hd: 0.0,
    };
    for p in pitchers {
        total.ip += p.ip;
        total.h += p.h;
        total.er += p.er;
        total.bb += p.bb;
        total.k += p.k;
        total.w += p.w;
        total.sv += p.sv;
        total.hd += p.hd;
    }
    total
}

/// Truncate a string to `max_len` chars, appending "…" if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut result: String = s.chars().take(max_len - 1).collect();
        result.push('…');
        result
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matchup::{DailyPlayerRow, ScoringDay, TeamDailyRoster};

    // -- Test helpers --

    fn make_batting_row(
        slot: &str,
        name: &str,
        team: &str,
        positions: Vec<&str>,
        opponent: Option<&str>,
        stats: Vec<Option<f64>>,
    ) -> DailyPlayerRow {
        DailyPlayerRow {
            slot: slot.to_string(),
            player_name: name.to_string(),
            team: team.to_string(),
            positions: positions.into_iter().map(String::from).collect(),
            opponent: opponent.map(String::from),
            game_status: None,
            stats,
        }
    }

    fn make_pitching_row(
        slot: &str,
        name: &str,
        team: &str,
        positions: Vec<&str>,
        opponent: Option<&str>,
        stats: Vec<Option<f64>>,
    ) -> DailyPlayerRow {
        DailyPlayerRow {
            slot: slot.to_string(),
            player_name: name.to_string(),
            team: team.to_string(),
            positions: positions.into_iter().map(String::from).collect(),
            opponent: opponent.map(String::from),
            game_status: None,
            stats,
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

    /// Build a day with batting/pitching rows placed on the Home side.
    /// Away side is empty. Tests default to `TeamSide::Home` unless otherwise
    /// noted.
    fn make_day(
        label: &str,
        batting: Vec<DailyPlayerRow>,
        pitching: Vec<DailyPlayerRow>,
    ) -> ScoringDay {
        ScoringDay {
            date: "2026-03-26".to_string(),
            label: label.to_string(),
            batting_stat_columns: batting_headers(),
            pitching_stat_columns: pitching_headers(),
            home: TeamDailyRoster {
                batting_rows: batting,
                pitching_rows: pitching,
                batting_totals: None,
                pitching_totals: None,
            },
            away: TeamDailyRoster::default(),
        }
    }

    // -- Batting aggregation tests --

    #[test]
    fn aggregate_batters_sums_counting_stats_across_days() {
        let days = vec![
            make_day(
                "Day 1",
                vec![make_batting_row(
                    "C", "B. Rice", "NYY", vec!["1B", "C", "DH"],
                    Some("@BOS"),
                    // AB, H, R, HR, RBI, BB, SB, AVG
                    vec![Some(4.0), Some(1.0), Some(1.0), Some(0.0), Some(2.0), Some(1.0), Some(0.0), Some(0.250)],
                )],
                vec![],
            ),
            make_day(
                "Day 2",
                vec![make_batting_row(
                    "C", "B. Rice", "NYY", vec!["1B", "C", "DH"],
                    Some("TB"),
                    vec![Some(3.0), Some(1.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.333)],
                )],
                vec![],
            ),
        ];

        let batters = aggregate_batters(&days, TeamSide::Home);
        assert_eq!(batters.len(), 1);

        let rice = &batters[0];
        assert_eq!(rice.player_name, "B. Rice");
        assert_eq!(rice.gp, 2);
        assert_eq!(rice.ab, 7.0);
        assert_eq!(rice.h, 2.0);
        assert_eq!(rice.r, 1.0);
        assert_eq!(rice.hr, 0.0);
        assert_eq!(rice.rbi, 2.0);
        assert_eq!(rice.bb, 1.0);
        assert_eq!(rice.sb, 0.0);
        // AVG = 2/7 ≈ 0.286
        let avg = rice.avg().unwrap();
        assert!((avg - 0.2857).abs() < 0.001);
    }

    #[test]
    fn aggregate_batters_multiple_players_preserves_order() {
        let days = vec![make_day(
            "Day 1",
            vec![
                make_batting_row(
                    "C", "Player A", "NYY", vec!["C"],
                    Some("@BOS"),
                    vec![Some(4.0), Some(1.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.250)],
                ),
                make_batting_row(
                    "1B", "Player B", "LAD", vec!["1B"],
                    Some("SD"),
                    vec![Some(3.0), Some(2.0), Some(1.0), Some(1.0), Some(3.0), Some(1.0), Some(0.0), Some(0.667)],
                ),
            ],
            vec![],
        )];

        let batters = aggregate_batters(&days, TeamSide::Home);
        assert_eq!(batters.len(), 2);
        assert_eq!(batters[0].player_name, "Player A");
        assert_eq!(batters[1].player_name, "Player B");
    }

    #[test]
    fn aggregate_batters_bench_player_zero_gp() {
        let days = vec![make_day(
            "Day 1",
            vec![make_batting_row(
                "BENCH", "Bench Guy", "MIL", vec!["LF"],
                Some("@PIT"),
                vec![Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), None],
            )],
            vec![],
        )];

        let batters = aggregate_batters(&days, TeamSide::Home);
        assert_eq!(batters.len(), 1);
        assert_eq!(batters[0].gp, 0); // Bench players don't count as GP
    }

    #[test]
    fn aggregate_batters_no_game_day_zero_gp() {
        let days = vec![make_day(
            "Day 1",
            vec![make_batting_row(
                "3B", "A. Riley", "ATL", vec!["3B"],
                None, // no game
                vec![None, None, None, None, None, None, None, None],
            )],
            vec![],
        )];

        let batters = aggregate_batters(&days, TeamSide::Home);
        assert_eq!(batters.len(), 1);
        assert_eq!(batters[0].gp, 0);
        assert_eq!(batters[0].ab, 0.0);
    }

    #[test]
    fn aggregate_batters_avg_zero_ab_returns_none() {
        let batter = AggregatedBatter {
            slot: "3B".to_string(),
            player_name: "A. Riley".to_string(),
            team: "ATL".to_string(),
            positions: vec!["3B".to_string()],
            gp: 0,
            ab: 0.0,
            h: 0.0,
            r: 0.0,
            hr: 0.0,
            rbi: 0.0,
            bb: 0.0,
            sb: 0.0,
        };
        assert!(batter.avg().is_none());
    }

    #[test]
    fn aggregate_batters_il_player_zero_gp() {
        let days = vec![make_day(
            "Day 1",
            vec![make_batting_row(
                "IL", "Hurt Guy", "BOS", vec!["SS"],
                Some("NYY"),
                vec![Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), None],
            )],
            vec![],
        )];

        let batters = aggregate_batters(&days, TeamSide::Home);
        assert_eq!(batters[0].gp, 0);
    }

    // -- Pitching aggregation tests --

    #[test]
    fn aggregate_pitchers_sums_counting_stats() {
        let days = vec![
            make_day(
                "Day 1",
                vec![],
                vec![make_pitching_row(
                    "SP", "F. Valdez", "HOU", vec!["SP"],
                    Some("@TEX"),
                    // IP, H, ER, BB, K, W, SV, HD
                    vec![Some(7.0), Some(4.0), Some(2.0), Some(1.0), Some(8.0), Some(1.0), Some(0.0), Some(0.0)],
                )],
            ),
            make_day(
                "Day 2",
                vec![],
                vec![make_pitching_row(
                    "SP", "F. Valdez", "HOU", vec!["SP"],
                    Some("LAA"),
                    vec![Some(6.0), Some(3.0), Some(1.0), Some(2.0), Some(9.0), Some(0.0), Some(0.0), Some(0.0)],
                )],
            ),
        ];

        let pitchers = aggregate_pitchers(&days, TeamSide::Home);
        assert_eq!(pitchers.len(), 1);

        let valdez = &pitchers[0];
        assert_eq!(valdez.player_name, "F. Valdez");
        assert_eq!(valdez.gs, 2);
        assert_eq!(valdez.ip, 13.0);
        assert_eq!(valdez.h, 7.0);
        assert_eq!(valdez.er, 3.0);
        assert_eq!(valdez.bb, 3.0);
        assert_eq!(valdez.k, 17.0);
        assert_eq!(valdez.w, 1.0);
        assert_eq!(valdez.sv, 0.0);
        assert_eq!(valdez.hd, 0.0);
        // ERA = (3/13) * 9 ≈ 2.08
        let era = valdez.era().unwrap();
        assert!((era - 2.077).abs() < 0.01);
        // WHIP = (7+3)/13 ≈ 0.769
        let whip = valdez.whip().unwrap();
        assert!((whip - 0.769).abs() < 0.01);
    }

    #[test]
    fn aggregate_pitchers_rp_zero_gs() {
        let days = vec![make_day(
            "Day 1",
            vec![],
            vec![make_pitching_row(
                "RP", "L. Weaver", "NYY", vec!["RP"],
                Some("@BOS"),
                vec![Some(1.0), Some(0.0), Some(0.0), Some(0.0), Some(2.0), Some(0.0), Some(1.0), Some(0.0)],
            )],
        )];

        let pitchers = aggregate_pitchers(&days, TeamSide::Home);
        assert_eq!(pitchers.len(), 1);
        assert_eq!(pitchers[0].gs, 0); // RP slot doesn't count as GS
        assert_eq!(pitchers[0].sv, 1.0);
    }

    #[test]
    fn aggregate_pitchers_era_zero_ip_returns_none() {
        let pitcher = AggregatedPitcher {
            slot: "SP".to_string(),
            player_name: "No Start".to_string(),
            team: "TST".to_string(),
            positions: vec!["SP".to_string()],
            gs: 0,
            ip: 0.0,
            h: 0.0,
            er: 0.0,
            bb: 0.0,
            k: 0.0,
            w: 0.0,
            sv: 0.0,
            hd: 0.0,
        };
        assert!(pitcher.era().is_none());
        assert!(pitcher.whip().is_none());
    }

    #[test]
    fn aggregate_pitchers_bench_pitcher_zero_gs() {
        let days = vec![make_day(
            "Day 1",
            vec![],
            vec![make_pitching_row(
                "BENCH", "B. Woo", "SEA", vec!["SP"],
                None,
                vec![None, None, None, None, None, None, None, None],
            )],
        )];

        let pitchers = aggregate_pitchers(&days, TeamSide::Home);
        assert_eq!(pitchers[0].gs, 0);
        assert_eq!(pitchers[0].ip, 0.0);
    }

    // -- Rate stat computation tests --

    #[test]
    fn avg_computation() {
        let batter = AggregatedBatter {
            slot: "1B".to_string(),
            player_name: "F. Freeman".to_string(),
            team: "LAD".to_string(),
            positions: vec!["1B".to_string()],
            gp: 2,
            ab: 7.0,
            h: 4.0,
            r: 2.0,
            hr: 1.0,
            rbi: 3.0,
            bb: 1.0,
            sb: 0.0,
        };
        let avg = batter.avg().unwrap();
        assert!((avg - 0.5714).abs() < 0.001);
    }

    #[test]
    fn era_computation() {
        let pitcher = AggregatedPitcher {
            slot: "SP".to_string(),
            player_name: "T. Glasnow".to_string(),
            team: "LAD".to_string(),
            positions: vec!["SP".to_string()],
            gs: 1,
            ip: 6.0,
            h: 3.0,
            er: 1.0,
            bb: 2.0,
            k: 9.0,
            w: 0.0,
            sv: 0.0,
            hd: 0.0,
        };
        let era = pitcher.era().unwrap();
        assert!((era - 1.50).abs() < 0.01);
    }

    #[test]
    fn whip_computation() {
        let pitcher = AggregatedPitcher {
            slot: "SP".to_string(),
            player_name: "T. Glasnow".to_string(),
            team: "LAD".to_string(),
            positions: vec!["SP".to_string()],
            gs: 1,
            ip: 6.0,
            h: 3.0,
            er: 1.0,
            bb: 2.0,
            k: 9.0,
            w: 0.0,
            sv: 0.0,
            hd: 0.0,
        };
        let whip = pitcher.whip().unwrap();
        // WHIP = (3+2)/6 = 0.833
        assert!((whip - 0.833).abs() < 0.01);
    }

    // -- Totals tests --

    #[test]
    fn batting_totals_sums_all_batters() {
        let batters = vec![
            AggregatedBatter {
                slot: "C".to_string(),
                player_name: "A".to_string(),
                team: "NYY".to_string(),
                positions: vec![],
                gp: 2,
                ab: 7.0,
                h: 2.0,
                r: 1.0,
                hr: 0.0,
                rbi: 2.0,
                bb: 1.0,
                sb: 0.0,
            },
            AggregatedBatter {
                slot: "1B".to_string(),
                player_name: "B".to_string(),
                team: "LAD".to_string(),
                positions: vec![],
                gp: 2,
                ab: 7.0,
                h: 4.0,
                r: 2.0,
                hr: 1.0,
                rbi: 3.0,
                bb: 1.0,
                sb: 0.0,
            },
        ];

        let total = batting_totals(&batters);
        assert_eq!(total.ab, 14.0);
        assert_eq!(total.h, 6.0);
        assert_eq!(total.r, 3.0);
        assert_eq!(total.hr, 1.0);
        assert_eq!(total.rbi, 5.0);
        assert_eq!(total.bb, 2.0);
        assert_eq!(total.sb, 0.0);
        // AVG = 6/14 ≈ 0.429
        let avg = total.avg().unwrap();
        assert!((avg - 0.4286).abs() < 0.001);
    }

    #[test]
    fn pitching_totals_sums_all_pitchers() {
        let pitchers = vec![
            AggregatedPitcher {
                slot: "SP".to_string(),
                player_name: "A".to_string(),
                team: "HOU".to_string(),
                positions: vec![],
                gs: 1,
                ip: 7.0,
                h: 4.0,
                er: 2.0,
                bb: 1.0,
                k: 8.0,
                w: 1.0,
                sv: 0.0,
                hd: 0.0,
            },
            AggregatedPitcher {
                slot: "RP".to_string(),
                player_name: "B".to_string(),
                team: "NYY".to_string(),
                positions: vec![],
                gs: 0,
                ip: 1.0,
                h: 0.0,
                er: 0.0,
                bb: 0.0,
                k: 2.0,
                w: 0.0,
                sv: 1.0,
                hd: 0.0,
            },
        ];

        let total = pitching_totals(&pitchers);
        assert_eq!(total.ip, 8.0);
        assert_eq!(total.h, 4.0);
        assert_eq!(total.er, 2.0);
        assert_eq!(total.bb, 1.0);
        assert_eq!(total.k, 10.0);
        assert_eq!(total.w, 1.0);
        assert_eq!(total.sv, 1.0);
        // ERA = (2/8)*9 = 2.25
        let era = total.era().unwrap();
        assert!((era - 2.25).abs() < 0.01);
        // WHIP = (4+1)/8 = 0.625
        let whip = total.whip().unwrap();
        assert!((whip - 0.625).abs() < 0.01);
    }

    // -- Formatting tests --

    #[test]
    fn fmt_avg_formats_correctly() {
        assert_eq!(fmt_avg(Some(0.286)), ".286");
        assert_eq!(fmt_avg(Some(0.0)), ".000");
        assert_eq!(fmt_avg(Some(1.0)), "1.000");
        assert_eq!(fmt_avg(None), "--");
    }

    #[test]
    fn fmt_era_formats_correctly() {
        assert_eq!(fmt_era(Some(2.57)), "2.57");
        assert_eq!(fmt_era(Some(0.0)), "0.00");
        assert_eq!(fmt_era(None), "--");
    }

    #[test]
    fn fmt_whip_formats_correctly() {
        assert_eq!(fmt_whip(Some(0.71)), "0.71");
        assert_eq!(fmt_whip(None), "--");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("Hello", 10), "Hello");
    }

    #[test]
    fn truncate_long_string_adds_ellipsis() {
        assert_eq!(truncate("Very Long Player Name", 16), "Very Long Playe…");
    }

    #[test]
    fn fmt_positions_formats_correctly() {
        assert_eq!(fmt_positions(&["1B".to_string(), "C".to_string(), "DH".to_string()]), "1B,C,DH");
        assert_eq!(fmt_positions(&[]), "--");
    }

    // -- Empty data tests --

    #[test]
    fn aggregate_empty_days() {
        let days: Vec<ScoringDay> = vec![];
        let roster = aggregate_roster(&days, TeamSide::Home);
        assert!(roster.batters.is_empty());
        assert!(roster.pitchers.is_empty());
    }

    // -- Side selection tests --

    #[test]
    fn aggregate_batters_selects_side() {
        let day = ScoringDay {
            date: "2026-03-26".to_string(),
            label: "Day 1".to_string(),
            batting_stat_columns: batting_headers(),
            pitching_stat_columns: pitching_headers(),
            home: TeamDailyRoster {
                batting_rows: vec![make_batting_row(
                    "C", "Home Hitter", "NYY", vec!["C"],
                    Some("@BOS"),
                    vec![Some(4.0), Some(2.0), Some(1.0), Some(0.0), Some(1.0), Some(0.0), Some(0.0), Some(0.500)],
                )],
                pitching_rows: vec![],
                batting_totals: None,
                pitching_totals: None,
            },
            away: TeamDailyRoster {
                batting_rows: vec![make_batting_row(
                    "1B", "Away Hitter", "NYM", vec!["1B"],
                    Some("@PHI"),
                    vec![Some(3.0), Some(1.0), Some(0.0), Some(0.0), Some(0.0), Some(1.0), Some(0.0), Some(0.333)],
                )],
                pitching_rows: vec![],
                batting_totals: None,
                pitching_totals: None,
            },
        };
        let days = vec![day];

        let home_batters = aggregate_batters(&days, TeamSide::Home);
        assert_eq!(home_batters.len(), 1);
        assert_eq!(home_batters[0].player_name, "Home Hitter");

        let away_batters = aggregate_batters(&days, TeamSide::Away);
        assert_eq!(away_batters.len(), 1);
        assert_eq!(away_batters[0].player_name, "Away Hitter");
    }

    #[test]
    fn aggregate_pitchers_selects_side() {
        let day = ScoringDay {
            date: "2026-03-26".to_string(),
            label: "Day 1".to_string(),
            batting_stat_columns: batting_headers(),
            pitching_stat_columns: pitching_headers(),
            home: TeamDailyRoster {
                batting_rows: vec![],
                pitching_rows: vec![make_pitching_row(
                    "SP", "Home Ace", "LAD", vec!["SP"],
                    Some("SD"),
                    vec![Some(7.0), Some(4.0), Some(2.0), Some(1.0), Some(8.0), Some(1.0), Some(0.0), Some(0.0)],
                )],
                batting_totals: None,
                pitching_totals: None,
            },
            away: TeamDailyRoster {
                batting_rows: vec![],
                pitching_rows: vec![make_pitching_row(
                    "SP", "Away Ace", "HOU", vec!["SP"],
                    Some("@TEX"),
                    vec![Some(6.0), Some(5.0), Some(3.0), Some(2.0), Some(7.0), Some(0.0), Some(0.0), Some(0.0)],
                )],
                batting_totals: None,
                pitching_totals: None,
            },
        };
        let days = vec![day];

        let home_pitchers = aggregate_pitchers(&days, TeamSide::Home);
        assert_eq!(home_pitchers.len(), 1);
        assert_eq!(home_pitchers[0].player_name, "Home Ace");

        let away_pitchers = aggregate_pitchers(&days, TeamSide::Away);
        assert_eq!(away_pitchers.len(), 1);
        assert_eq!(away_pitchers[0].player_name, "Away Ace");
    }

    #[test]
    fn build_roster_lines_empty_shows_placeholder() {
        let roster = AggregatedRoster {
            batters: vec![],
            pitchers: vec![],
        };
        let lines = build_roster_lines(&roster);
        assert_eq!(lines.len(), 1);
        // The placeholder text should contain "No roster data"
    }

    // -- Scroll tests --

    #[test]
    fn scroll_down_increments() {
        let mut panel = RosterViewPanel::new();
        panel.update(RosterViewPanelMessage::Scroll(ScrollDirection::Down));
        assert_eq!(panel.scroll_offset(), 1);
    }

    #[test]
    fn scroll_up_at_zero_stays() {
        let mut panel = RosterViewPanel::new();
        panel.update(RosterViewPanelMessage::Scroll(ScrollDirection::Up));
        assert_eq!(panel.scroll_offset(), 0);
    }

    #[test]
    fn scroll_page_down() {
        let mut panel = RosterViewPanel::new();
        panel.update(RosterViewPanelMessage::Scroll(ScrollDirection::PageDown));
        assert_eq!(panel.scroll_offset(), PAGE_SIZE);
    }

    // -- Render smoke tests --

    #[test]
    fn view_does_not_panic_empty_data() {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = RosterViewPanel::new();
        terminal
            .draw(|frame| {
                panel.view(frame, frame.area(), "Test Team", &[], TeamSide::Home, false)
            })
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_data() {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = RosterViewPanel::new();
        let days = vec![make_day(
            "Day 1",
            vec![make_batting_row(
                "C", "B. Rice", "NYY", vec!["1B", "C", "DH"],
                Some("@BOS"),
                vec![Some(4.0), Some(1.0), Some(1.0), Some(0.0), Some(2.0), Some(1.0), Some(0.0), Some(0.250)],
            )],
            vec![make_pitching_row(
                "SP", "F. Valdez", "HOU", vec!["SP"],
                Some("@TEX"),
                vec![Some(7.0), Some(4.0), Some(2.0), Some(1.0), Some(8.0), Some(1.0), Some(0.0), Some(0.0)],
            )],
        )];
        terminal
            .draw(|frame| {
                panel.view(
                    frame,
                    frame.area(),
                    "Bob Dole Experience",
                    &days,
                    TeamSide::Home,
                    true,
                )
            })
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_narrow_terminal() {
        let backend = ratatui::backend::TestBackend::new(60, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = RosterViewPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), "Team", &[], TeamSide::Home, false))
            .unwrap();
    }

    #[test]
    fn view_with_scrolled_position() {
        let backend = ratatui::backend::TestBackend::new(120, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = RosterViewPanel::new();
        // Scroll way past content — clamped_offset will handle it
        for _ in 0..50 {
            panel.update(RosterViewPanelMessage::Scroll(ScrollDirection::Down));
        }
        let days = vec![make_day(
            "Day 1",
            vec![make_batting_row(
                "C", "B. Rice", "NYY", vec!["C"],
                Some("@BOS"),
                vec![Some(4.0), Some(1.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.250)],
            )],
            vec![],
        )];
        terminal
            .draw(|frame| panel.view(frame, frame.area(), "Team", &days, TeamSide::Home, false))
            .unwrap();
    }

    // -- GP/GS mixed scenario test --

    #[test]
    fn gp_counts_only_active_slot_with_opponent() {
        let days = vec![
            // Day 1: Player starts at C with opponent
            make_day(
                "Day 1",
                vec![make_batting_row(
                    "C", "Player A", "NYY", vec!["C"],
                    Some("@BOS"),
                    vec![Some(4.0), Some(1.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.250)],
                )],
                vec![],
            ),
            // Day 2: Same player, no game
            make_day(
                "Day 2",
                vec![make_batting_row(
                    "C", "Player A", "NYY", vec!["C"],
                    None,
                    vec![None, None, None, None, None, None, None, None],
                )],
                vec![],
            ),
            // Day 3: Same player on BENCH with opponent
            make_day(
                "Day 3",
                vec![make_batting_row(
                    "BENCH", "Player A", "NYY", vec!["C"],
                    Some("TB"),
                    vec![Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0), None],
                )],
                vec![],
            ),
        ];

        let batters = aggregate_batters(&days, TeamSide::Home);
        assert_eq!(batters.len(), 1);
        assert_eq!(batters[0].gp, 1); // Only Day 1 counts (Day 2 no opp, Day 3 bench)
    }

    #[test]
    fn gs_counts_only_sp_slot_with_opponent() {
        let days = vec![
            // Day 1: Pitcher starts as SP
            make_day(
                "Day 1",
                vec![],
                vec![make_pitching_row(
                    "SP", "Ace", "LAD", vec!["SP"],
                    Some("SD"),
                    vec![Some(7.0), Some(4.0), Some(2.0), Some(1.0), Some(8.0), Some(1.0), Some(0.0), Some(0.0)],
                )],
            ),
            // Day 2: Same pitcher in RP slot
            make_day(
                "Day 2",
                vec![],
                vec![make_pitching_row(
                    "RP", "Ace", "LAD", vec!["SP"],
                    Some("SF"),
                    vec![Some(1.0), Some(0.0), Some(0.0), Some(0.0), Some(2.0), Some(0.0), Some(0.0), Some(0.0)],
                )],
            ),
            // Day 3: SP slot but no game
            make_day(
                "Day 3",
                vec![],
                vec![make_pitching_row(
                    "SP", "Ace", "LAD", vec!["SP"],
                    None,
                    vec![None, None, None, None, None, None, None, None],
                )],
            ),
        ];

        let pitchers = aggregate_pitchers(&days, TeamSide::Home);
        assert_eq!(pitchers.len(), 1);
        assert_eq!(pitchers[0].gs, 1); // Only Day 1 counts (Day 2 RP, Day 3 no game)
        assert_eq!(pitchers[0].ip, 8.0); // 7 + 1
    }
}
