mod category_tracker;
mod scoreboard;
pub mod tabs;

use iced::widget::row;
use iced::{Element, Length, Padding, Task};
use twui::{
    BoxStyle, Colors, StackGap, StackStyle, TextColor, TextSize, TextStyle,
    frame, text, v_stack,
};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::matchup::MatchupSnapshot;

use crate::widgets::focus_ring;

use category_tracker::{CategoryTracker, CategoryTrackerMessage};
use tabs::{
    AnalyticsMessage, AnalyticsPanel, AwayRosterMessage, AwayRosterPanel, DailyStatsMessage,
    DailyStatsPanel, HomeRosterMessage, HomeRosterPanel, MatchupTab, view_tab_bar,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Which panel has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchupFocusPanel {
    MainPanel,
    CategoryTracker,
}

impl MatchupFocusPanel {
    const CYCLE: &[MatchupFocusPanel] = &[
        MatchupFocusPanel::MainPanel,
        MatchupFocusPanel::CategoryTracker,
    ];

    pub fn next(current: Option<Self>) -> Option<Self> {
        match current {
            None => Some(Self::CYCLE[0]),
            Some(p) => {
                let idx = Self::CYCLE.iter().position(|&x| x == p)?;
                Self::CYCLE.get(idx + 1).copied()
            }
        }
    }

    pub fn prev(current: Option<Self>) -> Option<Self> {
        match current {
            None => Some(*Self::CYCLE.last().unwrap()),
            Some(p) => {
                let idx = Self::CYCLE.iter().position(|&x| x == p)?;
                if idx == 0 { None } else { Self::CYCLE.get(idx - 1).copied() }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum MatchupMessage {
    TabSelected(MatchupTab),
    PreviousDay,
    NextDay,
    FocusToggled,
    FocusToggledBack,
    ScrollRequested(ScrollDirection),
    DailyStats(DailyStatsMessage),
    Analytics(AnalyticsMessage),
    HomeRoster(HomeRosterMessage),
    AwayRoster(AwayRosterMessage),
    CategoryTracker(CategoryTrackerMessage),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct MatchupScreen {
    snapshot: Option<Box<MatchupSnapshot>>,
    active_tab: MatchupTab,
    focus: Option<MatchupFocusPanel>,
    /// Whether the sidebar is visible (hidden below 1100 px window width).
    pub show_sidebar: bool,
    // Tab panels
    daily_stats: DailyStatsPanel,
    analytics: AnalyticsPanel,
    home_roster: HomeRosterPanel,
    away_roster: AwayRosterPanel,
    category_tracker: CategoryTracker,
}

impl MatchupScreen {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            active_tab: MatchupTab::DailyStats,
            focus: None,
            show_sidebar: true,
            daily_stats: DailyStatsPanel::new(),
            analytics: AnalyticsPanel::new(),
            home_roster: HomeRosterPanel::new(),
            away_roster: AwayRosterPanel::new(),
            category_tracker: CategoryTracker::new(),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: Box<MatchupSnapshot>) {
        // Clamp selected_day to valid range.
        let max_day = snapshot.scoring_period_days.len().saturating_sub(1);
        let clamped_day = if snapshot.scoring_period_days.is_empty() {
            0
        } else {
            snapshot.selected_day.min(max_day)
        };
        let mut s = snapshot;
        s.selected_day = clamped_day;
        self.snapshot = Some(s);
    }

    pub fn update(&mut self, msg: MatchupMessage) -> Task<MatchupMessage> {
        match msg {
            MatchupMessage::TabSelected(tab) => {
                self.active_tab = tab;
                self.focus = None;
                Task::none()
            }
            MatchupMessage::PreviousDay => {
                if let Some(s) = &mut self.snapshot {
                    if s.selected_day > 0 {
                        s.selected_day -= 1;
                    }
                }
                Task::none()
            }
            MatchupMessage::NextDay => {
                if let Some(s) = &mut self.snapshot {
                    let max = s.scoring_period_days.len().saturating_sub(1);
                    if s.selected_day < max {
                        s.selected_day += 1;
                    }
                }
                Task::none()
            }
            MatchupMessage::FocusToggled => {
                self.focus = MatchupFocusPanel::next(self.focus);
                Task::none()
            }
            MatchupMessage::FocusToggledBack => {
                self.focus = MatchupFocusPanel::prev(self.focus);
                Task::none()
            }
            MatchupMessage::ScrollRequested(dir) => self.dispatch_scroll(dir),
            MatchupMessage::DailyStats(msg) => {
                self.daily_stats.update(msg).map(MatchupMessage::DailyStats)
            }
            MatchupMessage::Analytics(msg) => {
                self.analytics.update(msg).map(MatchupMessage::Analytics)
            }
            MatchupMessage::HomeRoster(msg) => {
                self.home_roster.update(msg).map(MatchupMessage::HomeRoster)
            }
            MatchupMessage::AwayRoster(msg) => {
                self.away_roster.update(msg).map(MatchupMessage::AwayRoster)
            }
            MatchupMessage::CategoryTracker(msg) => {
                self.category_tracker.update(msg).map(MatchupMessage::CategoryTracker)
            }
        }
    }

    fn dispatch_scroll(&mut self, dir: ScrollDirection) -> Task<MatchupMessage> {
        match self.focus {
            Some(MatchupFocusPanel::CategoryTracker) => {
                self.category_tracker
                    .update(CategoryTrackerMessage::ScrollBy(dir))
                    .map(MatchupMessage::CategoryTracker)
            }
            Some(MatchupFocusPanel::MainPanel) | None => match self.active_tab {
                MatchupTab::DailyStats => self
                    .daily_stats
                    .update(DailyStatsMessage::ScrollBy(dir))
                    .map(MatchupMessage::DailyStats),
                MatchupTab::Analytics => self
                    .analytics
                    .update(AnalyticsMessage::ScrollBy(dir))
                    .map(MatchupMessage::Analytics),
                MatchupTab::HomeRoster => self
                    .home_roster
                    .update(HomeRosterMessage::ScrollBy(dir))
                    .map(MatchupMessage::HomeRoster),
                MatchupTab::AwayRoster => self
                    .away_roster
                    .update(AwayRosterMessage::ScrollBy(dir))
                    .map(MatchupMessage::AwayRoster),
            },
        }
    }
}

impl Default for MatchupScreen {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view(screen: &MatchupScreen) -> Element<'_, MatchupMessage> {
    match &screen.snapshot {
        None => waiting_view(),
        Some(snapshot) => populated_view(screen, snapshot),
    }
}

fn waiting_view<'a>() -> Element<'a, MatchupMessage> {
    let msg = text(
        "Waiting for matchup data…",
        TextStyle { size: TextSize::Md, color: TextColor::Dimmed, ..Default::default() },
    );

    frame(
        msg,
        BoxStyle { width: Length::Fill, height: Length::Fill, ..Default::default() },
    )
    .into()
}

fn populated_view<'a>(
    screen: &'a MatchupScreen,
    snapshot: &'a MatchupSnapshot,
) -> Element<'a, MatchupMessage> {
    let scoreboard_elem = scoreboard::view(
        &snapshot.home_team,
        &snapshot.away_team,
        &snapshot.matchup_info,
        &snapshot.category_scores,
    );

    let day_label = snapshot
        .scoring_period_days
        .get(snapshot.selected_day)
        .map(|d| format!("Day: {} ({}/{})", d.label, snapshot.selected_day + 1, snapshot.scoring_period_days.len()))
        .unwrap_or_else(|| "No days".to_string());

    let day_info: Element<MatchupMessage> = text(
        day_label,
        TextStyle { size: TextSize::Xs, color: TextColor::Dimmed, ..Default::default() },
    )
    .into();

    let tab_bar_elem = view_tab_bar(screen.active_tab);

    let tab_content = tab_content(screen, snapshot);
    let tab_content_with_ring =
        focus_ring(tab_content, screen.focus == Some(MatchupFocusPanel::MainPanel));

    let main_panel: Element<MatchupMessage> = v_stack(
        vec![day_info, tab_bar_elem, tab_content_with_ring],
        StackStyle {
            gap: StackGap::None,
            width: Length::Fill,
            height: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let content = if screen.show_sidebar {
        let sidebar_elem = sidebar_view(screen, snapshot);
        // Matchup sidebar is purely visibility-toggled by window width — no
        // drag-to-resize needed, so we use a static 65/35 row layout.
        let left = iced::widget::container(main_panel)
            .width(Length::FillPortion(65))
            .height(Length::Fill);
        let right = iced::widget::container(sidebar_elem)
            .width(Length::FillPortion(35))
            .height(Length::Fill);
        let r: Element<MatchupMessage> = row![left, right]
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        r
    } else {
        frame(
            main_panel,
            BoxStyle { width: Length::Fill, height: Length::Fill, ..Default::default() },
        )
        .into()
    };

    let content_framed: Element<MatchupMessage> = frame(
        content,
        BoxStyle { width: Length::Fill, height: Length::Fill, ..Default::default() },
    )
    .into();

    v_stack(
        vec![scoreboard_elem, content_framed],
        StackStyle {
            gap: StackGap::None,
            width: Length::Fill,
            height: Length::Fill,
            padding: Padding::ZERO,
            background: Some(Colors::BgApp),
            ..Default::default()
        },
    )
    .into()
}

fn tab_content<'a>(
    screen: &'a MatchupScreen,
    snapshot: &'a MatchupSnapshot,
) -> Element<'a, MatchupMessage> {
    let day = snapshot.scoring_period_days.get(snapshot.selected_day);
    let home_name = &snapshot.home_team.name;
    let away_name = &snapshot.away_team.name;

    match screen.active_tab {
        MatchupTab::DailyStats => screen
            .daily_stats
            .view(day, home_name, away_name)
            .map(MatchupMessage::DailyStats),
        MatchupTab::Analytics => {
            let total_days = snapshot.scoring_period_days.len();
            let days_elapsed = if total_days > 0 {
                (snapshot.selected_day + 1).min(total_days)
            } else {
                0
            };
            screen
                .analytics
                .view(&snapshot.category_scores, days_elapsed, total_days)
                .map(MatchupMessage::Analytics)
        }
        MatchupTab::HomeRoster => screen
            .home_roster
            .view(day.map(|d| &d.home), home_name)
            .map(MatchupMessage::HomeRoster),
        MatchupTab::AwayRoster => screen
            .away_roster
            .view(day.map(|d| &d.away), away_name)
            .map(MatchupMessage::AwayRoster),
    }
}

fn sidebar_view<'a>(
    screen: &'a MatchupScreen,
    snapshot: &'a MatchupSnapshot,
) -> Element<'a, MatchupMessage> {
    let focused = screen.focus == Some(MatchupFocusPanel::CategoryTracker);
    let tracker = screen
        .category_tracker
        .view(&snapshot.category_scores, focused)
        .map(MatchupMessage::CategoryTracker);

    frame(
        tracker,
        BoxStyle {
            width: Length::Fill,
            height: Length::Fill,
            background: Some(Colors::BgSidebar),
            padding: Padding::new(4.0),
            ..Default::default()
        },
    )
    .into()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wyncast_baseball::matchup::{
        CategoryScore, CategoryState, MatchupInfo, ScoringDay, TeamDailyRoster,
        TeamMatchupState, TeamRecord,
    };

    fn make_snapshot() -> Box<MatchupSnapshot> {
        Box::new(MatchupSnapshot {
            matchup_info: MatchupInfo {
                matchup_period: 1,
                start_date: "2026-03-25".to_string(),
                end_date: "2026-04-05".to_string(),
                home_team_name: "Home Team".to_string(),
                away_team_name: "Away Team".to_string(),
                home_record: TeamRecord { wins: 1, losses: 0, ties: 0 },
                away_record: TeamRecord { wins: 0, losses: 1, ties: 0 },
            },
            home_team: TeamMatchupState {
                name: "Home Team".to_string(),
                abbrev: "HT".to_string(),
                record: TeamRecord { wins: 1, losses: 0, ties: 0 },
                category_score: TeamRecord { wins: 6, losses: 4, ties: 2 },
            },
            away_team: TeamMatchupState {
                name: "Away Team".to_string(),
                abbrev: "AT".to_string(),
                record: TeamRecord { wins: 0, losses: 1, ties: 0 },
                category_score: TeamRecord { wins: 4, losses: 6, ties: 2 },
            },
            category_scores: vec![CategoryScore {
                stat_abbrev: "R".to_string(),
                home_value: 5.0,
                away_value: 3.0,
                state: CategoryState::HomeWinning,
            }],
            selected_day: 0,
            scoring_period_days: vec![ScoringDay {
                date: "2026-03-26".to_string(),
                label: "Day 1".to_string(),
                batting_stat_columns: vec!["R".to_string()],
                pitching_stat_columns: vec![],
                home: TeamDailyRoster::default(),
                away: TeamDailyRoster::default(),
            }],
        })
    }

    #[test]
    fn new_screen_has_no_snapshot() {
        let screen = MatchupScreen::new();
        assert!(screen.snapshot.is_none());
    }

    #[test]
    fn apply_snapshot_stores_snapshot() {
        let mut screen = MatchupScreen::new();
        screen.apply_snapshot(make_snapshot());
        assert!(screen.snapshot.is_some());
    }

    #[test]
    fn apply_snapshot_clamps_selected_day() {
        let mut screen = MatchupScreen::new();
        let mut snap = *make_snapshot();
        snap.selected_day = 99;
        screen.apply_snapshot(Box::new(snap));
        assert_eq!(screen.snapshot.as_ref().unwrap().selected_day, 0);
    }

    #[test]
    fn tab_selected_clears_focus() {
        let mut screen = MatchupScreen::new();
        screen.focus = Some(MatchupFocusPanel::MainPanel);
        let _ = screen.update(MatchupMessage::TabSelected(MatchupTab::Analytics));
        assert_eq!(screen.active_tab, MatchupTab::Analytics);
        assert_eq!(screen.focus, None);
    }

    #[test]
    fn previous_day_does_not_go_below_zero() {
        let mut screen = MatchupScreen::new();
        screen.apply_snapshot(make_snapshot());
        let _ = screen.update(MatchupMessage::PreviousDay);
        assert_eq!(screen.snapshot.as_ref().unwrap().selected_day, 0);
    }

    #[test]
    fn next_day_does_not_exceed_max() {
        let mut screen = MatchupScreen::new();
        screen.apply_snapshot(make_snapshot());
        // Only 1 day, max idx = 0
        let _ = screen.update(MatchupMessage::NextDay);
        assert_eq!(screen.snapshot.as_ref().unwrap().selected_day, 0);
    }

    #[test]
    fn focus_toggle_cycles() {
        let mut screen = MatchupScreen::new();
        assert_eq!(screen.focus, None);
        let _ = screen.update(MatchupMessage::FocusToggled);
        assert_eq!(screen.focus, Some(MatchupFocusPanel::MainPanel));
        let _ = screen.update(MatchupMessage::FocusToggled);
        assert_eq!(screen.focus, Some(MatchupFocusPanel::CategoryTracker));
        let _ = screen.update(MatchupMessage::FocusToggled);
        assert_eq!(screen.focus, None);
    }

    #[test]
    fn focus_toggle_back_cycles_reverse() {
        let mut screen = MatchupScreen::new();
        let _ = screen.update(MatchupMessage::FocusToggledBack);
        assert_eq!(screen.focus, Some(MatchupFocusPanel::CategoryTracker));
        let _ = screen.update(MatchupMessage::FocusToggledBack);
        assert_eq!(screen.focus, Some(MatchupFocusPanel::MainPanel));
        let _ = screen.update(MatchupMessage::FocusToggledBack);
        assert_eq!(screen.focus, None);
    }

    #[test]
    fn focus_panel_next() {
        assert_eq!(MatchupFocusPanel::next(None), Some(MatchupFocusPanel::MainPanel));
        assert_eq!(MatchupFocusPanel::next(Some(MatchupFocusPanel::MainPanel)), Some(MatchupFocusPanel::CategoryTracker));
        assert_eq!(MatchupFocusPanel::next(Some(MatchupFocusPanel::CategoryTracker)), None);
    }

    #[test]
    fn focus_panel_prev() {
        assert_eq!(MatchupFocusPanel::prev(None), Some(MatchupFocusPanel::CategoryTracker));
        assert_eq!(MatchupFocusPanel::prev(Some(MatchupFocusPanel::CategoryTracker)), Some(MatchupFocusPanel::MainPanel));
        assert_eq!(MatchupFocusPanel::prev(Some(MatchupFocusPanel::MainPanel)), None);
    }

    #[test]
    fn scroll_does_not_panic_without_snapshot() {
        let mut screen = MatchupScreen::new();
        let _ = screen.update(MatchupMessage::ScrollRequested(ScrollDirection::Down));
        // no panic is the assertion
    }
}
