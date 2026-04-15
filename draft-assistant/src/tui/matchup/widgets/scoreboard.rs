// Scoreboard widget: category-by-category H2H comparison header.
//
// The scoreboard is rendered symmetrically for home and away — there is no
// "my team" bias.
//
// Layout (rendered top-to-bottom within the provided area):
//   Row 1: Header    — blank label | R HR RBI … | K W SV …
//   Row 2: Home team — values with winning cells highlighted
//   Row 3: Away team — values with winning cells highlighted
//   Row 4: H-A diff  — per-category signed differential (home - away)

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::matchup::{CategoryScore, CategoryState, TeamMatchupState};
use crate::stats::{SortDirection, StatRegistry};

/// Column width for each stat value cell.
const COL_WIDTH: usize = 6;

/// Column width for the team name label.
const LABEL_WIDTH: usize = 8;

/// Format a stat value using the given precision.
///
/// For AVG (precision 3) and ERA/WHIP (precision 2), strips the leading "0"
/// so we get ".275" instead of "0.275".
fn format_value(value: f64, precision: u8) -> String {
    let formatted = format!("{:.prec$}", value, prec = precision as usize);
    if precision >= 2 && value.abs() < 1.0 && value >= 0.0 {
        // Strip leading zero: "0.275" -> ".275"
        formatted.strip_prefix('0').unwrap_or(&formatted).to_string()
    } else {
        formatted
    }
}

/// Format a home-minus-away differential value with sign prefix.
fn format_diff(home_value: f64, away_value: f64, precision: u8) -> String {
    let raw_diff = home_value - away_value;
    let abs_formatted = format_value(raw_diff.abs(), precision);

    if raw_diff > 0.0 {
        format!("+{}", abs_formatted)
    } else if raw_diff < 0.0 {
        format!("-{}", abs_formatted)
    } else {
        abs_formatted
    }
}

/// Determine the color for the home-minus-away differential cell.
///
/// For `LowerIsBetter` stats, the sign sense is inverted so the color still
/// reflects who is "ahead", not just numerical sign.
fn diff_color(home_value: f64, away_value: f64, lower_is_better: bool) -> Color {
    let raw_diff = home_value - away_value;
    let effective = if lower_is_better { -raw_diff } else { raw_diff };

    if effective > 0.0 {
        Color::Green
    } else if effective < 0.0 {
        Color::Red
    } else {
        Color::Yellow
    }
}

/// Truncate a string to at most `max_len` characters.
pub fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        name[..max_len].to_string()
    }
}

/// Look up format precision for a stat abbreviation from the registry.
fn precision_for_stat(abbrev: &str, registry: &StatRegistry) -> u8 {
    registry
        .get(abbrev)
        .map(|def| def.format_precision)
        .unwrap_or(0)
}

/// Look up whether a stat is LowerIsBetter from the registry.
fn is_lower_better(abbrev: &str, registry: &StatRegistry) -> bool {
    registry
        .get(abbrev)
        .map(|def| def.sort_direction == SortDirection::LowerIsBetter)
        .unwrap_or(false)
}

/// Render the scoreboard into the given area.
///
/// Displays all H2H categories with both teams' values, winning indicators,
/// and the H-A differential row.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    category_scores: &[CategoryScore],
    home_team: &TeamMatchupState,
    away_team: &TeamMatchupState,
    registry: &StatRegistry,
) {
    let batting_cats: Vec<&CategoryScore> = category_scores
        .iter()
        .filter(|c| {
            registry
                .get(&c.stat_abbrev)
                .is_some_and(|d| d.player_type == crate::stats::PlayerType::Hitter)
        })
        .collect();
    let pitching_cats: Vec<&CategoryScore> = category_scores
        .iter()
        .filter(|c| {
            registry
                .get(&c.stat_abbrev)
                .is_some_and(|d| d.player_type == crate::stats::PlayerType::Pitcher)
        })
        .collect();

    let lines = vec![
        build_header_line(&batting_cats, &pitching_cats),
        build_team_line(
            &truncate_name(&home_team.abbrev, LABEL_WIDTH - 1),
            &batting_cats,
            &pitching_cats,
            true,
            registry,
        ),
        build_team_line(
            &truncate_name(&away_team.abbrev, LABEL_WIDTH - 1),
            &batting_cats,
            &pitching_cats,
            false,
            registry,
        ),
        build_diff_line(&batting_cats, &pitching_cats, registry),
    ];

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(paragraph, area);
}

