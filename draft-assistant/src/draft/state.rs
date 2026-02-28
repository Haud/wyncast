// Draft state: current nomination, budgets, available players.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::warn;

use super::pick::DraftPick;
use super::roster::Roster;

/// The state of a single team during the draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamState {
    /// Team identifier (e.g., "team_1").
    pub team_id: String,
    /// Display name of the team.
    pub team_name: String,
    /// The team's roster.
    pub roster: Roster,
    /// Total salary spent so far.
    pub budget_spent: u32,
    /// Remaining salary cap.
    pub budget_remaining: u32,
}

/// The currently active nomination in an auction draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveNomination {
    /// Name of the nominated player.
    pub player_name: String,
    /// ESPN player ID.
    pub player_id: String,
    /// Position string (e.g., "SP", "OF").
    pub position: String,
    /// Team name/ID that nominated the player.
    pub nominated_by: String,
    /// Current high bid.
    pub current_bid: u32,
    /// Team currently holding the high bid, if any.
    pub current_bidder: Option<String>,
    /// Seconds remaining on the nomination timer, if known.
    pub time_remaining: Option<u32>,
    /// ESPN eligible slot IDs for multi-position awareness.
    #[serde(default)]
    pub eligible_slots: Vec<u16>,
}

/// The complete state of the draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftState {
    /// All teams participating in the draft, sorted by team_id.
    pub teams: Vec<TeamState>,
    /// All recorded draft picks in order.
    pub picks: Vec<DraftPick>,
    /// The currently active nomination, if any.
    pub current_nomination: Option<ActiveNomination>,
    /// Number of picks completed so far.
    pub pick_count: usize,
    /// Total number of picks in the draft (sum of all draftable slots).
    pub total_picks: usize,
    /// Index into `teams` for the user's team.
    pub my_team_idx: usize,
    /// Order of team indices for nominations (round-robin, etc.).
    pub nomination_order: Vec<usize>,
    /// The salary cap per team (stored for restore).
    salary_cap: u32,
    /// The roster configuration (stored for restore).
    roster_config: HashMap<String, usize>,
}

impl DraftState {
    /// Create a new draft state.
    ///
    /// # Arguments
    /// - `teams`: Vec of (team_id, team_name) pairs
    /// - `my_team_id`: The user's team ID
    /// - `salary_cap`: Per-team salary cap
    /// - `roster_config`: Position -> slot count mapping from league config
    pub fn new(
        teams: Vec<(String, String)>,
        my_team_id: &str,
        salary_cap: u32,
        roster_config: &HashMap<String, usize>,
    ) -> Self {
        let mut team_states: Vec<TeamState> = teams
            .into_iter()
            .map(|(id, name)| TeamState {
                team_id: id,
                team_name: name,
                roster: Roster::new(roster_config),
                budget_spent: 0,
                budget_remaining: salary_cap,
            })
            .collect();

        // Sort teams by team_id for deterministic ordering
        team_states.sort_by(|a, b| a.team_id.cmp(&b.team_id));

        // Match by team_id first; fall back to team_name when the extension
        // sends the display name instead of a numeric ID (DOM scraping mode).
        let my_team_idx = team_states
            .iter()
            .position(|t| t.team_id == my_team_id)
            .or_else(|| team_states.iter().position(|t| t.team_name == my_team_id))
            .unwrap_or(0);

        // Total picks = sum of draftable (non-IL) slots per team
        let draftable_per_team = team_states
            .first()
            .map(|t| t.roster.draftable_count())
            .unwrap_or(0);
        let total_picks = draftable_per_team * team_states.len();

        // Default nomination order: sequential by team index
        let nomination_order: Vec<usize> = (0..team_states.len()).collect();

        DraftState {
            teams: team_states,
            picks: Vec::new(),
            current_nomination: None,
            pick_count: 0,
            total_picks,
            my_team_idx,
            nomination_order,
            salary_cap,
            roster_config: roster_config.clone(),
        }
    }

