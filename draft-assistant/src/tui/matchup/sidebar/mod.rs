// Matchup sidebar: category tracker and limits panels.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::subscription::Subscription;
use crate::tui::subscription::keybinding::KeybindManager;

// ---------------------------------------------------------------------------
// Stub panels
// ---------------------------------------------------------------------------

/// Category tracker panel (stub — will be implemented in a later task).
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

    pub fn view(&self, frame: &mut Frame, area: Rect, _focused: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Category Tracker ")
            .border_style(Style::default().fg(Color::DarkGray));
        let text = Paragraph::new(Line::from("Category tracker coming soon..."))
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(text, area);
    }
}

impl Default for CategoryTrackerPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Limits panel (stub — will be implemented in a later task).
pub struct LimitsPanel {
    scroll: ScrollState,
}

/// Message type for the limits panel.
#[derive(Debug, Clone)]
pub enum LimitsPanelMessage {
    Scroll(ScrollDirection),
}

impl LimitsPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    pub fn update(&mut self, msg: LimitsPanelMessage) -> Option<Action> {
        match msg {
            LimitsPanelMessage::Scroll(dir) => {
                self.scroll.scroll(dir, 10);
                None
            }
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    pub fn view(&self, frame: &mut Frame, area: Rect, _focused: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Limits & Resources ")
            .border_style(Style::default().fg(Color::DarkGray));
        let text = Paragraph::new(Line::from("Limits info coming soon..."))
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(text, area);
    }
}

impl Default for LimitsPanel {
    fn default() -> Self {
        Self::new()
    }
}

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

    pub fn view(
        &self,
        frame: &mut Frame,
        category_area: Rect,
        limits_area: Rect,
        category_focused: bool,
        limits_focused: bool,
    ) {
        self.category_tracker
            .view(frame, category_area, category_focused);
        self.limits_panel.view(frame, limits_area, limits_focused);
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
