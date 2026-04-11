// Matchup sidebar: category tracker and limits panels.

pub mod category_tracker;
pub mod limits;

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::matchup::CategoryScore;
use crate::tui::action::Action;
use crate::tui::subscription::Subscription;
use crate::tui::subscription::keybinding::KeybindManager;

pub use category_tracker::{CategoryTrackerPanel, CategoryTrackerPanelMessage};
pub use limits::{LimitsData, LimitsPanel, LimitsPanelMessage};

// ---------------------------------------------------------------------------
// MatchupSidebar
// ---------------------------------------------------------------------------

/// Messages that can be sent to the matchup sidebar.
#[derive(Debug, Clone)]
pub enum MatchupSidebarMessage {
    CategoryTracker(CategoryTrackerPanelMessage),
    Limits(LimitsPanelMessage),
}

/// Matchup sidebar component: category tracker + limits.
pub struct MatchupSidebar {
    pub category_tracker: CategoryTrackerPanel,
    pub limits_panel: LimitsPanel,
}

impl MatchupSidebar {
    pub fn new() -> Self {
        Self {
            category_tracker: CategoryTrackerPanel::new(),
            limits_panel: LimitsPanel::new(),
        }
    }

    pub fn subscription(&self, _kb: &mut KeybindManager) -> Subscription<MatchupSidebarMessage> {
        Subscription::none()
    }

    pub fn update(&mut self, msg: MatchupSidebarMessage) -> Option<Action> {
        match msg {
            MatchupSidebarMessage::CategoryTracker(m) => self.category_tracker.update(m),
            MatchupSidebarMessage::Limits(m) => self.limits_panel.update(m),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &self,
        frame: &mut Frame,
        category_area: Rect,
        limits_area: Rect,
        category_scores: &[CategoryScore],
        limits_data: &LimitsData,
        category_focused: bool,
        limits_focused: bool,
    ) {
        self.category_tracker
            .view(frame, category_area, category_scores, category_focused);
        self.limits_panel
            .view(frame, limits_area, limits_data, limits_focused);
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

    #[test]
    fn limits_scroll_delegates() {
        let mut sidebar = MatchupSidebar::new();
        sidebar.update(MatchupSidebarMessage::Limits(
            LimitsPanelMessage::Scroll(ScrollDirection::Down),
        ));
        assert_eq!(sidebar.limits_panel.scroll_offset(), 1);
    }
}
