// Matchup screen: top-level component for the ESPN matchup viewer.
//
// Analogous to DraftScreen, this owns all matchup-related state and child
// components. It implements the standard Elm-style API:
// - `apply_snapshot()` — populate state from MatchupSnapshot
// - `update()` — handle MatchupScreenMessage
// - `subscription()` — declare keybindings
// - `view()` — render all components

pub mod layout;
pub mod main_panel;
pub mod sidebar;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crossterm::event::KeyCode;
use ratatui::Frame;

use crate::matchup::{
    CategoryScore, CategoryState, MatchupInfo, MatchupSnapshot, ScoringDay, TeamMatchupState,
};
use crate::tui::action::Action;
use crate::tui::scroll::ScrollDirection;
use crate::tui::subscription::{Subscription, SubscriptionId};
use crate::tui::subscription::keybinding::{
    exact, shift, KeyBindingRecipe, KeybindHint as KbHint, KeybindManager, PRIORITY_NORMAL,
};

use layout::{build_matchup_layout, build_sidebar_layout};
use main_panel::{
    DailyStatsPanelMessage, MatchupAnalyticsPanelMessage, MatchupMainPanel,
    MatchupMainPanelMessage, MatchupTab, RosterViewPanelMessage,
};
use sidebar::{
    CategoryTrackerPanelMessage, LimitsPanelMessage, MatchupSidebar, MatchupSidebarMessage,
};

// ---------------------------------------------------------------------------
// MatchupFocusPanel
// ---------------------------------------------------------------------------

/// Identifies which panel has keyboard focus for scroll routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchupFocusPanel {
    MainPanel,
    CategoryTracker,
    Limits,
}

impl MatchupFocusPanel {
    const CYCLE: &[MatchupFocusPanel] = &[
        MatchupFocusPanel::MainPanel,
        MatchupFocusPanel::CategoryTracker,
        MatchupFocusPanel::Limits,
    ];

    /// Advance focus forward:
    /// None -> MainPanel -> CategoryTracker -> Limits -> None
    pub fn next(current: Option<MatchupFocusPanel>) -> Option<MatchupFocusPanel> {
        match current {
            None => Some(Self::CYCLE[0]),
            Some(panel) => {
                let idx = Self::CYCLE.iter().position(|&p| p == panel);
                match idx {
                    Some(i) if i + 1 < Self::CYCLE.len() => Some(Self::CYCLE[i + 1]),
                    _ => None,
                }
            }
        }
    }

