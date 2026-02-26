// Screen layout: panel arrangement and sizing.
//
// Divides the terminal area into fixed zones for the draft dashboard:
//
// +--------------------------------------------------+
// | Status Bar (1 row)                                |
// +--------------------------------------------------+
// | Nomination Banner (4 rows)                        |
// +-------------------------+------------------------+
// | Main Panel (65%)         | Sidebar (35%)          |
// |                          | +- Roster (45%) ------+|
// |                          | +- Scarcity (35%) ----+|
// |                          | +- Budget (20%) ------+|
// +-------------------------+------------------------+
// | Help Bar (1 row)                                  |
// +--------------------------------------------------+

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Resolved screen areas for each dashboard zone.
#[derive(Debug, Clone)]
pub struct AppLayout {
    /// Top row: connection status, draft progress, pick counter.
    pub status_bar: Rect,
    /// Second row: current nomination details (player, bid, timer).
    pub nomination_banner: Rect,
    /// Left side of the middle section: tab-switched content area.
    pub main_panel: Rect,
    /// Right sidebar top: user's roster.
    pub roster: Rect,
    /// Right sidebar middle: positional scarcity index.
    pub scarcity: Rect,
    /// Right sidebar bottom: budget/inflation summary.
    pub budget: Rect,
    /// Bottom row: keyboard shortcut hints.
    pub help_bar: Rect,
}

/// Build the dashboard layout from the available terminal area.
///
/// The layout uses fixed heights for the status bar, nomination banner,
/// and help bar, with the remaining space split between the main panel
/// and a sidebar column.
pub fn build_layout(area: Rect) -> AppLayout {
    // Vertical: status(1) | nomination(4) | middle(fill) | help(1)
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // status bar
            Constraint::Length(4),  // nomination banner
            Constraint::Min(10),   // middle section (main + sidebar)
            Constraint::Length(1),  // help bar
        ])
        .split(area);

    let status_bar = vertical[0];
    let nomination_banner = vertical[1];
    let middle = vertical[2];
    let help_bar = vertical[3];

    // Horizontal: main panel (65%) | sidebar (35%)
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(65),
            Constraint::Percentage(35),
        ])
        .split(middle);

    let main_panel = horizontal[0];
    let sidebar = horizontal[1];

    // Sidebar vertical: roster (45%) | scarcity (35%) | budget (20%)
    let sidebar_sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(45),
            Constraint::Percentage(35),
            Constraint::Percentage(20),
        ])
        .split(sidebar);

    let roster = sidebar_sections[0];
    let scarcity = sidebar_sections[1];
    let budget = sidebar_sections[2];

    AppLayout {
        status_bar,
        nomination_banner,
        main_panel,
        roster,
        scarcity,
        budget,
        help_bar,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A reasonable terminal size for testing.
    fn test_area() -> Rect {
        Rect::new(0, 0, 160, 50)
    }

    #[test]
    fn layout_all_rects_nonzero() {
        let layout = build_layout(test_area());
        let rects = [
            ("status_bar", layout.status_bar),
            ("nomination_banner", layout.nomination_banner),
            ("main_panel", layout.main_panel),
            ("roster", layout.roster),
            ("scarcity", layout.scarcity),
            ("budget", layout.budget),
            ("help_bar", layout.help_bar),
        ];
        for (name, rect) in &rects {
            assert!(
                rect.width > 0 && rect.height > 0,
                "{} has zero area: {:?}",
                name,
                rect
            );
        }
    }

    #[test]
    fn layout_status_bar_height_is_one() {
        let layout = build_layout(test_area());
        assert_eq!(
            layout.status_bar.height, 1,
            "Status bar should be exactly 1 row"
        );
    }

    #[test]
    fn layout_help_bar_height_is_one() {
        let layout = build_layout(test_area());
        assert_eq!(
            layout.help_bar.height, 1,
            "Help bar should be exactly 1 row"
        );
    }

    #[test]
    fn layout_nomination_banner_height_is_four() {
        let layout = build_layout(test_area());
        assert_eq!(
            layout.nomination_banner.height, 4,
            "Nomination banner should be exactly 4 rows"
        );
    }

    #[test]
    fn layout_main_panel_wider_than_sidebar() {
        let layout = build_layout(test_area());
        assert!(
            layout.main_panel.width > layout.roster.width,
            "Main panel ({}) should be wider than sidebar ({})",
            layout.main_panel.width,
            layout.roster.width
        );
    }

    #[test]
    fn layout_sidebar_sections_stack_vertically() {
        let layout = build_layout(test_area());
        // Roster should be above scarcity
        assert!(
            layout.roster.y < layout.scarcity.y,
            "Roster should be above scarcity"
        );
        // Scarcity should be above budget
        assert!(
            layout.scarcity.y < layout.budget.y,
            "Scarcity should be above budget"
        );
    }

    #[test]
    fn layout_sidebar_sections_same_width() {
        let layout = build_layout(test_area());
        assert_eq!(
            layout.roster.width, layout.scarcity.width,
            "Sidebar sections should have the same width"
        );
        assert_eq!(
            layout.scarcity.width, layout.budget.width,
            "Sidebar sections should have the same width"
        );
    }

    #[test]
    fn layout_fits_within_area() {
        let area = test_area();
        let layout = build_layout(area);
        let all_rects = [
            layout.status_bar,
            layout.nomination_banner,
            layout.main_panel,
            layout.roster,
            layout.scarcity,
            layout.budget,
            layout.help_bar,
        ];
        for rect in &all_rects {
            assert!(
                rect.x + rect.width <= area.width,
                "Rect {:?} exceeds area width {}",
                rect,
                area.width
            );
            assert!(
                rect.y + rect.height <= area.height,
                "Rect {:?} exceeds area height {}",
                rect,
                area.height
            );
        }
    }

    #[test]
    fn layout_small_terminal_still_valid() {
        // Minimum viable terminal size
        let area = Rect::new(0, 0, 40, 16);
        let layout = build_layout(area);
        // All zones should still get some area
        let rects = [
            layout.status_bar,
            layout.nomination_banner,
            layout.main_panel,
            layout.roster,
            layout.scarcity,
            layout.budget,
            layout.help_bar,
        ];
        for rect in &rects {
            assert!(
                rect.width > 0 && rect.height > 0,
                "Small terminal: rect {:?} has zero area",
                rect
            );
        }
    }
}
