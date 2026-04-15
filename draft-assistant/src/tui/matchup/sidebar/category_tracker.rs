// Category tracker panel: visual bars showing relative position in each
// scoring category, grouped by batting and pitching.
//
// Bars render home share on the left and away share on the right.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::matchup::{CategoryScore, CategoryState};
use crate::stats::{PlayerType, SortDirection, lookup_stat_definition};
use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::widgets::focused_border_style;

// ---------------------------------------------------------------------------
// CategoryTrackerPanel
// ---------------------------------------------------------------------------

/// Category tracker panel showing visual bars for each scoring category.
pub struct CategoryTrackerPanel {
    scroll: ScrollState,
}

/// Message type for the category tracker panel.
#[derive(Debug, Clone)]
pub enum CategoryTrackerPanelMessage {
    Scroll(ScrollDirection),
}

impl CategoryTrackerPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    pub fn update(&mut self, msg: CategoryTrackerPanelMessage) -> Option<Action> {
        match msg {
            CategoryTrackerPanelMessage::Scroll(dir) => {
                self.scroll.scroll(dir, 10);
                None
            }
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    pub fn view(
        &self,
        frame: &mut Frame,
        area: Rect,
        category_scores: &[CategoryScore],
        focused: bool,
    ) {
        let border = focused_border_style(focused, Style::default());
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Category Tracker ")
            .border_style(border);

        if category_scores.is_empty() {
            let text = Paragraph::new(Line::from("Waiting for data..."))
                .style(Style::default().fg(Color::DarkGray))
                .block(block);
            frame.render_widget(text, area);
            return;
        }

        // Partition categories into batting and pitching
        let mut batting: Vec<&CategoryScore> = Vec::new();
        let mut pitching: Vec<&CategoryScore> = Vec::new();
        for cs in category_scores {
            let player_type = infer_player_type(&cs.stat_abbrev);
            match player_type {
                PlayerType::Hitter => batting.push(cs),
                PlayerType::Pitcher => pitching.push(cs),
            }
        }

        // Available width for bar content (inside borders)
        let inner_width = area.width.saturating_sub(2) as usize;
        let viewport_height = area.height.saturating_sub(2) as usize;

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Batting section
        if !batting.is_empty() {
            lines.push(Line::from(Span::styled(
                " Batting",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )));
            for cs in &batting {
                lines.push(build_category_line(cs, inner_width));
            }
        }

        // Pitching section
        if !pitching.is_empty() {
            if !batting.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                " Pitching",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )));
            for cs in &pitching {
                lines.push(build_category_line(cs, inner_width));
            }
        }

        let content_height = lines.len();
        let scroll_offset = self.scroll.clamped_offset(content_height, viewport_height);

        let paragraph = Paragraph::new(lines)
            .block(block)
            .scroll((scroll_offset as u16, 0));
        frame.render_widget(paragraph, area);

        // Scrollbar
        if content_height > viewport_height {
            let mut scrollbar_state = ScrollbarState::new(content_height.saturating_sub(viewport_height))
                .position(scroll_offset);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None),
                area,
                &mut scrollbar_state,
            );
        }
    }
}

impl Default for CategoryTrackerPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Bar rendering helpers
// ---------------------------------------------------------------------------

/// Compute the home-side proportion for the visual bar.
///
/// For `HigherIsBetter` stats: `home_share = home / (home + away)` — a higher
/// home value means the home team is ahead (larger green bar on the left).
/// For `LowerIsBetter` stats: `home_share = away / (home + away)` — a lower
/// home value means the home team is ahead.
/// Both zero: 0.5 (50/50 split).
pub(crate) fn compute_home_share(home_value: f64, away_value: f64, lower_is_better: bool) -> f64 {
    let total = home_value.abs() + away_value.abs();
    if total == 0.0 {
        return 0.5;
    }
    if lower_is_better {
        away_value.abs() / total
    } else {
        home_value.abs() / total
    }
}

/// Format the differential between home and away values (home - away).
fn format_diff(cs: &CategoryScore) -> String {
    let diff = cs.home_value - cs.away_value;
    let precision = infer_format_precision(&cs.stat_abbrev);

    if precision == 0 {
        let d = diff as i64;
        if d > 0 {
            format!("+{d}")
        } else if d < 0 {
            format!("{d}")
        } else {
            "0".to_string()
        }
    } else if diff > 0.0 {
        format!("+{diff:.prec$}", prec = precision as usize)
    } else if diff < 0.0 {
        format!("{diff:.prec$}", prec = precision as usize)
    } else {
        format!("{:.prec$}", 0.0, prec = precision as usize)
    }
}