    /// Record a completed draft pick.
    ///
    /// Updates the winning team's budget and roster, increments the pick count,
    /// and appends the pick to the history. If the team is not yet known (e.g.
    /// ESPN sends real team names that don't match config placeholders), the
    /// team is auto-registered with default budget values.
    pub fn record_pick(&mut self, pick: DraftPick) {
        // Look up team by team_id first; fall back to team_name when the ID
        // is empty or doesn't match (DOM scraping uses team names as IDs).
        let team_idx = self
            .teams
            .iter()
            .position(|t| !pick.team_id.is_empty() && t.team_id == pick.team_id)
            .or_else(|| {
                self.teams
                    .iter()
                    .position(|t| !pick.team_name.is_empty() && t.team_name == pick.team_name)
            });

        // If no existing team matched, auto-register from the pick data.
        let team_idx = team_idx.unwrap_or_else(|| {
            let id = if pick.team_id.is_empty() {
                pick.team_name.clone()
            } else {
                pick.team_id.clone()
            };
            let name = if pick.team_name.is_empty() {
                pick.team_id.clone()
            } else {
                pick.team_name.clone()
            };
            warn!(
                "Auto-registered unknown team: '{}' (id='{}')",
                name, id
            );
            let new_team = TeamState {
                team_id: id,
                team_name: name,
                roster: Roster::new(&self.roster_config),
                budget_spent: 0,
                budget_remaining: self.salary_cap,
            };
            self.teams.push(new_team);
            self.teams.len() - 1
        });

        let team = &mut self.teams[team_idx];
        team.budget_spent += pick.price;
        team.budget_remaining = team.budget_remaining.saturating_sub(pick.price);
        team.roster.add_player_with_slots(
            &pick.player_name,
            &pick.position,
            pick.price,
            &pick.eligible_slots,
        );

        self.pick_count += 1;
        self.picks.push(pick);
    }

    /// Reconcile team budgets with data scraped from the ESPN DOM.
    ///
    /// Uses the ESPN-reported remaining budget as the source of truth
    /// and adjusts `budget_remaining` and `budget_spent` accordingly.
    /// If a team name from ESPN doesn't match any known team, the team
    /// is auto-registered.
    pub fn reconcile_budgets(&mut self, espn_budgets: &[TeamBudgetPayload]) {
        for budget_data in espn_budgets {
            // Match by team_name since DOM scraping doesn't provide numeric IDs.
            // Try case-insensitive match as a fallback.
            let idx = self
                .teams
                .iter()
                .position(|t| t.team_name == budget_data.team_name)
                .or_else(|| {
                    self.teams.iter().position(|t| {
                        t.team_name.eq_ignore_ascii_case(&budget_data.team_name)
                    })
                });

            let idx = match idx {
                Some(i) => i,
                None => {
                    warn!(
                        "Auto-registered unknown team from budget reconciliation: '{}'",
                        budget_data.team_name
                    );
                    let new_team = TeamState {
                        team_id: budget_data.team_name.clone(),
                        team_name: budget_data.team_name.clone(),
                        roster: Roster::new(&self.roster_config),
                        budget_spent: 0,
                        budget_remaining: self.salary_cap,
                    };
                    self.teams.push(new_team);
                    self.teams.len() - 1
                }
            };

            let team = &mut self.teams[idx];
            team.budget_remaining = budget_data.budget;
            team.budget_spent = self.salary_cap.saturating_sub(budget_data.budget);
        }
    }

    /// Total salary spent across all teams.
    pub fn total_spent(&self) -> u32 {
        self.teams.iter().map(|t| t.budget_spent).sum()
    }

    /// Look up a team by ID.
    pub fn team(&self, team_id: &str) -> Option<&TeamState> {
        self.teams.iter().find(|t| t.team_id == team_id)
    }

    /// Get a mutable reference to a team by ID.
    pub fn team_mut(&mut self, team_id: &str) -> Option<&mut TeamState> {
        self.teams.iter_mut().find(|t| t.team_id == team_id)
    }

    /// Reference to the user's team state.
    pub fn my_team(&self) -> &TeamState {
        &self.teams[self.my_team_idx]
    }

