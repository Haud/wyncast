// Matchup domain types for weekly head-to-head matchup tracking.

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
    pub my_team_name: String,
    pub opp_team_name: String,
    pub my_record: TeamRecord,
    pub opp_record: TeamRecord,
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

/// A single scoring category with both teams' values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryScore {
    pub stat_abbrev: String,
    pub my_value: f64,
    pub opp_value: f64,
    /// `Some(true)` = I'm winning, `Some(false)` = opponent winning, `None` = tied.
    pub i_am_winning: Option<bool>,
}

/// One day of the scoring period with player-level breakdowns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringDay {
    pub date: String,
    pub label: String,
    pub batting_rows: Vec<DailyPlayerRow>,
    pub pitching_rows: Vec<DailyPlayerRow>,
    pub batting_totals: Option<DailyTotals>,
    pub pitching_totals: Option<DailyTotals>,
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
    pub is_bench: bool,
    pub is_il: bool,
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
    pub my_team: TeamMatchupState,
    pub opp_team: TeamMatchupState,
    pub category_scores: Vec<CategoryScore>,
    pub selected_day: usize,
    pub scoring_period_days: Vec<ScoringDay>,
    pub games_started: u8,
    pub gs_limit: u8,
    pub acquisitions_used: u8,
    pub acquisitions_limit: u8,
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
    fn matchup_snapshot_construction() {
        let snapshot = MatchupSnapshot {
            matchup_info: MatchupInfo {
                matchup_period: 1,
                start_date: "2026-03-25".to_string(),
                end_date: "2026-04-05".to_string(),
                my_team_name: "Bob Dole Experience".to_string(),
                opp_team_name: "Certified! Smokified!".to_string(),
                my_record: TeamRecord { wins: 0, losses: 0, ties: 0 },
                opp_record: TeamRecord { wins: 0, losses: 0, ties: 0 },
            },
            my_team: TeamMatchupState {
                name: "Bob Dole Experience".to_string(),
                abbrev: "BDE".to_string(),
                record: TeamRecord { wins: 0, losses: 0, ties: 0 },
                category_score: TeamRecord { wins: 2, losses: 3, ties: 7 },
            },
            opp_team: TeamMatchupState {
                name: "Certified! Smokified!".to_string(),
                abbrev: "CS".to_string(),
                record: TeamRecord { wins: 0, losses: 0, ties: 0 },
                category_score: TeamRecord { wins: 3, losses: 2, ties: 7 },
            },
            category_scores: vec![
                CategoryScore {
                    stat_abbrev: "R".to_string(),
                    my_value: 5.0,
                    opp_value: 3.0,
                    i_am_winning: Some(true),
                },
                CategoryScore {
                    stat_abbrev: "ERA".to_string(),
                    my_value: 3.45,
                    opp_value: 4.12,
                    i_am_winning: Some(true),
                },
            ],
            selected_day: 1,
            scoring_period_days: vec![ScoringDay {
                date: "2026-03-26".to_string(),
                label: "March 26".to_string(),
                batting_rows: vec![DailyPlayerRow {
                    slot: "C".to_string(),
                    player_name: "Ben Rice".to_string(),
                    team: "NYY".to_string(),
                    positions: vec!["1B".to_string(), "C".to_string(), "DH".to_string()],
                    opponent: Some("@BOS".to_string()),
                    game_status: None,
                    stats: vec![Some(4.0), Some(1.0), Some(0.0)],
                    is_bench: false,
                    is_il: false,
                }],
                pitching_rows: vec![],
                batting_totals: Some(DailyTotals {
                    stats: vec![Some(29.0), Some(8.0), Some(5.0)],
                }),
                pitching_totals: None,
            }],
            games_started: 3,
            gs_limit: 7,
            acquisitions_used: 1,
            acquisitions_limit: 5,
        };

        assert_eq!(snapshot.matchup_info.matchup_period, 1);
        assert_eq!(snapshot.my_team.category_score.to_string(), "2-3-7");
        assert_eq!(snapshot.opp_team.category_score.to_string(), "3-2-7");
        assert_eq!(snapshot.category_scores.len(), 2);
        assert_eq!(snapshot.scoring_period_days.len(), 1);
        assert_eq!(snapshot.scoring_period_days[0].batting_rows.len(), 1);
        assert_eq!(snapshot.games_started, 3);
        assert_eq!(snapshot.gs_limit, 7);
    }
}
