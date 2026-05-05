// Matchup analytics panel: computed matchup insights rendered in a scrollable view.
//
// Sections:
// 1. Category Outlook — home-winning/away-winning/tied buckets
// 2. Close Categories — swingable categories within threshold
// 3. Pace Projections — linear projection of counting stats, component-based for rates
//
// The page is rendered symmetrically from the home/away perspective. The
// "diff" column is home - away (positive = home ahead).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::matchup::{CategoryScore, CategoryState, ScoringDay};
use crate::stats::{SortDirection, StatComputation, StatDefinition, StatRegistry};
use crate::tui::action::Action;
use crate::tui::matchup::colors::{HOME_COLOR, AWAY_COLOR, TIED_COLOR};
use crate::tui::scroll::{ScrollDirection, ScrollState};

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum MatchupAnalyticsPanelMessage {
    Scroll(ScrollDirection),
}

// ---------------------------------------------------------------------------
// MatchupAnalyticsPanel
// ---------------------------------------------------------------------------

pub struct MatchupAnalyticsPanel {
    scroll: ScrollState,
}

impl MatchupAnalyticsPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    pub fn update(&mut self, msg: MatchupAnalyticsPanelMessage) -> Option<Action> {
        match msg {
            MatchupAnalyticsPanelMessage::Scroll(dir) => {
                self.scroll.scroll(dir, 20);
                None
            }
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    /// Render the analytics panel with all matchup data.
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &self,
        frame: &mut Frame,
        area: Rect,
        category_scores: &[CategoryScore],
        scoring_period_days: &[ScoringDay],
        selected_day: usize,
        registry: Option<&StatRegistry>,
        home_abbrev: &str,
        away_abbrev: &str,
        _focused: bool,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Analytics ")
            .border_style(Style::default().fg(Color::DarkGray));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 2 || inner.width < 10 {
            return;
        }

        let total_days = scoring_period_days.len();
        let days_elapsed = if total_days > 0 {
            (selected_day + 1).min(total_days)
        } else {
            0
        };

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Section 1: Category Outlook
        build_category_outlook(
            &mut lines,
            category_scores,
            days_elapsed,
            total_days,
            registry,
            home_abbrev,
            away_abbrev,
        );

        // Section 2: Close Categories
        build_close_categories(&mut lines, category_scores, registry, home_abbrev, away_abbrev);

        // Section 3: Pace Projections
        build_pace_projections(
            &mut lines,
            category_scores,
            days_elapsed,
            total_days,
            registry,
            home_abbrev,
            away_abbrev,
        );

        let content_height = lines.len();
        let viewport_height = inner.height as usize;
        let offset = self.scroll.clamped_offset(content_height, viewport_height);

        let visible: Vec<Line<'static>> = lines
            .into_iter()
            .skip(offset)
            .take(viewport_height)
            .collect();

        let paragraph = Paragraph::new(visible).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);
    }
}

impl Default for MatchupAnalyticsPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Section builders
// ---------------------------------------------------------------------------

fn section_header(lines: &mut Vec<Line<'static>>, title: &str) {
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        format!("── {title} ──────────────────────────────────────────"),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));
}

