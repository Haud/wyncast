// Matchup sidebar: category tracker panel.

pub mod category_tracker;

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::matchup::CategoryScore;
use crate::tui::action::Action;
use crate::tui::subscription::Subscription;
use crate::tui::subscription::keybinding::KeybindManager;

pub use category_tracker::{CategoryTrackerPanel, CategoryTrackerPanelMessage};

// ---------------------------------------------------------------------------
// MatchupSidebar
// ---------------------------------------------------------------------------

/// Messages that can be sent to the matchup sidebar.
#[derive(Debug, Clone)]
pub enum MatchupSidebarMessage {
    CategoryTracker(CategoryTrackerPanelMessage),
}

/// Matchup sidebar component: category tracker.
pub struct MatchupSidebar {
    pub category_tracker: CategoryTrackerPanel,
}

impl MatchupSidebar {
    pub fn new() -> Self {
        Self {
            category_tracker: CategoryTrackerPanel::new(),
        }
    }

    pub fn subscription(&self, _kb: &mut KeybindManager) -> Subscription<MatchupSidebarMessage> {
        Subscription::none()
    }

    pub fn update(&mut self, msg: MatchupSidebarMessage) -> Option<Action> {
        match msg {
            MatchupSidebarMessage::CategoryTracker(m) => self.category_tracker.update(m),
        }
    }

    pub fn view(
        &self,
        frame: &mut Frame,
        category_area: Rect,
        category_scores: &[CategoryScore],
        home_abbrev: &str,
        away_abbrev: &str,
        category_focused: bool,
    ) {
        self.category_tracker
            .view(frame, category_area, category_scores, home_abbrev, away_abbrev, category_focused);
    }
}

impl Default for MatchupSidebar {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::scroll::ScrollDirection;

    #[test]
    fn category_tracker_scroll_delegates() {
        let mut sidebar = MatchupSidebar::new();
        sidebar.update(MatchupSidebarMessage::CategoryTracker(
            CategoryTrackerPanelMessage::Scroll(ScrollDirection::Down),
        ));
        assert_eq!(sidebar.category_tracker.scroll_offset(), 1);
    }
}