    /// Restore the draft state by replaying a sequence of picks.
    ///
    /// This is used for crash recovery: given a saved list of picks,
    /// replay them all to reconstruct the full state.
    pub fn restore_from_picks(&mut self, picks: Vec<DraftPick>) {
        // Reset state: rebuild rosters from stored config, reset budgets
        for team in &mut self.teams {
            team.budget_spent = 0;
            team.budget_remaining = self.salary_cap;
            team.roster = Roster::new(&self.roster_config);
        }
        self.picks.clear();
        self.pick_count = 0;
        self.current_nomination = None;

        for pick in picks {
            self.record_pick(pick);
        }
    }
}

// --- Differential State Detection ---

/// Budget data for a team as scraped from the ESPN DOM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamBudgetPayload {
    pub team_name: String,
    pub budget: u32,
}

/// Payload representing the current draft state as received from the extension.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateUpdatePayload {
    /// All picks completed so far.
    pub picks: Vec<PickPayload>,
    /// The currently active nomination, if any.
    pub current_nomination: Option<NominationPayload>,
    /// Team budget data from the ESPN pick train carousel.
    #[serde(default)]
    pub teams: Vec<TeamBudgetPayload>,
    /// Current pick number from the ESPN clock label (e.g. "PK 128 OF 260").
    #[serde(default)]
    pub pick_count: Option<u32>,
    /// Total number of picks from the ESPN clock label.
    #[serde(default)]
    pub total_picks: Option<u32>,
}

/// A single pick as received from the extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickPayload {
    pub pick_number: u32,
    pub team_id: String,
    pub team_name: String,
    pub player_id: String,
    pub player_name: String,
    pub position: String,
    pub price: u32,
    #[serde(default)]
    pub eligible_slots: Vec<u16>,
}

/// A nomination as received from the extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NominationPayload {
    pub player_id: String,
    pub player_name: String,
    pub position: String,
    pub nominated_by: String,
    pub current_bid: u32,
    pub current_bidder: Option<String>,
    pub time_remaining: Option<u32>,
    #[serde(default)]
    pub eligible_slots: Vec<u16>,
}

/// The result of comparing two consecutive state snapshots.
#[derive(Debug, Clone)]
pub struct StateDiff {
    /// New picks that appeared since the last snapshot.
    pub new_picks: Vec<DraftPick>,
    /// Whether the nomination changed (new player, or cleared).
    pub nomination_changed: bool,
    /// The new nomination, if one appeared or changed player.
    pub new_nomination: Option<ActiveNomination>,
    /// Whether the nomination was cleared (went from Some to None).
    pub nomination_cleared: bool,
    /// Whether only the bid amount/bidder changed on the same nomination.
    pub bid_updated: bool,
}

/// Compute the differences between two consecutive state snapshots.
///
/// If `previous` is `None`, all picks and the current nomination are treated as new.
pub fn compute_state_diff(
    previous: &Option<StateUpdatePayload>,
    current: &StateUpdatePayload,
) -> StateDiff {
    let mut diff = StateDiff {
        new_picks: Vec::new(),
        nomination_changed: false,
        new_nomination: None,
        nomination_cleared: false,
        bid_updated: false,
    };

    // Determine new picks by pick_number rather than array position.
    // This handles cases where the extension may re-order picks.
    let prev_pick_numbers: std::collections::HashSet<u32> = previous
        .as_ref()
        .map(|p| p.picks.iter().map(|pk| pk.pick_number).collect())
        .unwrap_or_default();

    for pick_payload in &current.picks {
        if !prev_pick_numbers.contains(&pick_payload.pick_number) {
            diff.new_picks.push(DraftPick {
                pick_number: pick_payload.pick_number,
                team_id: pick_payload.team_id.clone(),
                team_name: pick_payload.team_name.clone(),
                player_name: pick_payload.player_name.clone(),
                position: pick_payload.position.clone(),
                price: pick_payload.price,
                espn_player_id: Some(pick_payload.player_id.clone()),
                eligible_slots: pick_payload.eligible_slots.clone(),
            });
        }
    }

    // Compare nominations
    let prev_nom = previous.as_ref().and_then(|p| p.current_nomination.as_ref());
    let curr_nom = current.current_nomination.as_ref();

    match (prev_nom, curr_nom) {
        (None, None) => {
            // No change
        }
        (None, Some(nom)) => {
            // New nomination appeared
            diff.nomination_changed = true;
            diff.new_nomination = Some(nomination_from_payload(nom));
        }
        (Some(_), None) => {
            // Nomination was cleared (pick completed)
            diff.nomination_changed = true;
            diff.nomination_cleared = true;
        }
        (Some(prev), Some(curr)) => {
            // Detect nomination change: compare by player_id when available,
            // fall back to player_name (+ position) when IDs are empty
            // (DOM scraping doesn't provide player IDs).
            let is_different_player = if !prev.player_id.is_empty() && !curr.player_id.is_empty() {
                prev.player_id != curr.player_id
            } else {
                prev.player_name != curr.player_name || prev.position != curr.position
            };

            if is_different_player {
                // Different player nominated
                diff.nomination_changed = true;
                diff.new_nomination = Some(nomination_from_payload(curr));
            } else if prev.current_bid != curr.current_bid
                || prev.current_bidder != curr.current_bidder
            {
                // Same player, bid changed
                diff.bid_updated = true;
                diff.new_nomination = Some(nomination_from_payload(curr));
            }
        }
    }

    diff
}