/// Build a single category line with visual bar.
fn build_category_line(cs: &CategoryScore, available_width: usize) -> Line<'static> {
    let abbrev = &cs.stat_abbrev;

    // Layout: " ABBR ████░░░░ +diff STATUS "
    let label_width = 6; // " ABBR " (1 space + 4 chars + 1 space)
    let diff_str = format_diff(cs);
    let status_str = match cs.state {
        CategoryState::HomeWinning => "HOME",
        CategoryState::AwayWinning => "AWAY",
        CategoryState::Tied => "TIED",
    };
    let right_str = format!(" {} {}", diff_str, status_str);
    let right_width = right_str.len();

    let bar_width = available_width
        .saturating_sub(label_width)
        .saturating_sub(right_width);

    let lower_is_better = is_lower_is_better(abbrev);
    let home_share = compute_home_share(cs.home_value, cs.away_value, lower_is_better);
    let filled = ((bar_width as f64 * home_share).round() as usize).min(bar_width);
    let empty = bar_width.saturating_sub(filled);

    // Home bar is green when home is winning, red when home is losing.
    // Away fill uses the opposite color, so both cells always show the
    // winner in green.
    let (home_color, away_color) = match cs.state {
        CategoryState::HomeWinning => (Color::Green, Color::Red),
        CategoryState::AwayWinning => (Color::Red, Color::Green),
        CategoryState::Tied => (Color::Yellow, Color::Yellow),
    };

    let status_color = match cs.state {
        CategoryState::HomeWinning => Color::Green,
        CategoryState::AwayWinning => Color::Red,
        CategoryState::Tied => Color::Yellow,
    };

    let label = format!(" {:<4} ", abbrev);

    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::White)),
        Span::styled(
            "\u{2588}".repeat(filled),
            Style::default().fg(home_color),
        ),
        Span::styled(
            "\u{2591}".repeat(empty),
            Style::default().fg(away_color),
        ),
        Span::styled(
            format!(" {}", diff_str),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!(" {}", status_str),
            Style::default().fg(status_color),
        ),
    ])
}

/// Infer player type from stat abbreviation.
fn infer_player_type(abbrev: &str) -> PlayerType {
    // Try hitter first, then pitcher
    if lookup_stat_definition(abbrev, PlayerType::Hitter).is_some() {
        PlayerType::Hitter
    } else {
        PlayerType::Pitcher
    }
}

/// Check if a stat is "lower is better" (ERA, WHIP).
fn is_lower_is_better(abbrev: &str) -> bool {
    // Check both player types
    for pt in [PlayerType::Hitter, PlayerType::Pitcher] {
        if let Some(def) = lookup_stat_definition(abbrev, pt) {
            return def.sort_direction == SortDirection::LowerIsBetter;
        }
    }
    false
}

