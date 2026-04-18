// Matchup screen layout: panel arrangement and sizing.
//
// Divides the terminal area into fixed zones for the matchup page:
//
// +--------------------------------------------------+
// | Status Bar (1 row)                                |
// +--------------------------------------------------+
// | Scoreboard (5 rows)                               |
// +-------------------------+------------------------+
// | Main Panel (65%)         | Sidebar (35%)          |
// |                          |  Category Tracker      |
// +-------------------------+------------------------+
// | Help Bar (1 row)                                  |
// +--------------------------------------------------+
//
// When terminal width < 100 columns, the sidebar is hidden.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Resolved screen areas for the matchup dashboard.
#[derive(Debug, Clone)]
pub struct MatchupLayout {
    /// Top row: matchup period, team names, day counter.
    pub status_bar: Rect,
    /// Category scoreboard: head-to-head stat comparison.
    pub scoreboard: Rect,
    /// Tab-switched content area (daily stats, analytics, rosters).
    pub main_panel: Rect,
    /// Right sidebar: category tracker. `None` if terminal < 100 cols.
    pub sidebar: Option<Rect>,
    /// Bottom row: keyboard shortcut hints.
    pub help_bar: Rect,
}

/// Build the matchup layout from the available terminal area.
pub fn build_matchup_layout(area: Rect) -> MatchupLayout {
    // Vertical: status(1) | scoreboard(5) | content(fill) | help(1)
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // status bar
            Constraint::Length(5), // scoreboard
            Constraint::Min(10),   // content area
            Constraint::Length(1), // help bar
        ])
        .split(area);

    let status_bar = vertical[0];
    let scoreboard = vertical[1];
    let content = vertical[2];
    let help_bar = vertical[3];

    // Horizontal: main(65%) | sidebar(35%) if wide enough
    let (main_panel, sidebar) = if area.width >= 100 {
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(content);
        (horizontal[0], Some(horizontal[1]))
    } else {
        (content, None)
    };

    MatchupLayout {
        status_bar,
        scoreboard,
        main_panel,
        sidebar,
        help_bar,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn wide_area() -> Rect {
        Rect::new(0, 0, 160, 50)
    }

    fn narrow_area() -> Rect {
        Rect::new(0, 0, 80, 30)
    }

    #[test]
    fn layout_all_rects_nonzero_wide() {
        let layout = build_matchup_layout(wide_area());
        for (name, rect) in [
            ("status_bar", layout.status_bar),
            ("scoreboard", layout.scoreboard),
            ("main_panel", layout.main_panel),
            ("help_bar", layout.help_bar),
        ] {
            assert!(
                rect.width > 0 && rect.height > 0,
                "{name} has zero area: {rect:?}",
            );
        }
        assert!(layout.sidebar.is_some(), "Wide terminal should have sidebar");
        let sidebar = layout.sidebar.unwrap();
        assert!(
            sidebar.width > 0 && sidebar.height > 0,
            "sidebar has zero area: {sidebar:?}",
        );
    }

    #[test]
    fn layout_narrow_hides_sidebar() {
        let layout = build_matchup_layout(narrow_area());
        assert!(
            layout.sidebar.is_none(),
            "Narrow terminal should hide sidebar"
        );
    }

    #[test]
    fn layout_status_bar_height_is_one() {
        let layout = build_matchup_layout(wide_area());
        assert_eq!(layout.status_bar.height, 1);
    }

    #[test]
    fn layout_scoreboard_height_is_five() {
        let layout = build_matchup_layout(wide_area());
        assert_eq!(layout.scoreboard.height, 5);
    }

    #[test]
    fn layout_help_bar_height_is_one() {
        let layout = build_matchup_layout(wide_area());
        assert_eq!(layout.help_bar.height, 1);
    }

    #[test]
    fn layout_main_panel_wider_than_sidebar() {
        let layout = build_matchup_layout(wide_area());
        let sidebar = layout.sidebar.unwrap();
        assert!(
            layout.main_panel.width > sidebar.width,
            "Main panel ({}) should be wider than sidebar ({})",
            layout.main_panel.width,
            sidebar.width,
        );
    }

    #[test]
    fn layout_fits_within_area() {
        let area = wide_area();
        let layout = build_matchup_layout(area);
        for rect in [
            layout.status_bar,
            layout.scoreboard,
            layout.main_panel,
            layout.help_bar,
        ] {
            assert!(
                rect.x + rect.width <= area.width,
                "Rect {rect:?} exceeds area width {}",
                area.width,
            );
            assert!(
                rect.y + rect.height <= area.height,
                "Rect {rect:?} exceeds area height {}",
                area.height,
            );
        }
    }

    #[test]
    fn layout_small_terminal_still_valid() {
        let area = Rect::new(0, 0, 40, 16);
        let layout = build_matchup_layout(area);
        assert!(layout.status_bar.width > 0 && layout.status_bar.height > 0);
        assert!(layout.main_panel.width > 0 && layout.main_panel.height > 0);
        assert!(layout.help_bar.width > 0 && layout.help_bar.height > 0);
    }
}
