// Matchup domain types for weekly head-to-head matchup tracking.
//
// The matchup page is rendered symmetrically (home vs away). There is no
// "my team" / "opp team" distinction because ESPN's boxscore DOM doesn't
// surface which side belongs to the viewer — all UI state is addressed by
// `TeamSide::Home` or `TeamSide::Away`.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Core matchup types
// ---------------------------------------------------------------------------

/// Top-level matchup metadata: period, dates, team names, season records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchupInfo {
    pub matchup_period: u8,
    pub start_date: String,
    pub end_date: String,
    pub home_team_name: String,
    pub away_team_name: String,
    pub home_record: TeamRecord,
    pub away_record: TeamRecord,
}

/// A team's win-loss-tie record (season or matchup).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamRecord {
    pub wins: u16,
    pub losses: u16,
    pub ties: u16,
}

impl fmt::Display for TeamRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}-{}", self.wins, self.losses, self.ties)
    }
}

/// Which side of the matchup a given piece of data belongs to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TeamSide {
    Home,
    Away,
}

impl TeamSide {
    pub fn label(self) -> &'static str {
        match self {
            TeamSide::Home => "home",
            TeamSide::Away => "away",
        }
    }
}

/// Which team (if any) is ahead in a scoring category.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CategoryState {
    HomeWinning,
    AwayWinning,
    Tied,
}

/// A single scoring category with both teams' values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryScore {
    pub stat_abbrev: String,
    pub home_value: f64,
    pub away_value: f64,
    pub state: CategoryState,
}

/// One team's roster and totals for a single day.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamDailyRoster {
    pub batting_rows: Vec<DailyPlayerRow>,
    pub pitching_rows: Vec<DailyPlayerRow>,
    pub batting_totals: Option<DailyTotals>,
    pub pitching_totals: Option<DailyTotals>,
}

/// One day of the scoring period with per-team player breakdowns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringDay {
    pub date: String,
    pub label: String,
    /// Stat column headers for batting (e.g. ["AB", "H", "R", "HR", "RBI", "BB", "SB", "AVG"]).
    /// Provided by the extension; indices align with `DailyPlayerRow::stats`.
    #[serde(default)]
    pub batting_stat_columns: Vec<String>,
    /// Stat column headers for pitching (e.g. ["IP", "H", "ER", "BB", "K", "W", "SV", "HD"]).
    /// Provided by the extension; indices align with `DailyPlayerRow::stats`.
    #[serde(default)]
    pub pitching_stat_columns: Vec<String>,
    pub home: TeamDailyRoster,
    pub away: TeamDailyRoster,
}

impl ScoringDay {
    /// Return the roster for the requested side.
    pub fn roster(&self, side: TeamSide) -> &TeamDailyRoster {
        match side {
            TeamSide::Home => &self.home,
            TeamSide::Away => &self.away,
        }
    }
}

/// A single player's stats for one day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyPlayerRow {
    pub slot: String,
    pub player_name: String,
    pub team: String,
    pub positions: Vec<String>,
    /// `None` if no game scheduled.
    pub opponent: Option<String>,
    pub game_status: Option<String>,
    /// Per-stat values; `None` entries mean no game / not applicable.
    pub stats: Vec<Option<f64>>,
}

/// Aggregated totals for one section (batting or pitching) on a single day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyTotals {
    pub stats: Vec<Option<f64>>,
}

/// Per-team state within the current matchup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMatchupState {
    pub name: String,
    pub abbrev: String,
    pub record: TeamRecord,
    /// Category W-L-T within this matchup (e.g. 3-2-7).
    pub category_score: TeamRecord,
}

// ---------------------------------------------------------------------------
// Snapshot sent to the TUI
// ---------------------------------------------------------------------------

/// Complete matchup state snapshot for TUI rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchupSnapshot {
    pub matchup_info: MatchupInfo,
    pub home_team: TeamMatchupState,
    pub away_team: TeamMatchupState,
    pub category_scores: Vec<CategoryScore>,
    pub selected_day: usize,
    pub scoring_period_days: Vec<ScoringDay>,
}

