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
    /// Teams start empty and are populated dynamically from ESPN's live data
    /// via `reconcile_budgets()`. The `my_team_idx` defaults to 0 and is
    /// updated when the extension identifies the user's team.
    ///
    /// # Arguments
    /// - `salary_cap`: Per-team salary cap
    /// - `roster_config`: Position -> slot count mapping from league config
    pub fn new(
        salary_cap: u32,
        roster_config: &HashMap<String, usize>,
    ) -> Self {
        DraftState {
            teams: Vec::new(),
            picks: Vec::new(),
            current_nomination: None,
            pick_count: 0,
            total_picks: 0,
            my_team_idx: 0,
            nomination_order: Vec::new(),
            salary_cap,
            roster_config: roster_config.clone(),
        }
    }

    /// Identify the user's team by matching a team name from the extension.
    ///
    /// The ESPN extension's `identifyMyTeam()` returns a team name (not an ID).
    /// After teams are registered via `reconcile_budgets()`, this method
    /// finds and sets `my_team_idx` by matching the name.
    pub fn set_my_team_by_name(&mut self, team_name: &str) {
        if let Some(idx) = self.teams.iter().position(|t| t.team_name == team_name) {
            self.my_team_idx = idx;
        } else {
            warn!(
                "Could not find team matching '{}' — my_team_idx remains at {}",
                team_name, self.my_team_idx
            );
        }
    }

    /// Record a completed draft pick.
    ///
    /// Updates the winning team's budget and roster, increments the pick count,
    /// and appends the pick to the history.
    ///
    /// Deduplication: if a pick with the same `pick_number` has already been
    /// recorded, the call is a no-op. This guards against unstable pick numbers
    /// from the ESPN DOM scraper (virtualized pick list) and crash-recovery
    /// replays that could otherwise cause the same player to appear in multiple
    /// roster slots.
    pub fn record_pick(&mut self, pick: DraftPick) {
        // Skip if this pick_number was already recorded (deduplication).
        if self.picks.iter().any(|p| p.pick_number == pick.pick_number) {
            return;
        }

        // Skip if this player was already recorded (deduplication by identity).
        // ESPN's virtualized pick list can renumber picks, causing the same
        // player to appear with a different pick_number. Deduplicate by ESPN
        // player ID when available, falling back to player name.
        let dominated = match &pick.espn_player_id {
            Some(id) if !id.is_empty() => self
                .picks
                .iter()
                .any(|p| p.espn_player_id.as_deref() == Some(id.as_str())),
            _ => self
                .picks
                .iter()
                .any(|p| p.player_name == pick.player_name),
        };
        if dominated {
            return;
        }

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

        if let Some(team) = team_idx.map(|i| &mut self.teams[i]) {
            team.budget_spent += pick.price;
            team.budget_remaining = team.budget_remaining.saturating_sub(pick.price);
            team.roster.add_player_with_slots(
                &pick.player_name,
                &pick.position,
                pick.price,
                &pick.eligible_slots,
                pick.espn_player_id.as_deref(),
            );
        }
        self.pick_count += 1;
        self.picks.push(pick);
    }

    /// Reconcile team budgets with data scraped from the ESPN DOM.
    ///
    /// On the first call (when `self.teams` is empty), this auto-registers
    /// all teams from the ESPN data, building the full team registry.
    /// On subsequent calls, it uses the ESPN-reported remaining budget as the
    /// source of truth and adjusts `budget_remaining` and `budget_spent`.
    /// Returns `true` if this call registered teams for the first time.
    pub fn reconcile_budgets(&mut self, espn_budgets: &[TeamBudgetPayload]) -> bool {
        if self.teams.is_empty() && !espn_budgets.is_empty() {
            // First call: auto-register all teams from ESPN data
            for budget_data in espn_budgets {
                self.teams.push(TeamState {
                    team_id: budget_data.team_id.clone(),
                    team_name: budget_data.team_name.clone(),
                    roster: Roster::new(&self.roster_config),
                    budget_spent: self.salary_cap.saturating_sub(budget_data.budget),
                    budget_remaining: budget_data.budget,
                });
            }

            // Compute total picks and nomination order now that teams are registered
            let draftable_per_team = self
                .teams
                .first()
                .map(|t| t.roster.draftable_count())
                .unwrap_or(0);
            self.total_picks = draftable_per_team * self.teams.len();
            self.nomination_order = (0..self.teams.len()).collect();

            // Replay any picks stored during crash recovery before teams existed
            self.replay_pending_picks();

            return true;
        }

        for budget_data in espn_budgets {
            // Match by team_name since the pick train names are the canonical identifiers
            if let Some(team) = self
                .teams
                .iter_mut()
                .find(|t| t.team_name == budget_data.team_name)
            {
                team.budget_remaining = budget_data.budget;
                team.budget_spent = self.salary_cap.saturating_sub(budget_data.budget);
            }
        }
        false
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

    /// Reference to the user's team state, if teams have been registered.
    ///
    /// Returns `None` before `reconcile_budgets()` has populated the team list.
    pub fn my_team(&self) -> Option<&TeamState> {
        self.teams.get(self.my_team_idx)
    }

    /// Restore the draft state by replaying a sequence of picks.
    ///
    /// This is used for crash recovery: given a saved list of picks,
    /// replay them all to reconstruct the full state.
    ///
    /// If teams have not been registered yet (empty teams list), the picks
    /// are stored in `self.picks` and `self.pick_count` is set, but team
    /// budgets/rosters are not updated. When `reconcile_budgets()` later
    /// registers teams, it will call `replay_pending_picks()` to apply
    /// the stored picks against the newly registered teams.
    pub fn restore_from_picks(&mut self, picks: Vec<DraftPick>) {
        self.current_nomination = None;

        if self.teams.is_empty() {
            // Teams not registered yet — store picks for deferred replay
            self.pick_count = picks.len();
            self.picks = picks;
            return;
        }

        // Reset state: rebuild rosters from stored config, reset budgets
        for team in &mut self.teams {
            team.budget_spent = 0;
            team.budget_remaining = self.salary_cap;
            team.roster = Roster::new(&self.roster_config);
        }
        self.picks.clear();
        self.pick_count = 0;

        for pick in picks {
            self.record_pick(pick);
        }
    }

    /// Replay stored picks against newly registered teams.
    ///
    /// Called by `reconcile_budgets()` after the first team registration
    /// when there are pending picks from crash recovery.
    fn replay_pending_picks(&mut self) {
        if self.picks.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.picks);
        self.pick_count = 0;
        for pick in pending {
            self.record_pick(pick);
        }
    }
}