/// Build the header row with stat abbreviations.
fn build_header_line(batting: &[&CategoryScore], pitching: &[&CategoryScore]) -> Line<'static> {
    let mut spans = Vec::new();
    let header_style = Style::default().fg(Color::DarkGray);

    // Label column (blank for header)
    spans.push(Span::styled(
        format!("{:width$}", "", width = LABEL_WIDTH),
        header_style,
    ));

    // Batting categories
    for cat in batting {
        spans.push(Span::styled(
            format!("{:>width$}", cat.stat_abbrev, width = COL_WIDTH),
            header_style,
        ));
    }

    // Separator
    spans.push(Span::styled(" \u{2502} ", header_style));

    // Pitching categories
    for cat in pitching {
        spans.push(Span::styled(
            format!("{:>width$}", cat.stat_abbrev, width = COL_WIDTH),
            header_style,
        ));
    }

    Line::from(spans)
}

/// Build a team row showing stat values with winning indicators.
///
/// `is_home` selects whether `home_value` or `away_value` is shown for each
/// category and which side of the `CategoryState` counts as "us" for
/// highlighting.
fn build_team_line(
    label: &str,
    batting: &[&CategoryScore],
    pitching: &[&CategoryScore],
    is_home: bool,
    registry: &StatRegistry,
) -> Line<'static> {
    let mut spans = Vec::new();

    // Team name label
    spans.push(Span::styled(
        format!("{:width$}", label, width = LABEL_WIDTH),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ));

    // Batting values
    for cat in batting {
        let value = if is_home { cat.home_value } else { cat.away_value };
        let prec = precision_for_stat(&cat.stat_abbrev, registry);
        let formatted = format_value(value, prec);
        let (style, prefix) = cell_style(cat.state, is_home);
        let display = format!(
            "{:>width$}",
            format!("{}{}", prefix, formatted),
            width = COL_WIDTH
        );
        spans.push(Span::styled(display, style));
    }

    // Separator
    spans.push(Span::styled(
        " \u{2502} ",
        Style::default().fg(Color::DarkGray),
    ));

    // Pitching values
    for cat in pitching {
        let value = if is_home { cat.home_value } else { cat.away_value };
        let prec = precision_for_stat(&cat.stat_abbrev, registry);
        let formatted = format_value(value, prec);
        let (style, prefix) = cell_style(cat.state, is_home);
        let display = format!(
            "{:>width$}",
            format!("{}{}", prefix, formatted),
            width = COL_WIDTH
        );
        spans.push(Span::styled(display, style));
    }

    Line::from(spans)
}

/// Determine the style and prefix for a stat cell based on win state.
///
/// Returns `(Style, prefix_str)`. The winning team's cell gets a `*` prefix
/// and green bold text; the losing side is plain white; tied is yellow.
/// The treatment is fully symmetric between home and away.
fn cell_style(state: CategoryState, is_home: bool) -> (Style, &'static str) {
    match state {
        CategoryState::HomeWinning => {
            if is_home {
                (
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                    "*",
                )
            } else {
                (Style::default().fg(Color::White), "")
            }
        }
        CategoryState::AwayWinning => {
            if is_home {
                (Style::default().fg(Color::White), "")
            } else {
                (
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                    "*",
                )
            }
        }
        CategoryState::Tied => (Style::default().fg(Color::Yellow), ""),
    }
}