impl MatchupSnapshot {
    /// Return the team state for the requested side.
    pub fn team(&self, side: TeamSide) -> &TeamMatchupState {
        match side {
            TeamSide::Home => &self.home_team,
            TeamSide::Away => &self.away_team,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_record_display() {
        let record = TeamRecord { wins: 3, losses: 2, ties: 7 };
        assert_eq!(record.to_string(), "3-2-7");
    }

    #[test]
    fn team_record_display_zeros() {
        let record = TeamRecord { wins: 0, losses: 0, ties: 0 };
        assert_eq!(record.to_string(), "0-0-0");
    }

    #[test]
    fn team_side_label() {
        assert_eq!(TeamSide::Home.label(), "home");
        assert_eq!(TeamSide::Away.label(), "away");
    }

    #[test]
    fn matchup_snapshot_construction() {
        let snapshot = MatchupSnapshot {
            matchup_info: MatchupInfo {
                matchup_period: 1,
                start_date: "2026-03-25".to_string(),
                end_date: "2026-04-05".to_string(),
                home_team_name: "Bob Dole Experience".to_string(),
                away_team_name: "Certified! Smokified!".to_string(),
                home_record: TeamRecord { wins: 0, losses: 0, ties: 0 },
                away_record: TeamRecord { wins: 0, losses: 0, ties: 0 },
            },
            home_team: TeamMatchupState {
                name: "Bob Dole Experience".to_string(),
                abbrev: "BDE".to_string(),
                record: TeamRecord { wins: 0, losses: 0, ties: 0 },
                category_score: TeamRecord { wins: 2, losses: 3, ties: 7 },
            },
            away_team: TeamMatchupState {
                name: "Certified! Smokified!".to_string(),
                abbrev: "CS".to_string(),
                record: TeamRecord { wins: 0, losses: 0, ties: 0 },
                category_score: TeamRecord { wins: 3, losses: 2, ties: 7 },
            },
            category_scores: vec![
                CategoryScore {
                    stat_abbrev: "R".to_string(),
                    home_value: 5.0,
                    away_value: 3.0,
                    state: CategoryState::HomeWinning,
                },
                CategoryScore {
                    stat_abbrev: "ERA".to_string(),
                    home_value: 3.45,
                    away_value: 4.12,
                    state: CategoryState::HomeWinning,
                },
            ],
            selected_day: 1,
            scoring_period_days: vec![ScoringDay {
                date: "2026-03-26".to_string(),
                label: "March 26".to_string(),
                batting_stat_columns: vec!["AB".to_string(), "H".to_string(), "R".to_string()],
                pitching_stat_columns: vec![],
                home: TeamDailyRoster {
                    batting_rows: vec![DailyPlayerRow {
                        slot: "C".to_string(),
                        player_name: "Ben Rice".to_string(),
                        team: "NYY".to_string(),
                        positions: vec!["1B".to_string(), "C".to_string(), "DH".to_string()],
                        opponent: Some("@BOS".to_string()),
                        game_status: None,
                        stats: vec![Some(4.0), Some(1.0), Some(0.0)],
                    }],
                    pitching_rows: vec![],
                    batting_totals: Some(DailyTotals {
                        stats: vec![Some(29.0), Some(8.0), Some(5.0)],
                    }),
                    pitching_totals: None,
                },
                away: TeamDailyRoster::default(),
            }],
        };

        assert_eq!(snapshot.matchup_info.matchup_period, 1);
        assert_eq!(snapshot.home_team.category_score.to_string(), "2-3-7");
        assert_eq!(snapshot.away_team.category_score.to_string(), "3-2-7");
        assert_eq!(snapshot.category_scores.len(), 2);
        assert_eq!(snapshot.scoring_period_days.len(), 1);
        assert_eq!(snapshot.scoring_period_days[0].home.batting_rows.len(), 1);
    }
}