fn build_category_outlook(
    lines: &mut Vec<Line<'static>>,
    scores: &[CategoryScore],
    days_elapsed: usize,
    total_days: usize,
    registry: Option<&StatRegistry>,
    home_abbrev: &str,
    away_abbrev: &str,
) {
    section_header(
        lines,
        &format!(
            "CATEGORY OUTLOOK (Day {} of {})",
            days_elapsed, total_days
        ),
    );

    if scores.is_empty() {
        lines.push(Line::from("  No category data available."));
        return;
    }

    let home_winning: Vec<&CategoryScore> = scores
        .iter()
        .filter(|c| c.state == CategoryState::HomeWinning)
        .collect();
    let away_winning: Vec<&CategoryScore> = scores
        .iter()
        .filter(|c| c.state == CategoryState::AwayWinning)
        .collect();
    let tied: Vec<&CategoryScore> = scores
        .iter()
        .filter(|c| c.state == CategoryState::Tied)
        .collect();

    // Header row
    lines.push(Line::from(vec![
        Span::styled(
            format!("  {} ({})               ", home_abbrev, home_winning.len()),
            Style::default()
                .fg(HOME_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} ({})              ", away_abbrev, away_winning.len()),
            Style::default()
                .fg(AWAY_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("TIED ({})", tied.len()),
            Style::default()
                .fg(TIED_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let max_rows = home_winning
        .len()
        .max(away_winning.len())
        .max(tied.len());
    for i in 0..max_rows {
        let mut spans = Vec::new();

        // Home-winning column
        if let Some(cat) = home_winning.get(i) {
            let diff = format_diff(cat, registry);
            spans.push(Span::styled(
                format!("  {:<6}{:<18}", cat.stat_abbrev, diff),
                Style::default().fg(HOME_COLOR),
            ));
        } else {
            spans.push(Span::raw("                        "));
        }

        // Away-winning column
        if let Some(cat) = away_winning.get(i) {
            let diff = format_diff(cat, registry);
            spans.push(Span::styled(
                format!("{:<6}{:<16}", cat.stat_abbrev, diff),
                Style::default().fg(AWAY_COLOR),
            ));
        } else {
            spans.push(Span::raw("                      "));
        }

        // Tied column
        if let Some(cat) = tied.get(i) {
            let diff = format_diff(cat, registry);
            spans.push(Span::styled(
                format!("{:<6}{}", cat.stat_abbrev, diff),
                Style::default().fg(TIED_COLOR),
            ));
        }

        lines.push(Line::from(spans));
    }
}

fn build_close_categories(
    lines: &mut Vec<Line<'static>>,
    scores: &[CategoryScore],
    registry: Option<&StatRegistry>,
    home_abbrev: &str,
    away_abbrev: &str,
) {
    section_header(lines, "CLOSE CATEGORIES (swingable)");

    let close: Vec<&CategoryScore> = scores
        .iter()
        .filter(|c| is_close_category(c, registry))
        .collect();

    if close.is_empty() {
        lines.push(Line::from("  No close categories."));
        return;
    }

    lines.push(Line::from(Span::styled(
        format!("  Category  {:<6}  {:<6}  Diff     Status", home_abbrev, away_abbrev),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "  ────────  ──────  ──────  ───────  ──────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    for cat in &close {
        let stat_def = registry.and_then(|r| r.get(&cat.stat_abbrev));
        let precision = stat_def.map_or(0, |d| d.format_precision as usize);
        let is_counting = stat_def
            .map(|d| matches!(d.computation, StatComputation::Counting { .. }))
            .unwrap_or(true);
        let lower_is_better = stat_def
            .map(|d| d.sort_direction == SortDirection::LowerIsBetter)
            .unwrap_or(false);

        let home_display = format_value(cat.home_value, precision);
        let away_display = format_value(cat.away_value, precision);

        let raw_diff = cat.home_value - cat.away_value;
        // effective_diff: positive means the leader (per lower_is_better) is ahead.
        let effective_diff = if lower_is_better { -raw_diff } else { raw_diff };
        let diff_display = format_signed_value(raw_diff, precision);

        let status = build_close_status(cat, is_counting, effective_diff, home_abbrev, away_abbrev);

        let color = match cat.state {
            CategoryState::HomeWinning => HOME_COLOR,
            CategoryState::AwayWinning => AWAY_COLOR,
            CategoryState::Tied => TIED_COLOR,
        };

        lines.push(Line::from(vec![
            Span::raw(format!("  {:<10}", cat.stat_abbrev)),
            Span::raw(format!("{:<8}", home_display)),
            Span::raw(format!("{:<8}", away_display)),
            Span::styled(format!("{:<9}", diff_display), Style::default().fg(color)),
            Span::styled(status, Style::default().fg(color)),
        ]));
    }
}

fn build_pace_projections(
    lines: &mut Vec<Line<'static>>,
    scores: &[CategoryScore],
    days_elapsed: usize,
    total_days: usize,
    registry: Option<&StatRegistry>,
    home_abbrev: &str,
    away_abbrev: &str,
) {
    section_header(lines, "PACE PROJECTIONS");

    if days_elapsed == 0 || total_days == 0 {
        lines.push(Line::from("  No games played yet."));
        return;
    }

    if scores.is_empty() {
        lines.push(Line::from("  No category data available."));
        return;
    }

    lines.push(Line::from(Span::styled(
        format!(
            "  Based on {} day(s) played, projecting over {}-day period:",
            days_elapsed, total_days
        ),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    // Header
    lines.push(Line::from(Span::styled(
        format!("  Category  {:<7}  {:<4} Proj  {:<4} Proj  Proj Result", home_abbrev, home_abbrev, away_abbrev),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "  ────────  ───────  ─────────  ─────────  ───────────────",
        Style::default().fg(Color::DarkGray),
    )));

    for cat in scores {
        let stat_def = registry.and_then(|r| r.get(&cat.stat_abbrev));
        let precision = stat_def.map_or(0, |d| d.format_precision as usize);
        let lower_is_better = stat_def
            .map(|d| d.sort_direction == SortDirection::LowerIsBetter)
            .unwrap_or(false);

        let home_proj = project_stat(cat.home_value, days_elapsed, total_days, stat_def);
        let away_proj = project_stat(cat.away_value, days_elapsed, total_days, stat_def);

        // Result is expressed from the home team's perspective.
        let proj_diff = if lower_is_better {
            away_proj - home_proj
        } else {
            home_proj - away_proj
        };

        let (result_label, result_color) = if proj_diff > 0.001 {
            (home_abbrev, HOME_COLOR)
        } else if proj_diff < -0.001 {
            (away_abbrev, AWAY_COLOR)
        } else {
            ("TIE", TIED_COLOR)
        };

        let raw_proj_diff = home_proj - away_proj;
        let diff_str = format_signed_value(raw_proj_diff, precision);
        let result_str = format!("{result_label} ({diff_str})");

        lines.push(Line::from(vec![
            Span::raw(format!("  {:<10}", cat.stat_abbrev)),
            Span::raw(format!(
                "{:<9}",
                format_value(cat.home_value, precision)
            )),
            Span::raw(format!("{:<11}", format_value(home_proj, precision))),
            Span::raw(format!("{:<11}", format_value(away_proj, precision))),
            Span::styled(result_str, Style::default().fg(result_color)),
        ]));
    }
}

// ---------------------------------------------------------------------------
// Computation helpers
// ---------------------------------------------------------------------------

/// Format a differential value for a category, accounting for stat precision.
/// Diff is home - away.
fn format_diff(cat: &CategoryScore, registry: Option<&StatRegistry>) -> String {
    let stat_def = registry.and_then(|r| r.get(&cat.stat_abbrev));
    let precision = stat_def.map_or(0, |d| d.format_precision as usize);

    let raw_diff = cat.home_value - cat.away_value;
    format_signed_value(raw_diff, precision)
}

/// Check if a category is "close" (swingable) using registry thresholds.
pub fn is_close_category(cat: &CategoryScore, registry: Option<&StatRegistry>) -> bool {
    let diff = (cat.home_value - cat.away_value).abs();

    if let Some(reg) = registry {
        if let Some(stat_def) = reg.get(&cat.stat_abbrev) {
            return diff <= stat_def.matchup_close_threshold;
        }
    }

    // Fallback thresholds if no registry
    match cat.stat_abbrev.as_str() {
        "R" | "RBI" | "BB" => diff <= 5.0,
        "HR" | "SB" | "W" | "SV" | "HD" => diff <= 3.0,
        "K" => diff <= 10.0,
        "AVG" => diff <= 0.020,
        "ERA" => diff <= 1.00,
        "WHIP" => diff <= 0.20,
        _ => false,
    }
}

/// Project a stat value linearly over the full matchup period.
///
/// For counting stats: `(current / days_elapsed) * total_days`
/// For rate stats: project the current value directly (rate stats don't scale linearly
/// without component data, so we preserve the current rate as the projection).
pub fn project_stat(
    current: f64,
    days_elapsed: usize,
    total_days: usize,
    stat_def: Option<&StatDefinition>,
) -> f64 {
    if days_elapsed == 0 {
        return 0.0;
    }

    match stat_def.map(|d| &d.computation) {
        Some(StatComputation::RateStat { .. }) => {
            // Rate stats: preserve current rate as projection.
            // True component-based projection requires volume data we don't have
            // at this level (H, AB, ER, IP, etc.). The current rate is the best
            // estimate we can give from category scores alone.
            current
        }
        _ => {
            // Counting stats: linear projection
            project_counting_stat(current, days_elapsed, total_days)
        }
    }
}

/// Linear projection of a counting stat over the full period.
pub fn project_counting_stat(current: f64, days_elapsed: usize, total_days: usize) -> f64 {
    if days_elapsed == 0 {
        return 0.0;
    }
    (current / days_elapsed as f64) * total_days as f64
}

/// Build a close-category status string.
///
/// `effective_diff` is positive when the current leader (per sort direction) is ahead.
fn build_close_status(cat: &CategoryScore, is_counting: bool, effective_diff: f64, home_abbrev: &str, away_abbrev: &str) -> String {
    match cat.state {
        CategoryState::HomeWinning => format!("{} - lead is narrow", home_abbrev),
        CategoryState::AwayWinning => {
            if is_counting {
                let to_tie = effective_diff.abs().ceil() as i64;
                let to_lead = to_tie + 1;
                format!(
                    "{} - {} {} to tie, {} to lead",
                    away_abbrev, to_tie, cat.stat_abbrev, to_lead
                )
            } else {
                format!("{} - gap is closeable", away_abbrev)
            }
        }
        CategoryState::Tied => {
            if is_counting {
                "TIED - 1 to lead".to_string()
            } else {
                "TIED".to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_value(value: f64, precision: usize) -> String {
    if precision == 0 {
        format!("{}", value as i64)
    } else {
        format!("{:.prec$}", value, prec = precision)
    }
}

fn format_signed_value(value: f64, precision: usize) -> String {
    if precision == 0 {
        let v = value as i64;
        if v >= 0 {
            format!("+{v}")
        } else {
            format!("{v}")
        }
    } else if value >= 0.0 {
        format!("+{:.prec$}", value, prec = precision)
    } else {
        format!("{:.prec$}", value, prec = precision)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LeagueConfig;
    use crate::matchup::TeamDailyRoster;
    use crate::tui::scroll::ScrollDirection;

    fn test_registry() -> StatRegistry {
        StatRegistry::from_league_config(&LeagueConfig::default()).unwrap()
    }

    fn make_cat(abbrev: &str, home: f64, away: f64, state: CategoryState) -> CategoryScore {
        CategoryScore {
            stat_abbrev: abbrev.to_string(),
            home_value: home,
            away_value: away,
            state,
        }
    }

    // -- Category grouping tests --

    #[test]
    fn category_outlook_groups_correctly() {
        let scores = [
            make_cat("R", 5.0, 3.0, CategoryState::HomeWinning),
            make_cat("HR", 2.0, 3.0, CategoryState::AwayWinning),
            make_cat("SB", 1.0, 1.0, CategoryState::Tied),
            make_cat("RBI", 4.0, 2.0, CategoryState::HomeWinning),
        ];

        let home_winning: Vec<_> = scores
            .iter()
            .filter(|c| c.state == CategoryState::HomeWinning)
            .collect();
        let away_winning: Vec<_> = scores
            .iter()
            .filter(|c| c.state == CategoryState::AwayWinning)
            .collect();
        let tied: Vec<_> = scores
            .iter()
            .filter(|c| c.state == CategoryState::Tied)
            .collect();

        assert_eq!(home_winning.len(), 2);
        assert_eq!(away_winning.len(), 1);
        assert_eq!(tied.len(), 1);
        assert_eq!(home_winning[0].stat_abbrev, "R");
        assert_eq!(home_winning[1].stat_abbrev, "RBI");
        assert_eq!(away_winning[0].stat_abbrev, "HR");
        assert_eq!(tied[0].stat_abbrev, "SB");
    }

    // -- Close category detection tests --

    #[test]
    fn close_category_respects_threshold_counting() {
        let reg = test_registry();

        // HR threshold is 3.0 for matchup
        let close = make_cat("HR", 2.0, 4.0, CategoryState::AwayWinning); // diff=2
        assert!(is_close_category(&close, Some(&reg)));

        let not_close = make_cat("HR", 2.0, 10.0, CategoryState::AwayWinning); // diff=8
        assert!(!is_close_category(&not_close, Some(&reg)));
    }

    #[test]
    fn close_category_respects_threshold_rate() {
        let reg = test_registry();

        // ERA matchup_close_threshold = 1.00
        let close = make_cat("ERA", 3.50, 4.00, CategoryState::HomeWinning); // diff=0.50
        assert!(is_close_category(&close, Some(&reg)));

        let not_close = make_cat("ERA", 2.00, 5.50, CategoryState::HomeWinning); // diff=3.50
        assert!(!is_close_category(&not_close, Some(&reg)));
    }

    #[test]
    fn close_category_whip_threshold() {
        let reg = test_registry();

        // WHIP matchup_close_threshold = 0.20
        let close = make_cat("WHIP", 1.10, 1.25, CategoryState::HomeWinning); // diff=0.15
        assert!(is_close_category(&close, Some(&reg)));

        let not_close = make_cat("WHIP", 1.00, 1.50, CategoryState::HomeWinning); // diff=0.50
        assert!(!is_close_category(&not_close, Some(&reg)));
    }

    #[test]
    fn close_category_avg_threshold() {
        let reg = test_registry();

        // AVG matchup_close_threshold = 0.020
        let close = make_cat("AVG", 0.280, 0.295, CategoryState::AwayWinning); // diff=0.015
        assert!(is_close_category(&close, Some(&reg)));

        let not_close = make_cat("AVG", 0.250, 0.310, CategoryState::AwayWinning); // diff=0.060
        assert!(!is_close_category(&not_close, Some(&reg)));
    }

    #[test]
    fn close_category_fallback_without_registry() {
        // Without registry, uses hardcoded fallback
        let close = make_cat("HR", 2.0, 4.0, CategoryState::AwayWinning); // diff=2, threshold=3
        assert!(is_close_category(&close, None));

        let not_close = make_cat("HR", 2.0, 10.0, CategoryState::AwayWinning);
        assert!(!is_close_category(&not_close, None));
    }

    // -- Pace projection tests --

    #[test]
    fn pace_projection_counting_stat() {
        // 5 runs in 2 days, projecting over 12 days => 30
        let result = project_counting_stat(5.0, 2, 12);
        assert!((result - 30.0).abs() < 1e-10);
    }

    #[test]
    fn pace_projection_counting_stat_zero_elapsed() {
        let result = project_counting_stat(5.0, 0, 12);
        assert!((result - 0.0).abs() < 1e-10);
    }

    #[test]
    fn pace_projection_counting_stat_one_day() {
        // 3 HR in 1 day over 7-day period => 21
        let result = project_counting_stat(3.0, 1, 7);
        assert!((result - 21.0).abs() < 1e-10);
    }

    #[test]
    fn pace_projection_rate_stat_preserves_current() {
        let reg = test_registry();
        let era_def = reg.get("ERA").unwrap();

        // ERA of 3.50 should stay as 3.50 projection (no component data)
        let result = project_stat(3.50, 2, 12, Some(era_def));
        assert!((result - 3.50).abs() < 1e-10);
    }

    #[test]
    fn pace_projection_avg_preserves_current() {
        let reg = test_registry();
        let avg_def = reg.get("AVG").unwrap();

        let result = project_stat(0.275, 3, 12, Some(avg_def));
        assert!((result - 0.275).abs() < 1e-10);
    }

    #[test]
    fn project_stat_counting_with_definition() {
        let reg = test_registry();
        let hr_def = reg.get("HR").unwrap();

        // 2 HR in 2 days over 10 days => 10
        let result = project_stat(2.0, 2, 10, Some(hr_def));
        assert!((result - 10.0).abs() < 1e-10);
    }

    #[test]
    fn project_stat_zero_days_elapsed() {
        let result = project_stat(5.0, 0, 12, None);
        assert!((result - 0.0).abs() < 1e-10);
    }

    // -- Scroll tests --

    #[test]
    fn scroll_down_increments_offset() {
        let mut panel = MatchupAnalyticsPanel::new();
        panel.update(MatchupAnalyticsPanelMessage::Scroll(ScrollDirection::Down));
        assert_eq!(panel.scroll_offset(), 1);
    }

    #[test]
    fn scroll_returns_none() {
        let mut panel = MatchupAnalyticsPanel::new();
        let result = panel.update(MatchupAnalyticsPanelMessage::Scroll(ScrollDirection::Down));
        assert_eq!(result, None);
    }

    // -- View smoke tests --

    #[test]
    fn view_does_not_panic_empty_data() {
        let backend = ratatui::backend::TestBackend::new(80, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = MatchupAnalyticsPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], &[], 0, None, "HT", "AT", false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_data() {
        let backend = ratatui::backend::TestBackend::new(100, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = MatchupAnalyticsPanel::new();
        let reg = test_registry();
        let scores = vec![
            make_cat("R", 5.0, 3.0, CategoryState::HomeWinning),
            make_cat("HR", 2.0, 3.0, CategoryState::AwayWinning),
            make_cat("ERA", 3.50, 4.20, CategoryState::HomeWinning),
        ];
        let days = vec![make_scoring_day("March 26"), make_scoring_day("March 27")];
        terminal
            .draw(|frame| {
                panel.view(
                    frame,
                    frame.area(),
                    &scores,
                    &days,
                    0,
                    Some(&reg),
                    "HT",
                    "AT",
                    false,
                )
            })
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_tiny_area() {
        let backend = ratatui::backend::TestBackend::new(5, 3);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = MatchupAnalyticsPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], &[], 0, None, "HT", "AT", false))
            .unwrap();
    }

    // -- Format helpers --

    #[test]
    fn format_value_counting() {
        assert_eq!(format_value(5.0, 0), "5");
        assert_eq!(format_value(42.0, 0), "42");
    }

    #[test]
    fn format_value_rate() {
        assert_eq!(format_value(0.275, 3), "0.275");
        assert_eq!(format_value(3.50, 2), "3.50");
    }

    #[test]
    fn format_signed_value_positive() {
        assert_eq!(format_signed_value(2.0, 0), "+2");
        assert_eq!(format_signed_value(0.015, 3), "+0.015");
    }

    #[test]
    fn format_signed_value_negative() {
        assert_eq!(format_signed_value(-1.0, 0), "-1");
        assert_eq!(format_signed_value(-0.70, 2), "-0.70");
    }

    #[test]
    fn format_signed_value_zero() {
        assert_eq!(format_signed_value(0.0, 0), "+0");
        assert_eq!(format_signed_value(0.0, 3), "+0.000");
    }

    // -- Test helpers --

    fn make_scoring_day(label: &str) -> ScoringDay {
        ScoringDay {
            date: "2026-03-26".to_string(),
            label: label.to_string(),
            batting_stat_columns: vec![],
            pitching_stat_columns: vec![],
            home: TeamDailyRoster::default(),
            away: TeamDailyRoster::default(),
        }
    }
}
