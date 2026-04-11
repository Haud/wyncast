// Matchup main panel: tab container for daily stats, analytics, and roster views.

pub mod analytics;
pub mod daily_stats;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::matchup::{CategoryScore, ScoringDay};
use crate::stats::StatRegistry;
use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::subscription::Subscription;
use crate::tui::subscription::keybinding::KeybindManager;

pub use analytics::{MatchupAnalyticsPanel, MatchupAnalyticsPanelMessage};
pub use daily_stats::{DailyStatsPanel, DailyStatsPanelMessage};

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

    /// Render the active tab's content.
    ///
    /// Analytics-specific data is passed through for the analytics panel.
    /// When the analytics tab is not active, these parameters are unused.
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &self,
        frame: &mut Frame,
        area: Rect,
        category_scores: &[CategoryScore],
        scoring_period_days: &[ScoringDay],
        selected_day: usize,
        games_started: u8,
        gs_limit: u8,
        acquisitions_used: u8,
        acquisitions_limit: u8,
        registry: Option<&StatRegistry>,
        focused: bool,
    ) {
        let current_day = scoring_period_days.get(selected_day);
        match self.active_tab {
            MatchupTab::DailyStats => {
                if let Some(day) = current_day {
                    self.daily_panel.view(frame, area, day, focused);
                } else {
                    self.daily_panel.view_placeholder(frame, area);
                }
            }
            MatchupTab::Analytics => self.analytics_panel.view(
                frame,
                area,
                category_scores,
                scoring_period_days,
                selected_day,
                games_started,
                gs_limit,
                acquisitions_used,
                acquisitions_limit,
                registry,
                focused,
            ),
            MatchupTab::MyRoster => {
                self.my_roster_panel.view(frame, area, "My Roster", focused);
            }
            MatchupTab::OppRoster => {
                self.opp_roster_panel
                    .view(frame, area, "Opponent Roster", focused);
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
    fn view_does_not_panic_daily_stats_no_data() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = MatchupMainPanel::new();
        terminal
            .draw(|frame| {
                panel.view(frame, frame.area(), &[], &[], 0, 0, 7, 0, 5, None, false)
            })
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_daily_stats_with_data() {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = MatchupMainPanel::new();
        let day = make_test_scoring_day();
        let days = vec![day];
        terminal
            .draw(|frame| {
                panel.view(frame, frame.area(), &[], &days, 0, 0, 7, 0, 5, None, false)
            })
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_analytics() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MatchupMainPanel::new();
        panel.active_tab = MatchupTab::Analytics;
        terminal
            .draw(|frame| {
                panel.view(frame, frame.area(), &[], &[], 0, 0, 7, 0, 5, None, false)
            })
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_my_roster() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MatchupMainPanel::new();
        panel.active_tab = MatchupTab::MyRoster;
        terminal
            .draw(|frame| {
                panel.view(frame, frame.area(), &[], &[], 0, 0, 7, 0, 5, None, false)
            })
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_opp_roster() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MatchupMainPanel::new();
        panel.active_tab = MatchupTab::OppRoster;
        terminal
            .draw(|frame| {
                panel.view(frame, frame.area(), &[], &[], 0, 0, 7, 0, 5, None, false)
            })
            .unwrap();
    }

    fn make_test_scoring_day() -> ScoringDay {
        use crate::matchup::{DailyPlayerRow, DailyTotals};
        ScoringDay {
            date: "2026-03-26".to_string(),
            label: "March 26".to_string(),
            batting_rows: vec![DailyPlayerRow {
                slot: "C".to_string(),
                player_name: "Ben Rice".to_string(),
                team: "NYY".to_string(),
                positions: vec!["C".to_string()],
                opponent: Some("@BOS".to_string()),
                game_status: None,
                stats: vec![Some(4.0), Some(1.0), Some(0.0)],
            }],
            pitching_rows: vec![],
            batting_totals: Some(DailyTotals {
                stats: vec![Some(4.0), Some(1.0), Some(0.0)],
            }),
            pitching_totals: None,
        }
    }
}