/// Build the H-A differential row.
fn build_diff_line(
    batting: &[&CategoryScore],
    pitching: &[&CategoryScore],
    registry: &StatRegistry,
) -> Line<'static> {
    let mut spans = Vec::new();
    let label_style = Style::default().fg(Color::DarkGray);

    // Label
    spans.push(Span::styled(
        format!("{:width$}", "H-A", width = LABEL_WIDTH),
        label_style,
    ));

    // Batting diffs
    for cat in batting {
        let prec = precision_for_stat(&cat.stat_abbrev, registry);
        let lower = is_lower_better(&cat.stat_abbrev, registry);
        let diff_str = format_diff(cat.home_value, cat.away_value, prec);
        let color = diff_color(cat.home_value, cat.away_value, lower);
        spans.push(Span::styled(
            format!("{:>width$}", diff_str, width = COL_WIDTH),
            Style::default().fg(color),
        ));
    }

    // Separator
    spans.push(Span::styled(" \u{2502} ", label_style));

    // Pitching diffs
    for cat in pitching {
        let prec = precision_for_stat(&cat.stat_abbrev, registry);
        let lower = is_lower_better(&cat.stat_abbrev, registry);
        let diff_str = format_diff(cat.home_value, cat.away_value, prec);
        let color = diff_color(cat.home_value, cat.away_value, lower);
        spans.push(Span::styled(
            format!("{:>width$}", diff_str, width = COL_WIDTH),
            Style::default().fg(color),
        ));
    }

    Line::from(spans)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matchup::TeamRecord;
    use crate::test_utils::test_registry;

    fn make_category(abbrev: &str, home: f64, away: f64, state: CategoryState) -> CategoryScore {
        CategoryScore {
            stat_abbrev: abbrev.to_string(),
            home_value: home,
            away_value: away,
            state,
        }
    }

    fn make_full_categories() -> Vec<CategoryScore> {
        vec![
            make_category("R", 5.0, 3.0, CategoryState::HomeWinning),
            make_category("HR", 2.0, 3.0, CategoryState::AwayWinning),
            make_category("RBI", 5.0, 4.0, CategoryState::HomeWinning),
            make_category("BB", 3.0, 1.0, CategoryState::HomeWinning),
            make_category("SB", 1.0, 2.0, CategoryState::AwayWinning),
            make_category("AVG", 0.275, 0.290, CategoryState::AwayWinning),
            make_category("K", 42.0, 48.0, CategoryState::AwayWinning),
            make_category("W", 1.0, 2.0, CategoryState::AwayWinning),
            make_category("SV", 0.0, 1.0, CategoryState::AwayWinning),
            make_category("HD", 2.0, 0.0, CategoryState::HomeWinning),
            make_category("ERA", 3.50, 4.20, CategoryState::HomeWinning),
            make_category("WHIP", 1.20, 1.35, CategoryState::HomeWinning),
        ]
    }

    fn make_home_team() -> TeamMatchupState {
        TeamMatchupState {
            name: "Bob Dole Experience".to_string(),
            abbrev: "BDE".to_string(),
            record: TeamRecord { wins: 1, losses: 0, ties: 0 },
            category_score: TeamRecord { wins: 6, losses: 4, ties: 2 },
        }
    }

    fn make_away_team() -> TeamMatchupState {
        TeamMatchupState {
            name: "Certified! Smokified!".to_string(),
            abbrev: "C!S!".to_string(),
            record: TeamRecord { wins: 0, losses: 1, ties: 0 },
            category_score: TeamRecord { wins: 4, losses: 6, ties: 2 },
        }
    }

    // -- format_value tests --

    #[test]
    fn format_counting_stat() {
        assert_eq!(format_value(5.0, 0), "5");
        assert_eq!(format_value(42.0, 0), "42");
        assert_eq!(format_value(0.0, 0), "0");
    }

    #[test]
    fn format_avg_strips_leading_zero() {
        assert_eq!(format_value(0.275, 3), ".275");
        assert_eq!(format_value(0.290, 3), ".290");
        assert_eq!(format_value(0.000, 3), ".000");
    }

    #[test]
    fn format_era_strips_leading_zero() {
        assert_eq!(format_value(0.50, 2), ".50");
    }

    #[test]
    fn format_era_above_one() {
        assert_eq!(format_value(3.50, 2), "3.50");
        assert_eq!(format_value(4.20, 2), "4.20");
    }

    #[test]
    fn format_whip_above_one() {
        assert_eq!(format_value(1.20, 2), "1.20");
        assert_eq!(format_value(1.35, 2), "1.35");
    }

    // -- format_diff tests --

    #[test]
    fn diff_home_ahead_counting() {
        // R: home 5 vs away 3 => +2
        assert_eq!(format_diff(5.0, 3.0, 0), "+2");
    }

    #[test]
    fn diff_away_ahead_counting() {
        // HR: home 2 vs away 3 => -1
        assert_eq!(format_diff(2.0, 3.0, 0), "-1");
    }

    #[test]
    fn diff_zero() {
        assert_eq!(format_diff(3.0, 3.0, 0), "0");
    }

    #[test]
    fn diff_home_better_rate() {
        // ERA: home 3.50 vs away 4.20 (lower is better) -> raw diff -0.70
        assert_eq!(format_diff(3.50, 4.20, 2), "-.70");
    }

    // -- diff_color tests --

    #[test]
    fn diff_color_higher_is_better_home_ahead() {
        assert_eq!(diff_color(5.0, 3.0, false), Color::Green);
    }

    #[test]
    fn diff_color_higher_is_better_away_ahead() {
        assert_eq!(diff_color(2.0, 3.0, false), Color::Red);
    }

    #[test]
    fn diff_color_zero_is_yellow() {
        assert_eq!(diff_color(3.0, 3.0, false), Color::Yellow);
    }

    #[test]
    fn diff_color_lower_is_better_home_ahead() {
        // ERA: home 3.50 vs away 4.20 (lower is better) => home is ahead => green
        assert_eq!(diff_color(3.50, 4.20, true), Color::Green);
    }

    #[test]
    fn diff_color_lower_is_better_away_ahead() {
        // ERA: home 4.20 vs away 3.50 (lower is better) => away is ahead => red
        assert_eq!(diff_color(4.20, 3.50, true), Color::Red);
    }

    // -- truncate_name tests --

    #[test]
    fn truncate_short_name_unchanged() {
        assert_eq!(truncate_name("BDE", 7), "BDE");
    }

    #[test]
    fn truncate_long_name() {
        assert_eq!(truncate_name("Bob Dole Experience", 7), "Bob Dol");
    }

    #[test]
    fn truncate_exact_length() {
        assert_eq!(truncate_name("1234567", 7), "1234567");
    }

    // -- cell_style tests: symmetric home/away --

    #[test]
    fn home_winning_home_cell_is_green_bold_starred() {
        let (style, prefix) = cell_style(CategoryState::HomeWinning, true);
        assert_eq!(style.fg, Some(Color::Green));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(prefix, "*");
    }

    #[test]
    fn home_winning_away_cell_is_white() {
        let (style, prefix) = cell_style(CategoryState::HomeWinning, false);
        assert_eq!(style.fg, Some(Color::White));
        assert_eq!(prefix, "");
    }

    #[test]
    fn away_winning_home_cell_is_white() {
        let (style, prefix) = cell_style(CategoryState::AwayWinning, true);
        assert_eq!(style.fg, Some(Color::White));
        assert_eq!(prefix, "");
    }

    #[test]
    fn away_winning_away_cell_is_green_bold_starred() {
        let (style, prefix) = cell_style(CategoryState::AwayWinning, false);
        assert_eq!(style.fg, Some(Color::Green));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(prefix, "*");
    }

    #[test]
    fn tied_is_yellow_on_both_sides() {
        let (style, prefix) = cell_style(CategoryState::Tied, true);
        assert_eq!(style.fg, Some(Color::Yellow));
        assert_eq!(prefix, "");

        let (style, prefix) = cell_style(CategoryState::Tied, false);
        assert_eq!(style.fg, Some(Color::Yellow));
        assert_eq!(prefix, "");
    }

    // -- precision_for_stat tests --

    #[test]
    fn precision_from_registry() {
        let registry = test_registry();
        assert_eq!(precision_for_stat("R", &registry), 0);
        assert_eq!(precision_for_stat("HR", &registry), 0);
        assert_eq!(precision_for_stat("AVG", &registry), 3);
        assert_eq!(precision_for_stat("ERA", &registry), 2);
        assert_eq!(precision_for_stat("WHIP", &registry), 2);
        assert_eq!(precision_for_stat("K", &registry), 0);
    }

    #[test]
    fn unknown_stat_defaults_to_zero_precision() {
        let registry = test_registry();
        assert_eq!(precision_for_stat("UNKNOWN", &registry), 0);
    }

    // -- is_lower_better tests --

    #[test]
    fn era_is_lower_better() {
        let registry = test_registry();
        assert!(is_lower_better("ERA", &registry));
        assert!(is_lower_better("WHIP", &registry));
    }

    #[test]
    fn counting_stats_are_higher_better() {
        let registry = test_registry();
        assert!(!is_lower_better("R", &registry));
        assert!(!is_lower_better("HR", &registry));
        assert!(!is_lower_better("AVG", &registry));
        assert!(!is_lower_better("K", &registry));
    }

    // -- render smoke tests --

    #[test]
    fn render_does_not_panic_empty_categories() {
        let backend = ratatui::backend::TestBackend::new(160, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let registry = test_registry();
        let home_team = make_home_team();
        let away_team = make_away_team();
        terminal
            .draw(|frame| render(frame, frame.area(), &[], &home_team, &away_team, &registry))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_full_categories() {
        let backend = ratatui::backend::TestBackend::new(160, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let registry = test_registry();
        let categories = make_full_categories();
        let home_team = make_home_team();
        let away_team = make_away_team();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &categories,
                    &home_team,
                    &away_team,
                    &registry,
                )
            })
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_narrow_terminal() {
        let backend = ratatui::backend::TestBackend::new(80, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let registry = test_registry();
        let categories = make_full_categories();
        let home_team = make_home_team();
        let away_team = make_away_team();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &categories,
                    &home_team,
                    &away_team,
                    &registry,
                )
            })
            .unwrap();
    }

    // -- header line content --

    #[test]
    fn header_contains_stat_abbrevs() {
        let categories = make_full_categories();
        let registry = test_registry();
        let batting: Vec<&CategoryScore> = categories
            .iter()
            .filter(|c| registry.get(&c.stat_abbrev).is_some_and(|d| d.player_type == crate::stats::PlayerType::Hitter))
            .collect();
        let pitching: Vec<&CategoryScore> = categories
            .iter()
            .filter(|c| registry.get(&c.stat_abbrev).is_some_and(|d| d.player_type == crate::stats::PlayerType::Pitcher))
            .collect();
        let line = build_header_line(&batting, &pitching);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("R"));
        assert!(text.contains("HR"));
        assert!(text.contains("AVG"));
        assert!(text.contains("ERA"));
        assert!(text.contains("WHIP"));
    }
}
