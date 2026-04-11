// Matchup main panel: tab container for daily stats, analytics, and roster views.

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
// MatchupTab
// ---------------------------------------------------------------------------

/// Tab identifiers for the matchup main panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchupTab {
    DailyStats,
    Analytics,
    MyRoster,
    OppRoster,
}

// ---------------------------------------------------------------------------
// Stub panels
// ---------------------------------------------------------------------------

/// Daily stats panel (stub — will be implemented in a later task).
pub struct DailyStatsPanel {
    scroll: ScrollState,
}

/// Message type for the daily stats panel.
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

    pub fn view(&self, frame: &mut Frame, area: Rect, _focused: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Daily Stats ")
            .border_style(Style::default().fg(Color::DarkGray));
        let text = Paragraph::new(Line::from("Daily stats coming soon..."))
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

/// Analytics panel (stub — will be implemented in a later task).
pub struct MatchupAnalyticsPanel {
    scroll: ScrollState,
}

/// Message type for the analytics panel.
#[derive(Debug, Clone)]
pub enum MatchupAnalyticsPanelMessage {
    Scroll(ScrollDirection),
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

    pub fn view(&self, frame: &mut Frame, area: Rect, _focused: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Analytics ")
            .border_style(Style::default().fg(Color::DarkGray));
        let text = Paragraph::new(Line::from("Matchup analytics coming soon..."))
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(text, area);
    }
}

impl Default for MatchupAnalyticsPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Roster view panel (stub — reused for both My Roster and Opp Roster tabs).
pub struct RosterViewPanel {
    scroll: ScrollState,
}

/// Message type for the roster view panel.
#[derive(Debug, Clone)]
pub enum RosterViewPanelMessage {
    Scroll(ScrollDirection),
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
                self.scroll.scroll(dir, 20);
                None
            }
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    pub fn view(&self, frame: &mut Frame, area: Rect, title: &str, _focused: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {title} "))
            .border_style(Style::default().fg(Color::DarkGray));
        let text = Paragraph::new(Line::from("Roster view coming soon..."))
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(text, area);
    }
}

impl Default for RosterViewPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// MatchupMainPanel
// ---------------------------------------------------------------------------

/// Messages handled by the matchup main panel.
#[derive(Debug, Clone)]
pub enum MatchupMainPanelMessage {
    DailyStats(DailyStatsPanelMessage),
    Analytics(MatchupAnalyticsPanelMessage),
    MyRoster(RosterViewPanelMessage),
    OppRoster(RosterViewPanelMessage),
}

/// Mid-level component composing the four matchup tab panels.
pub struct MatchupMainPanel {
    pub active_tab: MatchupTab,
    pub daily_panel: DailyStatsPanel,
    pub analytics_panel: MatchupAnalyticsPanel,
    pub my_roster_panel: RosterViewPanel,
    pub opp_roster_panel: RosterViewPanel,
}

impl MatchupMainPanel {
    pub fn new() -> Self {
        Self {
            active_tab: MatchupTab::DailyStats,
            daily_panel: DailyStatsPanel::new(),
            analytics_panel: MatchupAnalyticsPanel::new(),
            my_roster_panel: RosterViewPanel::new(),
            opp_roster_panel: RosterViewPanel::new(),
        }
    }

    pub fn active_tab(&self) -> MatchupTab {
        self.active_tab
    }

    pub fn subscription(&self, _kb: &mut KeybindManager) -> Subscription<MatchupMainPanelMessage> {
        // No child subscriptions yet.
        Subscription::none()
    }

    pub fn update(&mut self, msg: MatchupMainPanelMessage) -> Option<Action> {
        match msg {
            MatchupMainPanelMessage::DailyStats(m) => self.daily_panel.update(m),
            MatchupMainPanelMessage::Analytics(m) => self.analytics_panel.update(m),
            MatchupMainPanelMessage::MyRoster(m) => self.my_roster_panel.update(m),
            MatchupMainPanelMessage::OppRoster(m) => self.opp_roster_panel.update(m),
        }
    }

    pub fn view(&self, frame: &mut Frame, area: Rect, focused: bool) {
        match self.active_tab {
            MatchupTab::DailyStats => self.daily_panel.view(frame, area, focused),
            MatchupTab::Analytics => self.analytics_panel.view(frame, area, focused),
            MatchupTab::MyRoster => {
                self.my_roster_panel.view(frame, area, "My Roster", focused);
            }
            MatchupTab::OppRoster => {
                self.opp_roster_panel.view(frame, area, "Opponent Roster", focused);
            }
        }
    }
}

impl Default for MatchupMainPanel {
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
    fn new_starts_on_daily_stats_tab() {
        let panel = MatchupMainPanel::new();
        assert_eq!(panel.active_tab(), MatchupTab::DailyStats);
    }

    #[test]
    fn daily_stats_scroll_delegates() {
        let mut panel = MatchupMainPanel::new();
        panel.update(MatchupMainPanelMessage::DailyStats(
            DailyStatsPanelMessage::Scroll(ScrollDirection::Down),
        ));
        assert_eq!(panel.daily_panel.scroll_offset(), 1);
    }

    #[test]
    fn analytics_scroll_delegates() {
        let mut panel = MatchupMainPanel::new();
        panel.update(MatchupMainPanelMessage::Analytics(
            MatchupAnalyticsPanelMessage::Scroll(ScrollDirection::Down),
        ));
        assert_eq!(panel.analytics_panel.scroll_offset(), 1);
    }

    #[test]
    fn my_roster_scroll_delegates() {
        let mut panel = MatchupMainPanel::new();
        panel.update(MatchupMainPanelMessage::MyRoster(
            RosterViewPanelMessage::Scroll(ScrollDirection::Down),
        ));
        assert_eq!(panel.my_roster_panel.scroll_offset(), 1);
    }

    #[test]
    fn opp_roster_scroll_delegates() {
        let mut panel = MatchupMainPanel::new();
        panel.update(MatchupMainPanelMessage::OppRoster(
            RosterViewPanelMessage::Scroll(ScrollDirection::Down),
        ));
        assert_eq!(panel.opp_roster_panel.scroll_offset(), 1);
    }

    #[test]
    fn view_does_not_panic_daily_stats() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = MatchupMainPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_analytics() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MatchupMainPanel::new();
        panel.active_tab = MatchupTab::Analytics;
        terminal
            .draw(|frame| panel.view(frame, frame.area(), false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_my_roster() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MatchupMainPanel::new();
        panel.active_tab = MatchupTab::MyRoster;
        terminal
            .draw(|frame| panel.view(frame, frame.area(), false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_opp_roster() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MatchupMainPanel::new();
        panel.active_tab = MatchupTab::OppRoster;
        terminal
            .draw(|frame| panel.view(frame, frame.area(), false))
            .unwrap();
    }
}