// --- Differential State Detection ---

/// Budget data for a team as scraped from the ESPN DOM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamBudgetPayload {
    /// ESPN team ID extracted from the pick train (e.g. "1", "2").
    pub team_id: String,
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

    /// Create ESPN-style team budget data for 10 teams, each with $260 budget.
    fn test_espn_budgets() -> Vec<TeamBudgetPayload> {
        (1..=10)
            .map(|i| TeamBudgetPayload {
                team_id: format!("{}", i),
                team_name: format!("Team {}", i),
                budget: 260,
            })
            .collect()
    }

    /// Helper: create a DraftState with teams pre-registered from ESPN data.
    fn create_test_state() -> DraftState {
        let mut state = DraftState::new(260, &test_roster_config());
        state.reconcile_budgets(&test_espn_budgets());
        state.set_my_team_by_name("Team 1");
        state
    }

    #[test]
    fn draft_state_creation_starts_empty() {
        let state = DraftState::new(260, &test_roster_config());
        assert_eq!(state.teams.len(), 0);
        assert_eq!(state.pick_count, 0);
        assert_eq!(state.total_picks, 0);
        assert_eq!(state.my_team_idx, 0);
        assert!(state.current_nomination.is_none());
    }

    #[test]
    fn reconcile_budgets_registers_teams() {
        let mut state = DraftState::new(260, &test_roster_config());
        state.reconcile_budgets(&test_espn_budgets());
        assert_eq!(state.teams.len(), 10);
        // Teams are in ESPN order (not sorted by ID)
        assert_eq!(state.teams[0].team_name, "Team 1");
        assert_eq!(state.teams[0].team_id, "1");
        assert_eq!(state.teams[9].team_name, "Team 10");
    }

    #[test]
    fn draft_state_total_picks_after_registration() {
        let state = create_test_state();
        // 26 draftable slots per team * 10 teams = 260
        assert_eq!(state.total_picks, 260);
    }

    #[test]
    fn set_my_team_by_name() {
        let mut state = DraftState::new(260, &test_roster_config());
        state.reconcile_budgets(&test_espn_budgets());
        state.set_my_team_by_name("Team 3");
        let my = state.my_team().expect("my_team should be Some after reconcile");
        assert_eq!(my.team_name, "Team 3");
        assert_eq!(my.budget_remaining, 260);
    }

    #[test]
    fn my_team_returns_none_when_teams_empty() {
        let state = DraftState::new(260, &test_roster_config());
        assert!(state.my_team().is_none());
    }

    #[test]
    fn record_pick_updates_budget() {
        let mut state = create_test_state();
        let pick = DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        };
        state.record_pick(pick);

        let team = state.team("1").unwrap();
        assert_eq!(team.budget_spent, 45);
        assert_eq!(team.budget_remaining, 215);
        assert_eq!(state.pick_count, 1);
        assert_eq!(state.picks.len(), 1);
    }

    #[test]
    fn record_pick_updates_roster() {
        let mut state = create_test_state();
        let pick = DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        };
        state.record_pick(pick);

        let team = state.team("1").unwrap();
        assert_eq!(team.roster.filled_count(), 1);
    }

    #[test]
    fn record_multiple_picks() {
        let mut state = create_test_state();

        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "2".to_string(),
            team_name: "Team 2".to_string(),
            player_name: "Shohei Ohtani".to_string(),
            position: "SP".to_string(),
            price: 50,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 3,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mookie Betts".to_string(),
            position: "RF".to_string(),
            price: 35,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        assert_eq!(state.pick_count, 3);
        assert_eq!(state.picks.len(), 3);

        let team1 = state.team("1").unwrap();
        assert_eq!(team1.budget_spent, 80);
        assert_eq!(team1.budget_remaining, 180);
        assert_eq!(team1.roster.filled_count(), 2);

        let team2 = state.team("2").unwrap();
        assert_eq!(team2.budget_spent, 50);
        assert_eq!(team2.budget_remaining, 210);
    }

    #[test]
    fn total_spent() {
        let mut state = create_test_state();
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Player A".to_string(),
            position: "SP".to_string(),
            price: 30,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "3".to_string(),
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
        let state = create_test_state();
        assert!(state.team("5").is_some());
        assert_eq!(state.team("5").unwrap().team_name, "Team 5");
        assert!(state.team("nonexistent").is_none());
    }

    #[test]
    fn restore_from_picks() {
        let roster_config = test_roster_config();

        let picks = vec![
            DraftPick {
                pick_number: 1,
                team_id: "1".to_string(),
                team_name: "Team 1".to_string(),
                player_name: "Mike Trout".to_string(),
                position: "CF".to_string(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
            },
            DraftPick {
                pick_number: 2,
                team_id: "2".to_string(),
                team_name: "Team 2".to_string(),
                player_name: "Shohei Ohtani".to_string(),
                position: "SP".to_string(),
                price: 50,
                espn_player_id: None,
                eligible_slots: vec![],
            },
            DraftPick {
                pick_number: 3,
                team_id: "1".to_string(),
                team_name: "Team 1".to_string(),
                player_name: "Mookie Betts".to_string(),
                position: "RF".to_string(),
                price: 35,
                espn_player_id: None,
                eligible_slots: vec![],
            },
        ];

        // Create a state with teams registered and restore from picks
        let mut state = DraftState::new(260, &roster_config);
        state.reconcile_budgets(&test_espn_budgets());
        state.set_my_team_by_name("Team 1");
        state.restore_from_picks(picks);

        assert_eq!(state.pick_count, 3);
        assert_eq!(state.picks.len(), 3);

        let team1 = state.team("1").unwrap();
        assert_eq!(team1.budget_spent, 80);
        assert_eq!(team1.budget_remaining, 180);
        assert_eq!(team1.roster.filled_count(), 2);
    }

    #[test]
    fn restore_from_picks_resets_previous_state() {
        let roster_config = test_roster_config();
        let mut state = DraftState::new(260, &roster_config);
        state.reconcile_budgets(&test_espn_budgets());
        state.set_my_team_by_name("Team 1");

        // Record some picks first
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
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
            team_id: "2".to_string(),
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
        let team1 = state.team("1").unwrap();
        assert_eq!(team1.budget_spent, 0); // Old pick should be gone
        assert_eq!(team1.budget_remaining, 260);
        let team2 = state.team("2").unwrap();
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
        let mut state = create_test_state();

        // Record some picks to set budget_spent/budget_remaining via local tracking
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "2".to_string(),
            team_name: "Team 2".to_string(),
            player_name: "Shohei Ohtani".to_string(),
            position: "SP".to_string(),
            price: 50,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        // Verify local tracking state before reconciliation
        let team1 = state.team("1").unwrap();
        assert_eq!(team1.budget_spent, 45);
        assert_eq!(team1.budget_remaining, 215);
        let team2 = state.team("2").unwrap();
        assert_eq!(team2.budget_spent, 50);
        assert_eq!(team2.budget_remaining, 210);

        // Reconcile with ESPN data that differs from local tracking
        // (simulating drift or missed picks)
        let espn_budgets = vec![
            TeamBudgetPayload {
                team_id: "1".to_string(),
                team_name: "Team 1".to_string(),
                budget: 200, // ESPN says $200 remaining (vs local $215)
            },
            TeamBudgetPayload {
                team_id: "2".to_string(),
                team_name: "Team 2".to_string(),
                budget: 205, // ESPN says $205 remaining (vs local $210)
            },
        ];
        state.reconcile_budgets(&espn_budgets);

        // Verify ESPN values override local tracking
        let team1 = state.team("1").unwrap();
        assert_eq!(team1.budget_remaining, 200);
        assert_eq!(team1.budget_spent, 60); // 260 - 200
        let team2 = state.team("2").unwrap();
        assert_eq!(team2.budget_remaining, 205);
        assert_eq!(team2.budget_spent, 55); // 260 - 205
    }

    #[test]
    fn reconcile_budgets_skips_non_matching_teams() {
        let mut state = create_test_state();

        // Record a pick to establish some local state
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        // Reconcile with ESPN data that includes a non-existent team
        let espn_budgets = vec![
            TeamBudgetPayload {
                team_id: "99".to_string(),
                team_name: "Nonexistent Team".to_string(),
                budget: 100,
            },
            TeamBudgetPayload {
                team_id: "1".to_string(),
                team_name: "Team 1".to_string(),
                budget: 210,
            },
        ];
        state.reconcile_budgets(&espn_budgets);

        // Team 1 should be updated from ESPN
        let team1 = state.team("1").unwrap();
        assert_eq!(team1.budget_remaining, 210);
        assert_eq!(team1.budget_spent, 50); // 260 - 210

        // Team 2 should be unaffected (no ESPN data for it)
        let team2 = state.team("2").unwrap();
        assert_eq!(team2.budget_remaining, 260);
        assert_eq!(team2.budget_spent, 0);
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

    // -----------------------------------------------------------------------
    // Tests: record_pick deduplication
    // -----------------------------------------------------------------------

    #[test]
    fn record_pick_dedup_by_pick_number() {
        let mut state = create_test_state();

        let pick = DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        };

        state.record_pick(pick.clone());
        assert_eq!(state.pick_count, 1);
        assert_eq!(state.picks.len(), 1);

        let team = state.team("1").unwrap();
        assert_eq!(team.roster.filled_count(), 1);
        assert_eq!(team.budget_spent, 45);

        // Record same pick_number again — should be a no-op
        state.record_pick(pick);
        assert_eq!(state.pick_count, 1, "pick_count should not increase on dup");
        assert_eq!(state.picks.len(), 1, "picks vec should not grow on dup");

        let team = state.team("1").unwrap();
        assert_eq!(
            team.roster.filled_count(),
            1,
            "roster should still have 1 player after dup"
        );
        assert_eq!(
            team.budget_spent, 45,
            "budget_spent should not change on dup"
        );
    }

    #[test]
    fn record_pick_dedup_by_player_name_different_pick_number() {
        let mut state = create_test_state();

        // Record pick #1 for Mike Trout on Team 1
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        let team = state.team("1").unwrap();
        assert_eq!(team.roster.filled_count(), 1);

        // Record same player with DIFFERENT pick_number (simulates ESPN renumbering).
        // The entire call should be a no-op: no new pick in picks list, no budget
        // or roster changes.
        state.record_pick(DraftPick {
            pick_number: 101, // Different number (renumbered)
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        assert_eq!(
            state.pick_count, 1,
            "pick_count should not increase (same player)"
        );
        assert_eq!(
            state.picks.len(),
            1,
            "picks vec should not grow (same player)"
        );

        let team = state.team("1").unwrap();
        assert_eq!(
            team.roster.filled_count(),
            1,
            "roster should still have 1 player"
        );
        assert_eq!(
            team.budget_spent, 45,
            "budget_spent should not change"
        );
    }

    #[test]
    fn record_pick_dedup_by_espn_player_id() {
        let mut state = create_test_state();

        // Record pick #1 for player with ESPN ID
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: Some("33039".to_string()),
            eligible_slots: vec![],
        });

        // Record same ESPN player ID with different pick_number — should be no-op
        state.record_pick(DraftPick {
            pick_number: 101,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: Some("33039".to_string()),
            eligible_slots: vec![],
        });

        assert_eq!(state.pick_count, 1, "same ESPN ID should dedup");
        assert_eq!(state.picks.len(), 1);
    }

    #[test]
    fn record_pick_different_players_same_team_not_deduped() {
        let mut state = create_test_state();

        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "1".to_string(),
            team_name: "Team 1".to_string(),
            player_name: "Mookie Betts".to_string(),
            position: "RF".to_string(),
            price: 35,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        assert_eq!(state.pick_count, 2);
        assert_eq!(state.picks.len(), 2);

        let team = state.team("1").unwrap();
        assert_eq!(team.roster.filled_count(), 2);
        assert_eq!(team.budget_spent, 80);

        // Verify each player is in the correct slot
        let cf = team
            .roster
            .slots
            .iter()
            .find(|s| s.position == crate::draft::pick::Position::CenterField)
            .unwrap();
        assert_eq!(cf.player.as_ref().unwrap().name, "Mike Trout");

        let rf = team
            .roster
            .slots
            .iter()
            .find(|s| s.position == crate::draft::pick::Position::RightField)
            .unwrap();
        assert_eq!(rf.player.as_ref().unwrap().name, "Mookie Betts");
    }

    #[test]
    fn crash_recovery_plus_first_update_no_duplicate_roster_entries() {
        let roster_config = test_roster_config();
        let mut state = DraftState::new(260, &roster_config);

        // Simulate crash recovery: store picks before teams are registered
        let recovery_picks = vec![
            DraftPick {
                pick_number: 1,
                team_id: "Team Alpha".to_string(),
                team_name: "Team Alpha".to_string(),
                player_name: "Shohei Ohtani".to_string(),
                position: "DH".to_string(),
                price: 62,
                espn_player_id: None,
                eligible_slots: vec![],
            },
            DraftPick {
                pick_number: 2,
                team_id: "Team Beta".to_string(),
                team_name: "Team Beta".to_string(),
                player_name: "Aaron Judge".to_string(),
                position: "CF".to_string(),
                price: 55,
                espn_player_id: None,
                eligible_slots: vec![],
            },
        ];
        state.restore_from_picks(recovery_picks);

        // Now simulate first extension STATE_UPDATE arriving with the same picks
        // (this is what happens after crash recovery when the extension reconnects).
        // process_new_picks would call record_pick for each.
        state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "Team Alpha".to_string(),
            team_name: "Team Alpha".to_string(),
            player_name: "Shohei Ohtani".to_string(),
            position: "DH".to_string(),
            price: 62,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "Team Beta".to_string(),
            team_name: "Team Beta".to_string(),
            player_name: "Aaron Judge".to_string(),
            position: "CF".to_string(),
            price: 55,
            espn_player_id: None,
            eligible_slots: vec![],
        });
        // Also a new pick
        state.record_pick(DraftPick {
            pick_number: 3,
            team_id: "Team Alpha".to_string(),
            team_name: "Team Alpha".to_string(),
            player_name: "Mookie Betts".to_string(),
            position: "SS".to_string(),
            price: 40,
            espn_player_id: None,
            eligible_slots: vec![],
        });

        // Picks 1 and 2 should be deduped (already in self.picks from recovery)
        // Only pick 3 should be truly new
        assert_eq!(state.picks.len(), 3, "Should have 3 unique picks");
        assert_eq!(state.pick_count, 3);

        // Now register teams and replay
        let budgets = vec![
            TeamBudgetPayload {
                team_id: "1".to_string(),
                team_name: "Team Alpha".to_string(),
                budget: 158, // 260 - 62 - 40
            },
            TeamBudgetPayload {
                team_id: "2".to_string(),
                team_name: "Team Beta".to_string(),
                budget: 205, // 260 - 55
            },
        ];
        state.reconcile_budgets(&budgets);
        state.set_my_team_by_name("Team Alpha");

        // Team Alpha should have exactly 2 players (Ohtani + Betts), not duplicates
        let my_team = state.my_team().unwrap();
        assert_eq!(
            my_team.roster.filled_count(),
            2,
            "Team Alpha should have exactly 2 players, not duplicates"
        );

        // Verify the correct players
        assert!(my_team.roster.has_player("Shohei Ohtani", None));
        assert!(my_team.roster.has_player("Mookie Betts", None));
        assert!(!my_team.roster.has_player("Aaron Judge", None));

        // Team Beta should have exactly 1 player (Judge)
        let team_beta = state.team("2").unwrap();
        assert_eq!(team_beta.roster.filled_count(), 1);
        assert!(team_beta.roster.has_player("Aaron Judge", None));
    }
}