/// Infer format precision from stat abbreviation.
fn infer_format_precision(abbrev: &str) -> u8 {
    for pt in [PlayerType::Hitter, PlayerType::Pitcher] {
        if let Some(def) = lookup_stat_definition(abbrev, pt) {
            return def.format_precision;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_share_higher_is_better_home_winning() {
        // Home has 8, away has 2 => home_share = 8/10 = 0.8
        let share = compute_home_share(8.0, 2.0, false);
        assert!((share - 0.8).abs() < 1e-9);
    }

    #[test]
    fn home_share_higher_is_better_away_winning() {
        // Home has 2, away has 8 => home_share = 2/10 = 0.2
        let share = compute_home_share(2.0, 8.0, false);
        assert!((share - 0.2).abs() < 1e-9);
    }

    #[test]
    fn home_share_lower_is_better_home_winning() {
        // ERA: home has 2.50, away has 4.50 => lower is better for home
        // home_share = away / total = 4.5/7.0 ≈ 0.643
        let share = compute_home_share(2.50, 4.50, true);
        assert!((share - 4.5 / 7.0).abs() < 1e-9);
    }

    #[test]
    fn home_share_lower_is_better_away_winning() {
        // ERA: home has 4.50, away has 2.50 => lower is better for away
        // home_share = away / total = 2.5/7.0 ≈ 0.357
        let share = compute_home_share(4.50, 2.50, true);
        assert!((share - 2.5 / 7.0).abs() < 1e-9);
    }

    #[test]
    fn home_share_both_zero() {
        let share = compute_home_share(0.0, 0.0, false);
        assert!((share - 0.5).abs() < 1e-9);

        let share_lower = compute_home_share(0.0, 0.0, true);
        assert!((share_lower - 0.5).abs() < 1e-9);
    }

    #[test]
    fn home_share_equal_values() {
        let share = compute_home_share(5.0, 5.0, false);
        assert!((share - 0.5).abs() < 1e-9);
    }

    #[test]
    fn format_diff_positive_counting() {
        let cs = CategoryScore {
            stat_abbrev: "R".to_string(),
            home_value: 10.0,
            away_value: 7.0,
            my_value: 10.0,
            opp_value: 7.0,
            state: CategoryState::HomeWinning,
        };
        assert_eq!(format_diff(&cs), "+3");
    }

    #[test]
    fn format_diff_negative_counting() {
        let cs = CategoryScore {
            stat_abbrev: "HR".to_string(),
            home_value: 3.0,
            away_value: 5.0,
            my_value: 3.0,
            opp_value: 5.0,
            state: CategoryState::AwayWinning,
        };
        assert_eq!(format_diff(&cs), "-2");
    }

    #[test]
    fn format_diff_rate_stat() {
        let cs = CategoryScore {
            stat_abbrev: "AVG".to_string(),
            home_value: 0.280,
            away_value: 0.265,
            my_value: 0.280,
            opp_value: 0.265,
            state: CategoryState::HomeWinning,
        };
        let diff = format_diff(&cs);
        assert!(diff.starts_with('+'));
        assert!(diff.contains("0.015"));
    }

    #[test]
    fn format_diff_era() {
        let cs = CategoryScore {
            stat_abbrev: "ERA".to_string(),
            home_value: 3.00,
            away_value: 3.50,
            my_value: 3.00,
            opp_value: 3.50,
            state: CategoryState::HomeWinning,
        };
        let diff = format_diff(&cs);
        // diff is -0.50 (home ERA is lower = good for home)
        assert!(diff.starts_with('-'));
    }

    #[test]
    fn format_diff_tied() {
        let cs = CategoryScore {
            stat_abbrev: "R".to_string(),
            home_value: 5.0,
            away_value: 5.0,
            my_value: 5.0,
            opp_value: 5.0,
            state: CategoryState::Tied,
        };
        assert_eq!(format_diff(&cs), "0");
    }

    #[test]
    fn infer_player_type_batting() {
        assert_eq!(infer_player_type("R"), PlayerType::Hitter);
        assert_eq!(infer_player_type("HR"), PlayerType::Hitter);
        assert_eq!(infer_player_type("AVG"), PlayerType::Hitter);
    }

    #[test]
    fn infer_player_type_pitching() {
        assert_eq!(infer_player_type("K"), PlayerType::Pitcher);
        assert_eq!(infer_player_type("ERA"), PlayerType::Pitcher);
        assert_eq!(infer_player_type("WHIP"), PlayerType::Pitcher);
    }

    #[test]
    fn is_lower_is_better_for_era_whip() {
        assert!(is_lower_is_better("ERA"));
        assert!(is_lower_is_better("WHIP"));
    }

    #[test]
    fn is_not_lower_is_better_for_counting() {
        assert!(!is_lower_is_better("R"));
        assert!(!is_lower_is_better("HR"));
        assert!(!is_lower_is_better("K"));
    }

    #[test]
    fn view_does_not_panic_empty_scores() {
        let backend = ratatui::backend::TestBackend::new(60, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = CategoryTrackerPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_scores() {
        let backend = ratatui::backend::TestBackend::new(60, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = CategoryTrackerPanel::new();
        let scores = vec![
            CategoryScore {
                stat_abbrev: "R".to_string(),
                home_value: 10.0,
                away_value: 7.0,
                my_value: 10.0,
                opp_value: 7.0,
                state: CategoryState::HomeWinning,
            },
            CategoryScore {
                stat_abbrev: "ERA".to_string(),
                home_value: 3.00,
                away_value: 4.00,
                my_value: 3.00,
                opp_value: 4.00,
                state: CategoryState::HomeWinning,
            },
        ];
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &scores, true))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_narrow_area() {
        let backend = ratatui::backend::TestBackend::new(20, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = CategoryTrackerPanel::new();
        let scores = vec![CategoryScore {
            stat_abbrev: "R".to_string(),
            home_value: 10.0,
            away_value: 7.0,
            my_value: 10.0,
            opp_value: 7.0,
            state: CategoryState::HomeWinning,
        }];
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &scores, false))
            .unwrap();
    }
}
