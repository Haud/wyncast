// Matchup main panel: tab container for daily stats, analytics, and roster views.

pub mod analytics;
pub mod daily_stats;
pub mod roster_view;

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::matchup::{CategoryScore, ScoringDay, TeamSide};
use crate::stats::StatRegistry;
use crate::tui::action::Action;
use crate::tui::subscription::Subscription;
use crate::tui::subscription::keybinding::KeybindManager;

pub use analytics::{MatchupAnalyticsPanel, MatchupAnalyticsPanelMessage};
pub use daily_stats::{DailyStatsPanel, DailyStatsPanelMessage};
pub use roster_view::{RosterViewPanel, RosterViewPanelMessage};

// ---------------------------------------------------------------------------
// MatchupTab
// ---------------------------------------------------------------------------

/// Tab identifiers for the matchup main panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchupTab {
    DailyStats,
    Analytics,
    HomeRoster,
    AwayRoster,
}

// ---------------------------------------------------------------------------
// MatchupMainPanel
// ---------------------------------------------------------------------------

/// Messages handled by the matchup main panel.
#[derive(Debug, Clone)]
pub enum MatchupMainPanelMessage {
    DailyStats(DailyStatsPanelMessage),
    Analytics(MatchupAnalyticsPanelMessage),
    HomeRoster(RosterViewPanelMessage),
    AwayRoster(RosterViewPanelMessage),
}

/// Mid-level component composing the four matchup tab panels.
pub struct MatchupMainPanel {
    pub active_tab: MatchupTab,
    pub daily_panel: DailyStatsPanel,
    pub analytics_panel: MatchupAnalyticsPanel,
    pub home_roster_panel: RosterViewPanel,
    pub away_roster_panel: RosterViewPanel,
}

impl MatchupMainPanel {
    pub fn new() -> Self {
        Self {
            active_tab: MatchupTab::DailyStats,
            daily_panel: DailyStatsPanel::new(),
            analytics_panel: MatchupAnalyticsPanel::new(),
            home_roster_panel: RosterViewPanel::new(),
            away_roster_panel: RosterViewPanel::new(),
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
            MatchupMainPanelMessage::HomeRoster(m) => self.home_roster_panel.update(m),
            MatchupMainPanelMessage::AwayRoster(m) => self.away_roster_panel.update(m),
        }
    }

    /// Render the active tab's content.
    ///
    /// Analytics-specific data is passed through for the analytics panel.
    /// Roster views select the correct side via their tab identity.
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &self,
        frame: &mut Frame,
        area: Rect,
        category_scores: &[CategoryScore],
        scoring_period_days: &[ScoringDay],
        selected_day: usize,
        registry: Option<&StatRegistry>,
        home_team_name: &str,
        away_team_name: &str,
        home_abbrev: &str,
        away_abbrev: &str,
        focused: bool,
    ) {
        let current_day = scoring_period_days.get(selected_day);
        match self.active_tab {
            MatchupTab::DailyStats => {
                if let Some(day) = current_day {
                    self.daily_panel.view(frame, area, day, home_team_name, away_team_name, focused);
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
                registry,
                home_abbrev,
                away_abbrev,
                focused,
            ),
            MatchupTab::HomeRoster => {
                self.home_roster_panel.view(
                    frame,
                    area,
                    home_team_name,
                    scoring_period_days,
                    TeamSide::Home,
                    focused,
                );
            }
            MatchupTab::AwayRoster => {
                self.away_roster_panel.view(
                    frame,
                    area,
                    away_team_name,
                    scoring_period_days,
                    TeamSide::Away,
                    focused,
                );
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
    use crate::tui::scroll::ScrollDirection;

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
    fn home_roster_scroll_delegates() {
        let mut panel = MatchupMainPanel::new();
        panel.update(MatchupMainPanelMessage::HomeRoster(
            RosterViewPanelMessage::Scroll(ScrollDirection::Down),
        ));
        assert_eq!(panel.home_roster_panel.scroll_offset(), 1);
    }

    #[test]
    fn away_roster_scroll_delegates() {
        let mut panel = MatchupMainPanel::new();
        panel.update(MatchupMainPanelMessage::AwayRoster(
            RosterViewPanelMessage::Scroll(ScrollDirection::Down),
        ));
        assert_eq!(panel.away_roster_panel.scroll_offset(), 1);
    }

    #[test]
    fn view_does_not_panic_daily_stats_no_data() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = MatchupMainPanel::new();
        terminal
            .draw(|frame| {
                panel.view(
                    frame,
                    frame.area(),
                    &[],
                    &[],
                    0,
                    None,
                    "Home Team",
                    "Away Team",
                    "HT",
                    "AT",
                    false,
                )
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
                panel.view(
                    frame,
                    frame.area(),
                    &[],
                    &days,
                    0,
                    None,
                    "Home Team",
                    "Away Team",
                    "HT",
                    "AT",
                    false,
                )
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
                panel.view(
                    frame,
                    frame.area(),
                    &[],
                    &[],
                    0,
                    None,
                    "Home Team",
                    "Away Team",
                    "HT",
                    "AT",
                    false,
                )
            })
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_home_roster() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MatchupMainPanel::new();
        panel.active_tab = MatchupTab::HomeRoster;
        terminal
            .draw(|frame| {
                panel.view(
                    frame,
                    frame.area(),
                    &[],
                    &[],
                    0,
                    None,
                    "Home Team",
                    "Away Team",
                    "HT",
                    "AT",
                    false,
                )
            })
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_away_roster() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MatchupMainPanel::new();
        panel.active_tab = MatchupTab::AwayRoster;
        terminal
            .draw(|frame| {
                panel.view(
                    frame,
                    frame.area(),
                    &[],
                    &[],
                    0,
                    None,
                    "Home Team",
                    "Away Team",
                    "HT",
                    "AT",
                    false,
                )
            })
            .unwrap();
    }

    fn make_test_scoring_day() -> ScoringDay {
        use crate::matchup::{DailyPlayerRow, DailyTotals, TeamDailyRoster};
        ScoringDay {
            date: "2026-03-26".to_string(),
            label: "March 26".to_string(),
            batting_stat_columns: vec!["AB".to_string(), "H".to_string(), "R".to_string()],
            pitching_stat_columns: vec![],
            home: TeamDailyRoster {
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
            },
            away: TeamDailyRoster::default(),
        }
    }
}