/// Convert a NominationPayload into an ActiveNomination.
fn nomination_from_payload(payload: &NominationPayload) -> ActiveNomination {
    ActiveNomination {
        player_name: payload.player_name.clone(),
        player_id: payload.player_id.clone(),
        position: payload.position.clone(),
        nominated_by: payload.nominated_by.clone(),
        current_bid: payload.current_bid,
        current_bidder: payload.current_bidder.clone(),
        time_remaining: payload.time_remaining,
        eligible_slots: payload.eligible_slots.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_roster_config() -> HashMap<String, usize> {
        let mut config = HashMap::new();
        config.insert("C".to_string(), 1);
        config.insert("1B".to_string(), 1);
        config.insert("2B".to_string(), 1);
        config.insert("3B".to_string(), 1);
        config.insert("SS".to_string(), 1);
        config.insert("LF".to_string(), 1);
        config.insert("CF".to_string(), 1);
        config.insert("RF".to_string(), 1);
        config.insert("UTIL".to_string(), 1);
        config.insert("SP".to_string(), 5);
        config.insert("RP".to_string(), 6);
        config.insert("BE".to_string(), 6);
        config.insert("IL".to_string(), 5);
        config
    }

    fn test_teams() -> Vec<(String, String)> {
        (1..=10)
            .map(|i| (format!("team_{}", i), format!("Team {}", i)))
            .collect()
    }

    #[test]
    fn draft_state_creation() {
        let state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        assert_eq!(state.teams.len(), 10);
        assert_eq!(state.pick_count, 0);
        assert_eq!(state.my_team_idx, 0); // team_1 sorts first
        assert!(state.current_nomination.is_none());
    }

    #[test]
    fn draft_state_teams_sorted() {
        let mut teams = test_teams();
        teams.reverse(); // Reverse order to test sorting
        let state = DraftState::new(teams, "team_5", 260, &test_roster_config());
        assert_eq!(state.teams[0].team_id, "team_1");
        assert_eq!(state.teams[1].team_id, "team_10"); // "team_10" < "team_2" lexicographically
        assert_eq!(state.teams[9].team_id, "team_9");
    }

    #[test]
    fn draft_state_total_picks() {
        let state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        // 26 draftable slots per team * 10 teams = 260
        assert_eq!(state.total_picks, 260);
    }

    #[test]
    fn draft_state_my_team() {
        let state = DraftState::new(test_teams(), "team_3", 260, &test_roster_config());
        let my = state.my_team();
        assert_eq!(my.team_id, "team_3");
        assert_eq!(my.budget_remaining, 260);
    }

    #[test]
    fn record_pick_updates_budget() {
        let mut state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        let pick = DraftPick {
            pick_number: 1,
            team_id: "team_1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        };
        state.record_pick(pick);

        let team = state.team("team_1").unwrap();
        assert_eq!(team.budget_spent, 45);
        assert_eq!(team.budget_remaining, 215);
        assert_eq!(state.pick_count, 1);
        assert_eq!(state.picks.len(), 1);
    }

    #[test]
    fn record_pick_updates_roster() {
        let mut state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        let pick = DraftPick {
            pick_number: 1,
            team_id: "team_1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        };
        state.record_pick(pick);

        let team = state.team("team_1").unwrap();
        assert_eq!(team.roster.filled_count(), 1);
    }

    #[test]
    fn record_multiple_picks() {
        let mut state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());

        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "team_1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "team_2".to_string(),
            team_name: "Team 2".to_string(),
            player_name: "Shohei Ohtani".to_string(),
            position: "SP".to_string(),
            price: 50,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 3,
            team_id: "team_1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mookie Betts".to_string(),
            position: "RF".to_string(),
            price: 35,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        assert_eq!(state.pick_count, 3);
        assert_eq!(state.picks.len(), 3);

        let team1 = state.team("team_1").unwrap();
        assert_eq!(team1.budget_spent, 80);
        assert_eq!(team1.budget_remaining, 180);
        assert_eq!(team1.roster.filled_count(), 2);

        let team2 = state.team("team_2").unwrap();
        assert_eq!(team2.budget_spent, 50);
        assert_eq!(team2.budget_remaining, 210);
    }

    #[test]
    fn total_spent() {
        let mut state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "team_1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Player A".to_string(),
            position: "SP".to_string(),
            price: 30,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "team_3".to_string(),
            team_name: "Team 3".to_string(),
            player_name: "Player B".to_string(),
            position: "1B".to_string(),
            price: 25,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        assert_eq!(state.total_spent(), 55);
    }

    #[test]
    fn team_lookup() {
        let state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        assert!(state.team("team_5").is_some());
        assert_eq!(state.team("team_5").unwrap().team_name, "Team 5");
        assert!(state.team("nonexistent").is_none());
    }

    #[test]
    fn restore_from_picks() {
        let roster_config = test_roster_config();

        let picks = vec![
            DraftPick {
                pick_number: 1,
                team_id: "team_1".to_string(),
                team_name: "Team 1".to_string(),
                player_name: "Mike Trout".to_string(),
                position: "CF".to_string(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
            },
            DraftPick {
                pick_number: 2,
                team_id: "team_2".to_string(),
                team_name: "Team 2".to_string(),
                player_name: "Shohei Ohtani".to_string(),
                position: "SP".to_string(),
                price: 50,
                espn_player_id: None,
                eligible_slots: vec![],
            },
            DraftPick {
                pick_number: 3,
                team_id: "team_1".to_string(),
                team_name: "Team 1".to_string(),
                player_name: "Mookie Betts".to_string(),
                position: "RF".to_string(),
                price: 35,
                espn_player_id: None,
                eligible_slots: vec![],
            },
        ];

        // Create a fresh state and restore from picks
        let mut state = DraftState::new(test_teams(), "team_1", 260, &roster_config);
        state.restore_from_picks(picks);

        assert_eq!(state.pick_count, 3);
        assert_eq!(state.picks.len(), 3);

        let team1 = state.team("team_1").unwrap();
        assert_eq!(team1.budget_spent, 80);
        assert_eq!(team1.budget_remaining, 180);
        assert_eq!(team1.roster.filled_count(), 2);
    }

    #[test]
    fn restore_from_picks_resets_previous_state() {
        let roster_config = test_roster_config();
        let mut state = DraftState::new(test_teams(), "team_1", 260, &roster_config);

        // Record some picks first
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "team_1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Old Player".to_string(),
            position: "C".to_string(),
            price: 20,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        assert_eq!(state.pick_count, 1);

        // Now restore from a different set of picks
        let new_picks = vec![DraftPick {
            pick_number: 1,
            team_id: "team_2".to_string(),
            team_name: "Team 2".to_string(),
            player_name: "New Player".to_string(),
            position: "SP".to_string(),
            price: 30,
            espn_player_id: None,
            eligible_slots: vec![],
        }];
        state.restore_from_picks(new_picks);

        // Old state should be completely replaced
        assert_eq!(state.pick_count, 1);
        assert_eq!(state.picks[0].player_name, "New Player");
        let team1 = state.team("team_1").unwrap();
        assert_eq!(team1.budget_spent, 0); // Old pick should be gone
        assert_eq!(team1.budget_remaining, 260);
        let team2 = state.team("team_2").unwrap();
        assert_eq!(team2.budget_spent, 30);
    }

    // --- State Diff Tests ---

    fn make_pick_payload(
        num: u32,
        team_id: &str,
        player: &str,
        pos: &str,
        price: u32,
    ) -> PickPayload {
        PickPayload {
            pick_number: num,
            team_id: team_id.to_string(),
            team_name: format!("Team {}", team_id),
            player_id: format!("player_{}", num),
            player_name: player.to_string(),
            position: pos.to_string(),
            price,
            eligible_slots: vec![],
        }
    }

    fn make_nomination(
        player_id: &str,
        player_name: &str,
        bid: u32,
        bidder: Option<&str>,
    ) -> NominationPayload {
        NominationPayload {
            player_id: player_id.to_string(),
            player_name: player_name.to_string(),
            position: "SP".to_string(),
            nominated_by: "team_1".to_string(),
            current_bid: bid,
            current_bidder: bidder.map(|s| s.to_string()),
            time_remaining: Some(30),
            eligible_slots: vec![],
        }
    }

    #[test]
    fn diff_first_snapshot_all_new() {
        let current = StateUpdatePayload {
            picks: vec![
                make_pick_payload(1, "team_1", "Player A", "SP", 20),
                make_pick_payload(2, "team_2", "Player B", "CF", 30),
            ],
            current_nomination: Some(make_nomination("p3", "Player C", 5, None)),
            ..Default::default()
        };

        let diff = compute_state_diff(&None, &current);
        assert_eq!(diff.new_picks.len(), 2);
        assert!(diff.nomination_changed);
        assert!(diff.new_nomination.is_some());
        assert_eq!(diff.new_nomination.as_ref().unwrap().player_name, "Player C");
        assert!(!diff.nomination_cleared);
        assert!(!diff.bid_updated);
    }

    #[test]
    fn diff_no_changes() {
        let state = StateUpdatePayload {
            picks: vec![make_pick_payload(1, "team_1", "Player A", "SP", 20)],
            current_nomination: Some(make_nomination("p2", "Player B", 10, Some("team_2"))),
            ..Default::default()
        };

        let diff = compute_state_diff(&Some(state.clone()), &state);
        assert!(diff.new_picks.is_empty());
        assert!(!diff.nomination_changed);
        assert!(diff.new_nomination.is_none());
        assert!(!diff.nomination_cleared);
        assert!(!diff.bid_updated);
    }

    #[test]
    fn diff_new_picks() {
        let previous = StateUpdatePayload {
            picks: vec![make_pick_payload(1, "team_1", "Player A", "SP", 20)],
            current_nomination: None,
            ..Default::default()
        };
        let current = StateUpdatePayload {
            picks: vec![
                make_pick_payload(1, "team_1", "Player A", "SP", 20),
                make_pick_payload(2, "team_2", "Player B", "CF", 30),
                make_pick_payload(3, "team_3", "Player C", "1B", 15),
            ],
            current_nomination: None,
            ..Default::default()
        };

        let diff = compute_state_diff(&Some(previous), &current);
        assert_eq!(diff.new_picks.len(), 2);
        assert_eq!(diff.new_picks[0].player_name, "Player B");
        assert_eq!(diff.new_picks[1].player_name, "Player C");
    }

    #[test]
    fn diff_nomination_appeared() {
        let previous = StateUpdatePayload {
            picks: vec![],
            current_nomination: None,
            ..Default::default()
        };
        let current = StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(make_nomination("p1", "Player A", 1, None)),
            ..Default::default()
        };

        let diff = compute_state_diff(&Some(previous), &current);
        assert!(diff.nomination_changed);
        assert!(diff.new_nomination.is_some());
        assert_eq!(
            diff.new_nomination.as_ref().unwrap().player_name,
            "Player A"
        );
        assert!(!diff.nomination_cleared);
    }

    #[test]
    fn diff_nomination_cleared() {
        let previous = StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(make_nomination("p1", "Player A", 10, Some("team_2"))),
            ..Default::default()
        };
        let current = StateUpdatePayload {
            picks: vec![make_pick_payload(1, "team_2", "Player A", "SP", 10)],
            current_nomination: None,
            ..Default::default()
        };

        let diff = compute_state_diff(&Some(previous), &current);
        assert!(diff.nomination_changed);
        assert!(diff.nomination_cleared);
        assert!(diff.new_nomination.is_none());
        assert_eq!(diff.new_picks.len(), 1);
    }

    #[test]
    fn diff_nomination_changed_player() {
        let previous = StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(make_nomination("p1", "Player A", 10, Some("team_2"))),
            ..Default::default()
        };
        let current = StateUpdatePayload {
            picks: vec![make_pick_payload(1, "team_2", "Player A", "SP", 10)],
            current_nomination: Some(make_nomination("p2", "Player B", 1, None)),
            ..Default::default()
        };

        let diff = compute_state_diff(&Some(previous), &current);
        assert!(diff.nomination_changed);
        assert!(diff.new_nomination.is_some());
        assert_eq!(
            diff.new_nomination.as_ref().unwrap().player_name,
            "Player B"
        );
        assert!(!diff.nomination_cleared);
    }

    #[test]
    fn diff_bid_updated() {
        let previous = StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(make_nomination("p1", "Player A", 5, None)),
            ..Default::default()
        };
        let current = StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(make_nomination("p1", "Player A", 12, Some("team_3"))),
            ..Default::default()
        };

        let diff = compute_state_diff(&Some(previous), &current);
        assert!(!diff.nomination_changed);
        assert!(diff.bid_updated);
        assert!(diff.new_nomination.is_some());
        assert_eq!(diff.new_nomination.as_ref().unwrap().current_bid, 12);
        assert_eq!(
            diff.new_nomination.as_ref().unwrap().current_bidder,
            Some("team_3".to_string())
        );
    }

    #[test]
    fn diff_bid_updated_bidder_only() {
        let previous = StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(make_nomination("p1", "Player A", 10, Some("team_2"))),
            ..Default::default()
        };
        let current = StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(make_nomination("p1", "Player A", 10, Some("team_3"))),
            ..Default::default()
        };

        let diff = compute_state_diff(&Some(previous), &current);
        assert!(!diff.nomination_changed);
        assert!(diff.bid_updated);
        assert_eq!(
            diff.new_nomination.as_ref().unwrap().current_bidder,
            Some("team_3".to_string())
        );
    }

    #[test]
    fn reconcile_budgets_overrides_local_tracking() {
        let mut state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());

        // Record some picks to set budget_spent/budget_remaining via local tracking
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "team_1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "team_2".to_string(),
            team_name: "Team 2".to_string(),
            player_name: "Shohei Ohtani".to_string(),
            position: "SP".to_string(),
            price: 50,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        // Verify local tracking state before reconciliation
        let team1 = state.team("team_1").unwrap();
        assert_eq!(team1.budget_spent, 45);
        assert_eq!(team1.budget_remaining, 215);
        let team2 = state.team("team_2").unwrap();
        assert_eq!(team2.budget_spent, 50);
        assert_eq!(team2.budget_remaining, 210);

        // Reconcile with ESPN data that differs from local tracking
        // (simulating drift or missed picks)
        let espn_budgets = vec![
            TeamBudgetPayload {
                team_name: "Team 1".to_string(),
                budget: 200, // ESPN says $200 remaining (vs local $215)
            },
            TeamBudgetPayload {
                team_name: "Team 2".to_string(),
                budget: 205, // ESPN says $205 remaining (vs local $210)
            },
        ];
        state.reconcile_budgets(&espn_budgets);

        // Verify ESPN values override local tracking
        let team1 = state.team("team_1").unwrap();
        assert_eq!(team1.budget_remaining, 200);
        assert_eq!(team1.budget_spent, 60); // 260 - 200
        let team2 = state.team("team_2").unwrap();
        assert_eq!(team2.budget_remaining, 205);
        assert_eq!(team2.budget_spent, 55); // 260 - 205
    }

    #[test]
    fn reconcile_budgets_auto_registers_unknown_teams() {
        let mut state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());

        // Record a pick to establish some local state
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "team_1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        let initial_team_count = state.teams.len();

        // Reconcile with ESPN data that includes an unknown team name
        let espn_budgets = vec![
            TeamBudgetPayload {
                team_name: "Jamaica Jiggle Party".to_string(),
                budget: 100,
            },
            TeamBudgetPayload {
                team_name: "Team 1".to_string(),
                budget: 210,
            },
        ];
        state.reconcile_budgets(&espn_budgets);

        // Team 1 should be updated from ESPN
        let team1 = state.team("team_1").unwrap();
        assert_eq!(team1.budget_remaining, 210);
        assert_eq!(team1.budget_spent, 50); // 260 - 210

        // Unknown team should have been auto-registered
        assert_eq!(state.teams.len(), initial_team_count + 1);
        let new_team = state.teams.iter().find(|t| t.team_name == "Jamaica Jiggle Party").unwrap();
        assert_eq!(new_team.budget_remaining, 100);
        assert_eq!(new_team.budget_spent, 160); // 260 - 100

        // Team 2 should be unaffected (no ESPN data for it)
        let team2 = state.team("team_2").unwrap();
        assert_eq!(team2.budget_remaining, 260);
        assert_eq!(team2.budget_spent, 0);
    }

    #[test]
    fn record_pick_auto_registers_unknown_team() {
        let mut state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        let initial_team_count = state.teams.len();

        // Simulate a pick from a team name that doesn't match any config placeholder
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "".to_string(),
            team_name: "Jamaica Jiggle Party".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        // The unknown team should have been auto-registered
        assert_eq!(state.teams.len(), initial_team_count + 1);
        assert_eq!(state.pick_count, 1);
        assert_eq!(state.picks.len(), 1);

        // The new team should have correct budget and roster
        let new_team = state.teams.iter().find(|t| t.team_name == "Jamaica Jiggle Party").unwrap();
        assert_eq!(new_team.budget_spent, 45);
        assert_eq!(new_team.budget_remaining, 215);
        assert_eq!(new_team.roster.filled_count(), 1);
    }

    #[test]
    fn record_pick_reuses_auto_registered_team() {
        let mut state = DraftState::new(test_teams(), "team_1", 260, &test_roster_config());

        // First pick auto-registers the team
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "".to_string(),
            team_name: "Jamaica Jiggle Party".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        let team_count_after_first = state.teams.len();

        // Second pick from the same team should reuse the existing entry
        state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "".to_string(),
            team_name: "Jamaica Jiggle Party".to_string(),
            player_name: "Shohei Ohtani".to_string(),
            position: "SP".to_string(),
            price: 50,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        // No new team entry should be created
        assert_eq!(state.teams.len(), team_count_after_first);

        let team = state.teams.iter().find(|t| t.team_name == "Jamaica Jiggle Party").unwrap();
        assert_eq!(team.budget_spent, 95);
        assert_eq!(team.budget_remaining, 165);
        assert_eq!(team.roster.filled_count(), 2);
    }

    #[test]
    fn diff_detects_new_picks_when_reordered() {
        // Previous had picks 1 and 2
        let previous = StateUpdatePayload {
            picks: vec![
                make_pick_payload(1, "team_1", "Player A", "SP", 20),
                make_pick_payload(2, "team_2", "Player B", "CF", 30),
            ],
            current_nomination: None,
            ..Default::default()
        };
        // Current has picks 2, 1, 3 (reordered, with one new pick)
        let current = StateUpdatePayload {
            picks: vec![
                make_pick_payload(2, "team_2", "Player B", "CF", 30),
                make_pick_payload(1, "team_1", "Player A", "SP", 20),
                make_pick_payload(3, "team_3", "Player C", "1B", 15),
            ],
            current_nomination: None,
            ..Default::default()
        };

        let diff = compute_state_diff(&Some(previous), &current);
        assert_eq!(diff.new_picks.len(), 1);
        assert_eq!(diff.new_picks[0].pick_number, 3);
        assert_eq!(diff.new_picks[0].player_name, "Player C");
    }
}