    /// Advance focus backward:
    /// None -> Limits -> CategoryTracker -> MainPanel -> None
    pub fn prev(current: Option<MatchupFocusPanel>) -> Option<MatchupFocusPanel> {
        match current {
            None => Some(*Self::CYCLE.last().unwrap()),
            Some(panel) => {
                let idx = Self::CYCLE.iter().position(|&p| p == panel);
                match idx {
                    Some(0) => None,
                    Some(i) => Some(Self::CYCLE[i - 1]),
                    None => None,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MatchupScreen
// ---------------------------------------------------------------------------

/// Top-level component for the matchup mode viewer.
pub struct MatchupScreen {
    pub main_panel: MatchupMainPanel,
    pub sidebar: MatchupSidebar,
    pub focused_panel: Option<MatchupFocusPanel>,
    // Matchup state (set from MatchupSnapshot)
    pub matchup_info: Option<MatchupInfo>,
    pub my_team: Option<TeamMatchupState>,
    pub opp_team: Option<TeamMatchupState>,
    pub category_scores: Vec<CategoryScore>,
    pub selected_day: usize,
    pub scoring_period_days: Vec<ScoringDay>,
    pub games_started: u8,
    pub gs_limit: u8,
    pub acquisitions_used: u8,
    pub acquisitions_limit: u8,
    sub_id_base: SubscriptionId,
}

impl MatchupScreen {
    pub fn new() -> Self {
        Self {
            main_panel: MatchupMainPanel::new(),
            sidebar: MatchupSidebar::new(),
            focused_panel: None,
            matchup_info: None,
            my_team: None,
            opp_team: None,
            category_scores: Vec::new(),
            selected_day: 0,
            scoring_period_days: Vec::new(),
            games_started: 0,
            gs_limit: 7,
            acquisitions_used: 0,
            acquisitions_limit: 5,
            sub_id_base: SubscriptionId::unique(),
        }
    }

    /// Populate all fields from a MatchupSnapshot.
    pub fn apply_snapshot(&mut self, snapshot: &MatchupSnapshot) {
        self.matchup_info = Some(snapshot.matchup_info.clone());
        self.my_team = Some(snapshot.my_team.clone());
        self.opp_team = Some(snapshot.opp_team.clone());
        self.category_scores = snapshot.category_scores.clone();
        self.scoring_period_days = snapshot.scoring_period_days.clone();
        self.games_started = snapshot.games_started;
        self.gs_limit = snapshot.gs_limit;
        self.acquisitions_used = snapshot.acquisitions_used;
        self.acquisitions_limit = snapshot.acquisitions_limit;
        // Clamp selected_day to valid range
        if !self.scoring_period_days.is_empty()
            && self.selected_day >= self.scoring_period_days.len()
        {
            self.selected_day = self.scoring_period_days.len() - 1;
        }
    }

    /// Render the matchup screen.
    pub fn view(&self, frame: &mut Frame, keybinds: &[KbHint]) {
        let layout = build_matchup_layout(frame.area());

        // Status bar: matchup info placeholder
        let status_text = if let Some(info) = &self.matchup_info {
            let day_display = self.selected_day + 1;
            let total_days = self.scoring_period_days.len();
            format!(
                " Matchup {} ({} - {})  |  {} vs {}  |  Day {} of {}",
                info.matchup_period,
                info.start_date,
                info.end_date,
                info.my_team_name,
                info.opp_team_name,
                day_display,
                total_days,
            )
        } else {
            " Matchup (waiting for data...)".to_string()
        };
        let status_bar = ratatui::widgets::Paragraph::new(status_text)
            .style(
                ratatui::style::Style::default()
                    .fg(ratatui::style::Color::White)
                    .bg(ratatui::style::Color::DarkGray),
            );
        frame.render_widget(status_bar, layout.status_bar);

        // Scoreboard: category comparison placeholder
        let scoreboard_text = if self.category_scores.is_empty() {
            "Scoreboard (waiting for data...)".to_string()
        } else {
            let my_wins = self
                .category_scores
                .iter()
                .filter(|c| c.state == CategoryState::Winning)
                .count();
            let opp_wins = self
                .category_scores
                .iter()
                .filter(|c| c.state == CategoryState::Losing)
                .count();
            let ties = self
                .category_scores
                .iter()
                .filter(|c| c.state == CategoryState::Tied)
                .count();
            format!("Scoreboard: {my_wins}-{opp_wins}-{ties}")
        };
        let scoreboard = ratatui::widgets::Paragraph::new(scoreboard_text)
            .style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan))
            .block(
                ratatui::widgets::Block::default()
                    .borders(ratatui::widgets::Borders::ALL)
                    .title(" Scoreboard "),
            );
        frame.render_widget(scoreboard, layout.scoreboard);

        // Main panel
        let main_focused = self.focused_panel == Some(MatchupFocusPanel::MainPanel);
        self.main_panel.view(
            frame,
            layout.main_panel,
            &self.category_scores,
            &self.scoring_period_days,
            self.selected_day,
            self.games_started,
            self.gs_limit,
            self.acquisitions_used,
            self.acquisitions_limit,
            None, // StatRegistry not available at screen level yet
            main_focused,
        );

        // Sidebar (only if wide enough)
        if let Some(sidebar_rect) = layout.sidebar {
            let sb_layout = build_sidebar_layout(sidebar_rect);
            let cat_focused = self.focused_panel == Some(MatchupFocusPanel::CategoryTracker);
            let limits_focused = self.focused_panel == Some(MatchupFocusPanel::Limits);
            self.sidebar.view(
                frame,
                sb_layout.category_tracker,
                sb_layout.limits,
                cat_focused,
                limits_focused,
            );
        }

        // Help bar
        crate::tui::render_keybind_hints(frame, layout.help_bar, keybinds);
    }

    // -- Scroll dispatch --

    fn dispatch_scroll(&mut self, dir: ScrollDirection) {
        match self.focused_panel {
            Some(MatchupFocusPanel::CategoryTracker) => {
                self.sidebar.update(MatchupSidebarMessage::CategoryTracker(
                    CategoryTrackerPanelMessage::Scroll(dir),
                ));
            }
            Some(MatchupFocusPanel::Limits) => {
                self.sidebar.update(MatchupSidebarMessage::Limits(
                    LimitsPanelMessage::Scroll(dir),
                ));
            }
            Some(MatchupFocusPanel::MainPanel) | None => {
                // Route to the active tab's panel
                match self.main_panel.active_tab() {
                    MatchupTab::DailyStats => {
                        self.main_panel.update(MatchupMainPanelMessage::DailyStats(
                            DailyStatsPanelMessage::Scroll(dir),
                        ));
                    }
                    MatchupTab::Analytics => {
                        self.main_panel.update(MatchupMainPanelMessage::Analytics(
                            MatchupAnalyticsPanelMessage::Scroll(dir),
                        ));
                    }
                    MatchupTab::MyRoster => {
                        self.main_panel.update(MatchupMainPanelMessage::MyRoster(
                            RosterViewPanelMessage::Scroll(dir),
                        ));
                    }
                    MatchupTab::OppRoster => {
                        self.main_panel.update(MatchupMainPanelMessage::OppRoster(
                            RosterViewPanelMessage::Scroll(dir),
                        ));
                    }
                }
            }
        }
    }
}

impl Default for MatchupScreen {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// MatchupScreenMessage
// ---------------------------------------------------------------------------

/// Messages that can be dispatched to [`MatchupScreen`].
#[derive(Debug, Clone)]
pub enum MatchupScreenMessage {
    /// Navigate to the previous day in the scoring period.
    PreviousDay,
    /// Navigate to the next day in the scoring period.
    NextDay,
    /// Switch the active tab.
    SwitchTab(MatchupTab),
    /// Cycle focus forward.
    CycleFocus,
    /// Cycle focus backward.
    CycleFocusBack,
    /// Delegate to the main panel.
    MainPanel(MatchupMainPanelMessage),
    /// Delegate to the sidebar.
    Sidebar(MatchupSidebarMessage),
    /// Scroll the currently focused panel.
    ScrollFocused(ScrollDirection),
    /// Quit the application.
    Quit,
}

impl MatchupScreen {
    /// Process a [`MatchupScreenMessage`] and return an optional [`Action`].
    pub fn update(&mut self, msg: MatchupScreenMessage) -> Option<Action> {
        match msg {
            MatchupScreenMessage::PreviousDay => {
                if self.selected_day > 0 {
                    self.selected_day -= 1;
                }
                None
            }
            MatchupScreenMessage::NextDay => {
                let max = if self.scoring_period_days.is_empty() {
                    0
                } else {
                    self.scoring_period_days.len() - 1
                };
                if self.selected_day < max {
                    self.selected_day += 1;
                }
                None
            }
            MatchupScreenMessage::SwitchTab(tab) => {
                self.main_panel.active_tab = tab;
                self.focused_panel = None;
                None
            }
            MatchupScreenMessage::CycleFocus => {
                self.focused_panel = MatchupFocusPanel::next(self.focused_panel);
                None
            }
            MatchupScreenMessage::CycleFocusBack => {
                self.focused_panel = MatchupFocusPanel::prev(self.focused_panel);
                None
            }
            MatchupScreenMessage::MainPanel(m) => self.main_panel.update(m),
            MatchupScreenMessage::Sidebar(m) => self.sidebar.update(m),
            MatchupScreenMessage::ScrollFocused(dir) => {
                self.dispatch_scroll(dir);
                None
            }
            MatchupScreenMessage::Quit => {
                Some(Action::Quit)
            }
        }
    }

    /// Declare keybindings for the subscription system.
    pub fn subscription(&self, kb: &mut KeybindManager) -> Subscription<MatchupScreenMessage> {
        // Child subscriptions
        let main_sub = self
            .main_panel
            .subscription(kb)
            .map(MatchupScreenMessage::MainPanel);
        let sidebar_sub = self
            .sidebar
            .subscription(kb)
            .map(MatchupScreenMessage::Sidebar);

        // State-dependent ID for screen-level bindings
        let own_sub = {
            let mut hasher = DefaultHasher::new();
            self.sub_id_base.hash(&mut hasher);
            let fp_disc: u8 = match self.focused_panel {
                None => 0,
                Some(MatchupFocusPanel::MainPanel) => 1,
                Some(MatchupFocusPanel::CategoryTracker) => 2,
                Some(MatchupFocusPanel::Limits) => 3,
            };
            fp_disc.hash(&mut hasher);
            let tab_disc: u8 = match self.main_panel.active_tab() {
                MatchupTab::DailyStats => 0,
                MatchupTab::Analytics => 1,
                MatchupTab::MyRoster => 2,
                MatchupTab::OppRoster => 3,
            };
            tab_disc.hash(&mut hasher);
            let own_id = SubscriptionId::from_u64(hasher.finish());

            let has_focus = self.focused_panel.is_some();

            let mut recipe = KeyBindingRecipe::<MatchupScreenMessage>::new(own_id)
                .priority(PRIORITY_NORMAL)
                // Always-present bindings
                .bind(
                    exact(KeyCode::Char('q')),
                    |_| MatchupScreenMessage::Quit,
                    KbHint::new("q", "Quit"),
                )
                .bind(
                    exact(KeyCode::Left),
                    |_| MatchupScreenMessage::PreviousDay,
                    KbHint::new("\u{2190}\u{2192}/h/l", "Day"),
                )
                .bind(
                    exact(KeyCode::Char('h')),
                    |_| MatchupScreenMessage::PreviousDay,
                    None,
                )
                .bind(
                    exact(KeyCode::Right),
                    |_| MatchupScreenMessage::NextDay,
                    None,
                )
                .bind(
                    exact(KeyCode::Char('l')),
                    |_| MatchupScreenMessage::NextDay,
                    None,
                )
                .bind(
                    exact(KeyCode::Char('1')),
                    |_| MatchupScreenMessage::SwitchTab(MatchupTab::DailyStats),
                    KbHint::new("1-4", "Tabs"),
                )
                .bind(
                    exact(KeyCode::Char('2')),
                    |_| MatchupScreenMessage::SwitchTab(MatchupTab::Analytics),
                    None,
                )
                .bind(
                    exact(KeyCode::Char('3')),
                    |_| MatchupScreenMessage::SwitchTab(MatchupTab::MyRoster),
                    None,
                )
                .bind(
                    exact(KeyCode::Char('4')),
                    |_| MatchupScreenMessage::SwitchTab(MatchupTab::OppRoster),
                    None,
                )
                .bind(
                    exact(KeyCode::Tab),
                    |_| MatchupScreenMessage::CycleFocus,
                    KbHint::new("Tab", "Focus"),
                )
                .bind(
                    shift(KeyCode::BackTab),
                    |_| MatchupScreenMessage::CycleFocusBack,
                    None,
                );

            // Scroll bindings
            if has_focus {
                recipe = recipe
                    .bind(
                        exact(KeyCode::Up),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::Up),
                        KbHint::new("\u{2191}\u{2193}/j/k", "Scroll"),
                    )
                    .bind(
                        exact(KeyCode::Char('k')),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::Up),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Down),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::Down),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Char('j')),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::Down),
                        None,
                    )
                    .bind(
                        exact(KeyCode::PageUp),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::PageUp),
                        None,
                    )
                    .bind(
                        exact(KeyCode::PageDown),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::PageDown),
                        None,
                    );
            } else {
                recipe = recipe
                    .bind(
                        exact(KeyCode::Up),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::Up),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Char('k')),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::Up),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Down),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::Down),
                        None,
                    )
                    .bind(
                        exact(KeyCode::Char('j')),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::Down),
                        None,
                    )
                    .bind(
                        exact(KeyCode::PageUp),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::PageUp),
                        None,
                    )
                    .bind(
                        exact(KeyCode::PageDown),
                        |_| MatchupScreenMessage::ScrollFocused(ScrollDirection::PageDown),
                        None,
                    );
            }

            kb.subscribe(recipe)
        };

        Subscription::batch([main_sub, sidebar_sub, own_sub])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Focus cycling --

    #[test]
    fn focus_cycle_forward() {
        // None -> MainPanel -> CategoryTracker -> Limits -> None
        let mut focus: Option<MatchupFocusPanel> = None;
        focus = MatchupFocusPanel::next(focus);
        assert_eq!(focus, Some(MatchupFocusPanel::MainPanel));
        focus = MatchupFocusPanel::next(focus);
        assert_eq!(focus, Some(MatchupFocusPanel::CategoryTracker));
        focus = MatchupFocusPanel::next(focus);
        assert_eq!(focus, Some(MatchupFocusPanel::Limits));
        focus = MatchupFocusPanel::next(focus);
        assert_eq!(focus, None);
    }

    #[test]
    fn focus_cycle_backward() {
        // None -> Limits -> CategoryTracker -> MainPanel -> None
        let mut focus: Option<MatchupFocusPanel> = None;
        focus = MatchupFocusPanel::prev(focus);
        assert_eq!(focus, Some(MatchupFocusPanel::Limits));
        focus = MatchupFocusPanel::prev(focus);
        assert_eq!(focus, Some(MatchupFocusPanel::CategoryTracker));
        focus = MatchupFocusPanel::prev(focus);
        assert_eq!(focus, Some(MatchupFocusPanel::MainPanel));
        focus = MatchupFocusPanel::prev(focus);
        assert_eq!(focus, None);
    }

    // -- Day navigation --

    #[test]
    fn previous_day_does_not_go_below_zero() {
        let mut screen = MatchupScreen::new();
        screen.selected_day = 0;
        screen.update(MatchupScreenMessage::PreviousDay);
        assert_eq!(screen.selected_day, 0);
    }

    #[test]
    fn next_day_does_not_exceed_max() {
        let mut screen = MatchupScreen::new();
        // No days — should stay at 0
        screen.update(MatchupScreenMessage::NextDay);
        assert_eq!(screen.selected_day, 0);
    }

    #[test]
    fn day_navigation_with_days() {
        let mut screen = MatchupScreen::new();
        screen.scoring_period_days = vec![
            make_scoring_day("Day 1"),
            make_scoring_day("Day 2"),
            make_scoring_day("Day 3"),
        ];
        assert_eq!(screen.selected_day, 0);

        screen.update(MatchupScreenMessage::NextDay);
        assert_eq!(screen.selected_day, 1);

        screen.update(MatchupScreenMessage::NextDay);
        assert_eq!(screen.selected_day, 2);

        // At max, stays at 2
        screen.update(MatchupScreenMessage::NextDay);
        assert_eq!(screen.selected_day, 2);

        screen.update(MatchupScreenMessage::PreviousDay);
        assert_eq!(screen.selected_day, 1);
    }

    // -- Tab switching --

    #[test]
    fn switch_tab_updates_and_clears_focus() {
        let mut screen = MatchupScreen::new();
        screen.focused_panel = Some(MatchupFocusPanel::MainPanel);

        screen.update(MatchupScreenMessage::SwitchTab(MatchupTab::Analytics));
        assert_eq!(screen.main_panel.active_tab(), MatchupTab::Analytics);
        assert_eq!(screen.focused_panel, None);
    }

    // -- apply_snapshot --

    #[test]
    fn apply_snapshot_populates_state() {
        let mut screen = MatchupScreen::new();
        let snapshot = make_test_snapshot();

        screen.apply_snapshot(&snapshot);

        assert!(screen.matchup_info.is_some());
        assert!(screen.my_team.is_some());
        assert!(screen.opp_team.is_some());
        assert_eq!(screen.category_scores.len(), 1);
        assert_eq!(screen.scoring_period_days.len(), 2);
        assert_eq!(screen.games_started, 3);
        assert_eq!(screen.gs_limit, 7);
        assert_eq!(screen.acquisitions_used, 1);
        assert_eq!(screen.acquisitions_limit, 5);
    }

    #[test]
    fn apply_snapshot_clamps_selected_day() {
        let mut screen = MatchupScreen::new();
        screen.selected_day = 10;

        let snapshot = make_test_snapshot(); // 2 days
        screen.apply_snapshot(&snapshot);

        assert_eq!(screen.selected_day, 1); // clamped to max index
    }

    // -- View smoke test --

    #[test]
    fn view_does_not_panic_empty_state() {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let screen = MatchupScreen::new();
        terminal
            .draw(|frame| screen.view(frame, &[]))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_narrow_terminal() {
        let backend = ratatui::backend::TestBackend::new(80, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let screen = MatchupScreen::new();
        terminal
            .draw(|frame| screen.view(frame, &[]))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_with_data() {
        let backend = ratatui::backend::TestBackend::new(160, 50);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut screen = MatchupScreen::new();
        screen.apply_snapshot(&make_test_snapshot());
        terminal
            .draw(|frame| screen.view(frame, &[]))
            .unwrap();
    }

    // -- Helpers --

    fn make_scoring_day(label: &str) -> ScoringDay {
        ScoringDay {
            date: "2026-03-26".to_string(),
            label: label.to_string(),
            batting_rows: Vec::new(),
            pitching_rows: Vec::new(),
            batting_totals: None,
            pitching_totals: None,
        }
    }

    fn make_test_snapshot() -> MatchupSnapshot {
        use crate::matchup::TeamRecord;

        MatchupSnapshot {
            matchup_info: MatchupInfo {
                matchup_period: 1,
                start_date: "2026-03-25".to_string(),
                end_date: "2026-04-05".to_string(),
                my_team_name: "My Team".to_string(),
                opp_team_name: "Opp Team".to_string(),
                my_record: TeamRecord { wins: 1, losses: 0, ties: 0 },
                opp_record: TeamRecord { wins: 0, losses: 1, ties: 0 },
            },
            my_team: TeamMatchupState {
                name: "My Team".to_string(),
                abbrev: "MT".to_string(),
                record: TeamRecord { wins: 1, losses: 0, ties: 0 },
                category_score: TeamRecord { wins: 6, losses: 4, ties: 2 },
            },
            opp_team: TeamMatchupState {
                name: "Opp Team".to_string(),
                abbrev: "OT".to_string(),
                record: TeamRecord { wins: 0, losses: 1, ties: 0 },
                category_score: TeamRecord { wins: 4, losses: 6, ties: 2 },
            },
            category_scores: vec![CategoryScore {
                stat_abbrev: "R".to_string(),
                my_value: 5.0,
                opp_value: 3.0,
                state: CategoryState::Winning,
            }],
            selected_day: 0,
            scoring_period_days: vec![
                make_scoring_day("Day 1"),
                make_scoring_day("Day 2"),
            ],
            games_started: 3,
            gs_limit: 7,
            acquisitions_used: 1,
            acquisitions_limit: 5,
        }
    }
}
