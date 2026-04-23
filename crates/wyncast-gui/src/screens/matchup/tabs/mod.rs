pub mod analytics;
pub mod away_roster;
pub mod daily_stats;
pub mod home_roster;

pub use analytics::{AnalyticsMessage, AnalyticsPanel};
pub use away_roster::{AwayRosterMessage, AwayRosterPanel};
pub use daily_stats::{DailyStatsMessage, DailyStatsPanel};
pub use home_roster::{HomeRosterMessage, HomeRosterPanel};

use iced::Element;
use twui::{Tab, TabBarStyle, tab_bar};

use super::MatchupMessage;

/// Tabs for the matchup main panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchupTab {
    DailyStats,
    Analytics,
    HomeRoster,
    AwayRoster,
}

impl MatchupTab {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            MatchupTab::DailyStats => "Daily Stats",
            MatchupTab::Analytics => "Analytics",
            MatchupTab::HomeRoster => "Home Roster",
            MatchupTab::AwayRoster => "Away Roster",
        }
    }
}

pub fn view_tab_bar(active: MatchupTab) -> Element<'static, MatchupMessage> {
    let tabs = vec![
        Tab::new("1: Daily Stats", MatchupMessage::TabSelected(MatchupTab::DailyStats)),
        Tab::new("2: Analytics", MatchupMessage::TabSelected(MatchupTab::Analytics)),
        Tab::new("3: Home Roster", MatchupMessage::TabSelected(MatchupTab::HomeRoster)),
        Tab::new("4: Away Roster", MatchupMessage::TabSelected(MatchupTab::AwayRoster)),
    ];

    let selected = match active {
        MatchupTab::DailyStats => 0,
        MatchupTab::Analytics => 1,
        MatchupTab::HomeRoster => 2,
        MatchupTab::AwayRoster => 3,
    };

    tab_bar(tabs, selected, TabBarStyle::default()).into()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_labels_non_empty() {
        for tab in [
            MatchupTab::DailyStats,
            MatchupTab::Analytics,
            MatchupTab::HomeRoster,
            MatchupTab::AwayRoster,
        ] {
            assert!(!tab.label().is_empty());
        }
    }

    #[test]
    fn tab_selected_index() {
        assert_eq!(
            match MatchupTab::DailyStats { MatchupTab::DailyStats => 0, _ => 99 },
            0
        );
        assert_eq!(
            match MatchupTab::AwayRoster { MatchupTab::AwayRoster => 3, _ => 99 },
            3
        );
    }
}
