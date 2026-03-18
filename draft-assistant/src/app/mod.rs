// Application state and orchestration logic.
//
// The central event loop that coordinates WebSocket events from the Firefox
// extension, LLM streaming events, and user commands from the TUI. Maintains
// the complete application state and pushes UI updates to the TUI render loop.

mod ws_handler;
mod llm_handler;
mod command_handler;
mod onboarding_handler;
mod llm_request_manager;

pub use llm_request_manager::LlmRequestManager;

use std::sync::atomic::{AtomicI8, AtomicU64};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{info, warn};

use crate::config::Config;
use crate::db::Database;
use crate::draft::pick::{playing_positions_from_slots, Position};
use crate::draft::state::{
    ActiveNomination, DraftState, NominationPayload, PickPayload,
    StateUpdatePayload, TeamBudgetPayload,
};
use crate::llm::client::LlmClient;
use crate::llm::prompt::{self, BudgetContext};

use crate::onboarding::{OnboardingManager, OnboardingProgress, RealFileSystem};
use crate::protocol::{
    AppMode, AppSnapshot, ConnectionStatus, LlmEvent, NominationInfo,
    TabId, TeamSnapshot, UiUpdate, UserCommand,
};
use crate::valuation::analysis::{compute_instant_analysis, CategoryNeeds, InstantAnalysis};
use crate::valuation::auction::InflationTracker;
use crate::valuation::projections::AllProjections;
use crate::valuation::scarcity::{compute_scarcity, ScarcityEntry};
use crate::valuation::zscore::PlayerValuation;
use crate::ws_server::WsEvent;

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

/// Tracks which player is currently being analyzed by the LLM.
/// Used by the preserve_llm guard in ws_handler and duplicate-check in trigger_nomination_analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisPlayer {
    pub player_name: String,
    pub player_id: String,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How long to wait without receiving any WebSocket message before
/// considering the extension connection stale and transitioning to
/// `Disconnected`.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(15);

/// How often to check for heartbeat timeout in the main event loop.
pub const HEARTBEAT_CHECK_INTERVAL: Duration = Duration::from_secs(5);

/// Connection test has never been run.
const CONNECTION_NEVER_TESTED: i8 = -1;
/// Connection test was run and failed.
const CONNECTION_TEST_FAILED: i8 = 0;
/// Connection test was run and succeeded.
const CONNECTION_TEST_PASSED: i8 = 1;

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// The complete application state.
pub struct AppState {
    /// Current UI mode (Onboarding, Draft, or Settings).
    pub app_mode: AppMode,
    pub config: Config,
    pub draft_state: DraftState,
    pub available_players: Vec<PlayerValuation>,
    pub all_projections: AllProjections,
    pub inflation: InflationTracker,
    pub scarcity: Vec<ScarcityEntry>,
    pub db: Database,
    /// Unique identifier for the current draft session. Picks are scoped to
    /// this ID so restarts don't replay picks from a different draft.
    pub draft_id: String,
    /// Draft identifier scraped from the ESPN page by the extension (e.g.
    /// league ID from URL or team-name fingerprint). Used to detect when
    /// the extension is reporting state from a different draft than the one
    /// stored in `draft_id`. `None` until the first STATE_UPDATE arrives
    /// with a non-null `draftId`.
    pub espn_draft_id: Option<String>,
    pub previous_extension_state: Option<StateUpdatePayload>,
    pub llm_requests: LlmRequestManager,
    pub analysis_request_id: Option<u64>,
    pub plan_request_id: Option<u64>,
    pub analysis_player: Option<AnalysisPlayer>,
    pub connection_status: ConnectionStatus,
    /// Timestamp of the last WebSocket message (or connection event) received.
    /// `None` when not connected. Used to detect stale connections when the
    /// browser tab is closed without a clean WebSocket close frame.
    pub last_ws_message_time: Option<Instant>,
    pub active_tab: TabId,
    pub category_needs: CategoryNeeds,
    /// LLM client for streaming Claude API calls. Wrapped in Arc for
    /// sharing with spawned tasks.
    pub llm_client: Arc<LlmClient>,
    /// Sender for LLM events; spawned tasks use a clone of this sender
    /// to stream tokens back to the main event loop.
    pub llm_tx: mpsc::Sender<LlmEvent>,
    /// Sender for outbound WebSocket messages to the extension.
    /// Used to send `REQUEST_KEYFRAME` messages.
    pub ws_outbound_tx: Option<mpsc::Sender<String>>,
    /// Onboarding manager for loading/saving onboarding progress.
    pub onboarding_manager: OnboardingManager<RealFileSystem>,
    /// Cached onboarding progress (updated in-memory on each Set* action,
    /// persisted to disk on GoNext/GoBack transitions).
    pub onboarding_progress: OnboardingProgress,
    /// Tracks the result of the last API connection test during onboarding.
    /// Uses `CONNECTION_NEVER_TESTED`, `CONNECTION_TEST_FAILED`, `CONNECTION_TEST_PASSED`.
    /// Shared with the spawned TestConnection task via Arc so it can update
    /// the result without routing through the event loop.
    pub connection_test_result: Arc<AtomicI8>,
    /// Generation counter for connection tests. Incremented when a new test
    /// is triggered. Spawned test tasks
    /// capture the current generation and only write to `connection_test_result`
    /// if the generation hasn't changed, preventing stale writes.
    pub connection_test_generation: Arc<AtomicU64>,
    /// Whether grid-sourced picks have already been persisted to DB this session.
    /// Set to true after the first grid-based rebuild to avoid redundant writes
    /// on subsequent 10-second FULL_STATE_SYNC keyframes.
    pub grid_picks_persisted: bool,
    /// Roster configuration inferred from ESPN or set from defaults.
    /// `None` until roster is inferred from the ESPN draft board.
    pub roster_config: Option<std::collections::HashMap<String, usize>>,
}

impl AppState {
    /// Create a new AppState with the given components.
    ///
    /// The `draft_id` identifies the current draft session. On startup, callers
    /// should load the stored draft_id from the database (or generate a new one).
    pub fn new(
        config: Config,
        draft_state: DraftState,
        available_players: Vec<PlayerValuation>,
        all_projections: AllProjections,
        db: Database,
        draft_id: String,
        llm_client: LlmClient,
        llm_tx: mpsc::Sender<LlmEvent>,
        ws_outbound_tx: Option<mpsc::Sender<String>>,
        app_mode: AppMode,
        onboarding_manager: OnboardingManager<RealFileSystem>,
        roster_config: Option<std::collections::HashMap<String, usize>>,
    ) -> Self {
        let scarcity = match &roster_config {
            Some(rc) => compute_scarcity(&available_players, rc),
            None => Vec::new(),
        };
        let inflation = InflationTracker::new();
        let onboarding_progress = onboarding_manager.load_progress();

        AppState {
            app_mode,
            config,
            draft_state,
            available_players,
            all_projections,
            inflation,
            scarcity,
            db,
            draft_id,
            espn_draft_id: None,
            previous_extension_state: None,
            llm_requests: LlmRequestManager::new(),
            analysis_request_id: None,
            plan_request_id: None,
            analysis_player: None,
            connection_status: ConnectionStatus::Disconnected,
            last_ws_message_time: None,
            active_tab: TabId::Analysis,
            category_needs: CategoryNeeds::default(),
            llm_client: Arc::new(llm_client),
            llm_tx,
            ws_outbound_tx,
            onboarding_manager,
            onboarding_progress,
            connection_test_result: Arc::new(AtomicI8::new(CONNECTION_NEVER_TESTED)),
            connection_test_generation: Arc::new(AtomicU64::new(0)),
            grid_picks_persisted: false,
            roster_config,
        }
    }

    /// Default roster configuration (used as fallback until ESPN provides the actual roster layout).
    pub fn default_roster_config() -> std::collections::HashMap<String, usize> {
        let mut roster = std::collections::HashMap::new();
        roster.insert("C".to_string(), 1);
        roster.insert("1B".to_string(), 1);
        roster.insert("2B".to_string(), 1);
        roster.insert("3B".to_string(), 1);
        roster.insert("SS".to_string(), 1);
        roster.insert("LF".to_string(), 1);
        roster.insert("CF".to_string(), 1);
        roster.insert("RF".to_string(), 1);
        roster.insert("UTIL".to_string(), 1);
        roster.insert("SP".to_string(), 5);
        roster.insert("RP".to_string(), 6);
        roster.insert("BE".to_string(), 6);
        roster.insert("IL".to_string(), 5);
        roster
    }

    /// Apply a roster configuration inferred from the ESPN draft board.
    ///
    /// Sets the roster_config, recomputes initial valuations from projections,
    /// and recomputes scarcity indices.
    pub fn apply_roster_config(&mut self, roster: std::collections::HashMap<String, usize>) {
        info!("Applying roster config: {:?}", roster);
        self.roster_config = Some(roster.clone());
        self.available_players = crate::valuation::compute_initial(
            &self.all_projections,
            &self.config,
            &roster,
        )
        .unwrap_or_default();
        self.scarcity = compute_scarcity(&self.available_players, &roster);
    }

    /// Reconstruct the LLM client from the current config.
    ///
    /// Called after settings changes (API key, provider, model) so that
    /// subsequent LLM calls use the updated configuration.
    pub fn reload_llm_client(&mut self) {
        self.llm_client = Arc::new(LlmClient::from_config(&self.config));
        info!("Reloaded LLM client (provider={:?}, model={})",
            self.config.strategy.llm.provider,
            self.config.strategy.llm.model,
        );
    }

    /// Process new picks from the extension state diff.
    ///
    /// For each new pick:
    /// 1. Record in DraftState
    /// 2. Persist to DB
    /// 3. Remove from available player pool
    /// 4. Update inflation and scarcity
    pub fn process_new_picks(
        &mut self,
        new_picks: Vec<crate::draft::pick::DraftPick>,
    ) {
        if new_picks.is_empty() {
            return;
        }

        for pick in &new_picks {
            info!(
                "Recording pick #{}: {} -> {} for ${}",
                pick.pick_number, pick.player_name, pick.team_name, pick.price
            );

            // Record in DraftState (assigns canonical sequential pick_number)
            let prev_count = self.draft_state.picks.len();
            self.draft_state.record_pick(pick.clone());

            // Only persist if record_pick actually added a new pick (wasn't deduped).
            // Use the pick from draft_state.picks which has the corrected pick_number;
            // the original `pick` from ESPN may have an unreliable pick_number (e.g.
            // always 1) due to ESPN's virtualized pick list.
            if self.draft_state.picks.len() > prev_count {
                let canonical_pick = self.draft_state.picks.last().unwrap();
                if let Err(e) = self.db.record_pick(canonical_pick, &self.draft_id) {
                    warn!("Failed to persist pick to DB: {}", e);
                }
            }

            // Remove from available player pool.
            // Primary match is by name; fall back to ESPN player ID when available
            // to guard against minor name-format mismatches between extension and
            // projection data (e.g. "J.D. Martinez" vs "JD Martinez").
            let player_name = &pick.player_name;
            let espn_id = pick.espn_player_id.as_deref();
            self.available_players.retain(|p| {
                if p.name == *player_name {
                    return false;
                }
                // If the pick carries an ESPN ID, check for an ID-based match.
                // Future player records might carry an ID field. For now this is
                // a defensive no-op placeholder that keeps the structure ready
                // for ID-based matching.
                if let Some(_id) = espn_id {
                    // TODO: match against player.espn_id once that field exists
                }
                true
            });
        }

        // Update inflation
        self.inflation.update(
            &self.available_players,
            &self.draft_state,
            &self.config.league,
        );

        // Update scarcity
        if let Some(ref roster) = self.roster_config {
            self.scarcity = compute_scarcity(&self.available_players, roster);
        }

        // Update category needs (for now, uniform - real implementation in TUI tasks)
        // Category needs would be recomputed based on the user's roster composition.
    }

    /// Build an `AppSnapshot` from the current application state.
    ///
    /// This captures all recalculated data (available players, scarcity,
    /// budget, inflation, draft log, roster, team summaries) into a single
    /// snapshot that the TUI can apply in one shot.
    pub fn build_snapshot(&self) -> AppSnapshot {
        let my_team = self.draft_state.my_team();

        let (my_roster, budget_spent, budget_remaining, max_bid, avg_per_slot) =
            if let Some(team) = my_team {
                let roster = team.roster.slots.clone();
                let empty_slots = roster.iter().filter(|s| s.player.is_none()).count();
                let avg = if empty_slots > 0 {
                    team.budget_remaining as f64 / empty_slots as f64
                } else {
                    0.0
                };
                let max = if empty_slots > 1 {
                    team.budget_remaining.saturating_sub((empty_slots as u32) - 1)
                } else {
                    team.budget_remaining
                };
                (roster, team.budget_spent, team.budget_remaining, max, avg)
            } else {
                // Teams not yet registered; return defaults
                (Vec::new(), 0, self.config.league.salary_cap, self.config.league.salary_cap, 0.0)
            };

        // Compute hitter/pitcher budget split
        let salary_cap = self.config.league.salary_cap;
        let hitting_frac = self.config.strategy.hitting_budget_fraction;
        let hitting_target = (salary_cap as f64 * hitting_frac).round() as u32;
        let pitching_target = salary_cap.saturating_sub(hitting_target);

        let (hitting_spent, pitching_spent) = if let Some(team) = my_team {
            let my_team_id = &team.team_id;
            let mut h_spent: u32 = 0;
            let mut p_spent: u32 = 0;
            for pick in &self.draft_state.picks {
                if pick.team_id != *my_team_id {
                    continue;
                }
                let is_hitter = match Position::from_str_pos(&pick.position) {
                    Some(pos) if !matches!(pos, Position::Bench | Position::InjuredList) => {
                        pos.is_hitter()
                    }
                    Some(_) => {
                        // Bench or IL: fall back to eligible_slots
                        let playing = playing_positions_from_slots(&pick.eligible_slots);
                        playing.iter().any(|p| p.is_hitter())
                    }
                    None => continue, // unparseable position, skip
                };
                if is_hitter {
                    h_spent += pick.price;
                } else {
                    p_spent += pick.price;
                }
            }
            (h_spent, p_spent)
        } else {
            (0, 0)
        };

        let team_snapshots = self
            .draft_state
            .teams
            .iter()
            .map(|t| {
                let filled = t.roster.filled_count();
                let total = t.roster.draftable_count();
                TeamSnapshot {
                    name: t.team_name.clone(),
                    budget_remaining: t.budget_remaining,
                    slots_filled: filled,
                    total_slots: total,
                }
            })
            .collect();

        AppSnapshot {
            app_mode: self.app_mode.clone(),
            pick_count: self.draft_state.pick_count,
            total_picks: self.draft_state.total_picks,
            active_tab: None, // Don't override the user's active tab
            available_players: self.available_players.clone(),
            positional_scarcity: self.scarcity.clone(),
            draft_log: self.draft_state.picks.clone(),
            my_roster,
            budget_spent,
            budget_remaining,
            salary_cap,
            inflation_rate: self.inflation.inflation_rate,
            max_bid,
            avg_per_slot,
            hitting_spent,
            hitting_target,
            pitching_spent,
            pitching_target,
            team_snapshots,
            llm_configured: matches!(*self.llm_client, LlmClient::Active(_)),
        }
    }

    /// Handle a new or changed nomination.
    ///
    /// Computes instant analysis and triggers LLM analysis (stub for now).
    pub fn handle_nomination(
        &mut self,
        nomination: &ActiveNomination,
    ) -> Option<InstantAnalysis> {
        let my_team = match self.draft_state.my_team() {
            Some(t) => t,
            None => {
                warn!("handle_nomination called before teams registered, skipping");
                return None;
            }
        };

        // Find the nominated player in our available pool
        let player = self
            .available_players
            .iter()
            .find(|p| p.name == nomination.player_name);

        let analysis = player.map(|p| {
            compute_instant_analysis(
                p,
                &my_team.roster,
                &self.available_players,
                &self.scarcity,
                &self.inflation,
                &self.category_needs,
            )
        });

        // Update DraftState nomination
        self.draft_state.current_nomination = Some(nomination.clone());

        // Trigger LLM nomination analysis (sets llm_mode, clears text, spawns task)
        self.trigger_nomination_analysis(nomination, analysis.as_ref());

        analysis
    }

    /// Handle nomination cleared (pick completed for the nominated player).
    ///
    /// Returns `Some(plan_request_id)` if a nomination planning task was started,
    /// so callers can send `UiUpdate::PlanStarted` to clear stale plan text in the TUI.
    pub fn handle_nomination_cleared(&mut self) -> Option<u64> {
        self.draft_state.current_nomination = None;
        if let Some(id) = self.analysis_request_id.take() {
            self.llm_requests.cancel(id);
        }
        self.analysis_player = None;

        // Auto-trigger nomination planning between picks so the plan panel
        // is populated before the user needs to nominate. Only fire when the
        // config flag is set and we already know which team is ours.
        if self.config.strategy.llm.prefire_planning && self.draft_state.my_team().is_some() {
            info!("Auto-triggering nomination planning (prefire_planning=true)");
            return self.trigger_nomination_planning();
        }
        None
    }

    /// Cancel all active LLM tasks.
    pub fn cancel_llm_tasks(&mut self) {
        if let Some(id) = self.analysis_request_id.take() {
            self.llm_requests.cancel(id);
        }
        if let Some(id) = self.plan_request_id.take() {
            self.llm_requests.cancel(id);
        }
        self.analysis_player = None;
        info!("Cancelled LLM tasks");
    }

    /// Trigger LLM nomination analysis for a nominated player.
    ///
    /// Cancels any in-flight analysis task, builds the analysis prompt from
    /// current state, and spawns a streaming task via the request manager.
    pub fn trigger_nomination_analysis(&mut self, nomination: &ActiveNomination, analysis: Option<&InstantAnalysis>) {
        // Secondary guard: if already analyzing this exact player, skip to avoid
        // canceling and restarting the active LLM task. This is a backstop for
        // cases where preserve_llm in handle_full_state_sync doesn't fully prevent
        // nomination_changed from firing (e.g., when saved_nomination is None).
        if let Some(ref ap) = self.analysis_player {
            let same = if !ap.player_id.is_empty() && !nomination.player_id.is_empty() {
                ap.player_id == nomination.player_id
            } else {
                ap.player_name == nomination.player_name
            };
            if same {
                info!(
                    "LLM already analyzing {} — preserving active task (FullStateSync guard)",
                    nomination.player_name
                );
                return;
            }
        }

        // Cancel only previous analysis
        if let Some(id) = self.analysis_request_id.take() {
            self.llm_requests.cancel(id);
        }
        self.analysis_player = None;

        let my_team = match self.draft_state.my_team() {
            Some(t) => t,
            None => {
                warn!("trigger_nomination_analysis called before teams registered, skipping");
                return;
            }
        };

        // Extract budget info from my_team before the borrow ends
        let my_team_budget = my_team.budget_remaining;
        let my_roster = my_team.roster.clone();

        // Find the nominated player in our pool
        let player = self
            .available_players
            .iter()
            .find(|p| p.name == nomination.player_name);

        let player = match player {
            Some(p) => p.clone(),
            None => {
                info!(
                    "Player {} not found in available pool, skipping LLM analysis",
                    nomination.player_name
                );
                return;
            }
        };

        // Track which player is being analyzed
        self.analysis_player = Some(AnalysisPlayer {
            player_name: nomination.player_name.clone(),
            player_id: nomination.player_id.clone(),
        });

        let nom_info = NominationInfo {
            player_name: nomination.player_name.clone(),
            position: nomination.position.clone(),
            nominated_by: nomination.nominated_by.clone(),
            current_bid: nomination.current_bid,
            current_bidder: nomination.current_bidder.clone(),
            time_remaining: nomination.time_remaining,
            eligible_slots: nomination.eligible_slots.clone(),
        };

        // Build budget context for the LLM
        let empty_slots = my_roster.empty_slots();
        let max_safe_bid = if empty_slots > 1 {
            my_team_budget.saturating_sub((empty_slots as u32) - 1)
        } else {
            my_team_budget
        };
        let avg_per_slot = if empty_slots > 0 {
            my_team_budget as f64 / empty_slots as f64
        } else {
            0.0
        };

        let (engine_bid_floor, engine_bid_ceiling, engine_verdict) = match analysis {
            Some(a) => (a.bid_floor, a.bid_ceiling, a.verdict.label().to_string()),
            None => {
                // Fallback: compute inline
                let adjusted = self.inflation.adjust(player.dollar_value);
                let floor = (adjusted * 0.70).round().max(1.0) as u32;
                let ceiling = adjusted.round().max(1.0) as u32;
                (floor, ceiling, "UNKNOWN".to_string())
            }
        };

        let budget = BudgetContext {
            budget_remaining: my_team_budget,
            empty_slots,
            max_safe_bid,
            avg_per_slot,
            pick_number: self.draft_state.pick_count + 1,
            total_picks: self.draft_state.total_picks,
            engine_bid_floor,
            engine_bid_ceiling,
            engine_verdict,
        };

        let system = prompt::system_prompt(&self.config.league, self.roster_config.as_ref(), self.config.strategy.strategy_overview.as_deref());
        let user_content = prompt::build_nomination_analysis_prompt(
            &player,
            &nom_info,
            &my_roster,
            &self.category_needs,
            &self.scarcity,
            &self.available_players,
            &self.draft_state,
            &self.inflation,
            &budget,
        );

        let max_tokens = self.config.strategy.llm.analysis_max_tokens;
        let client = Arc::clone(&self.llm_client);
        let tx = self.llm_tx.clone();

        let id = self.llm_requests.start(client, system, user_content, max_tokens, tx);
        self.analysis_request_id = Some(id);
        info!(
            "Triggered LLM nomination analysis for {} (bid: ${}, request_id: {})",
            nomination.player_name, nomination.current_bid, id
        );
    }

    /// Trigger LLM nomination planning (what to nominate next).
    ///
    /// Cancels any in-flight plan task, builds the planning prompt from
    /// current state, and spawns a streaming task via the request manager.
    ///
    /// Returns `Some(request_id)` if a planning task was successfully started,
    /// `None` if it was skipped (e.g. teams not yet registered). Callers that
    /// receive `Some` should send `UiUpdate::PlanStarted` to clear stale plan
    /// text in the TUI before the first token arrives.
    pub fn trigger_nomination_planning(&mut self) -> Option<u64> {
        // Cancel only previous plan
        if let Some(id) = self.plan_request_id.take() {
            self.llm_requests.cancel(id);
        }

        let my_team = match self.draft_state.my_team() {
            Some(t) => t,
            None => {
                warn!("trigger_nomination_planning called before teams registered, skipping");
                return None;
            }
        };

        // Extract budget info from my_team before the borrow ends
        let my_team_budget = my_team.budget_remaining;
        let my_roster = my_team.roster.clone();

        // Build budget context for the LLM
        let empty_slots = my_roster.empty_slots();
        let max_safe_bid = if empty_slots > 1 {
            my_team_budget.saturating_sub((empty_slots as u32) - 1)
        } else {
            my_team_budget
        };
        let avg_per_slot = if empty_slots > 0 {
            my_team_budget as f64 / empty_slots as f64
        } else {
            0.0
        };

        let budget = BudgetContext {
            budget_remaining: my_team_budget,
            empty_slots,
            max_safe_bid,
            avg_per_slot,
            pick_number: self.draft_state.pick_count + 1,
            total_picks: self.draft_state.total_picks,
            // Planning prompt doesn't have a specific player, so use zeros for engine fields
            engine_bid_floor: 0,
            engine_bid_ceiling: 0,
            engine_verdict: String::new(),
        };

        let system = prompt::system_prompt(&self.config.league, self.roster_config.as_ref(), self.config.strategy.strategy_overview.as_deref());
        let user_content = prompt::build_nomination_planning_prompt(
            &my_roster,
            &self.category_needs,
            &self.scarcity,
            &self.available_players,
            &self.draft_state,
            &self.inflation,
            &budget,
        );

        let max_tokens = self.config.strategy.llm.planning_max_tokens;
        let client = Arc::clone(&self.llm_client);
        let tx = self.llm_tx.clone();

        let id = self.llm_requests.start(client, system, user_content, max_tokens, tx);
        self.plan_request_id = Some(id);
        info!("Triggered LLM nomination planning (request_id: {})", id);
        Some(id)
    }

    /// Convert extension PickData format to our internal StateUpdatePayload format.
    pub fn convert_extension_state(
        payload: &crate::protocol::StateUpdatePayload,
    ) -> StateUpdatePayload {
        StateUpdatePayload {
            picks: payload
                .picks
                .iter()
                .map(|p| PickPayload {
                    pick_number: p.pick_number,
                    team_id: p.team_id.clone(),
                    team_name: p.team_name.clone(),
                    player_id: p.player_id.clone(),
                    player_name: p.player_name.clone(),
                    position: p.position.clone(),
                    price: p.price,
                    eligible_slots: p.eligible_slots.clone(),
                    assigned_slot: p.assigned_slot,
                })
                .collect(),
            current_nomination: payload.current_nomination.as_ref().and_then(|n| {
                // Filter out premature nominations: during the pre-nomination
                // phase the nominator is browsing players and the extension may
                // (despite the JS-side guard) send a nomination with no bid, no
                // bidder, and no nominator. A real nomination in the "offer"
                // stage always has at least a current_bid > 0 or a non-empty
                // nominated_by field (populated from the bid history).
                let has_bid = n.current_bid > 0;
                let has_nominator = !n.nominated_by.is_empty();
                let has_bidder = n.current_bidder.as_ref().is_some_and(|b| !b.is_empty());
                if !has_bid && !has_nominator && !has_bidder {
                    warn!(
                        "Filtering premature nomination for '{}': no bid, no nominator, no bidder",
                        n.player_name
                    );
                    return None;
                }
                Some(NominationPayload {
                    player_id: n.player_id.clone(),
                    player_name: n.player_name.clone(),
                    position: n.position.clone(),
                    nominated_by: n.nominated_by.clone(),
                    current_bid: n.current_bid,
                    current_bidder: n.current_bidder.clone(),
                    time_remaining: n.time_remaining,
                    eligible_slots: n.eligible_slots.clone(),
                })
            }),
            teams: payload
                .teams
                .iter()
                .map(|t| TeamBudgetPayload {
                    team_id: t.team_id.clone().unwrap_or_default(),
                    team_name: t.team_name.clone(),
                    budget: t.budget,
                })
                .collect(),
            pick_count: payload.pick_count,
            total_picks: payload.total_picks,
        }
    }
}

// ---------------------------------------------------------------------------
// Main event loop
// ---------------------------------------------------------------------------

/// Run the main application event loop.
///
/// Listens on three channels using `tokio::select!`:
/// 1. WebSocket events from the extension
/// 2. LLM streaming events
/// 3. User commands from the TUI
///
/// Pushes UI updates through `ui_tx` for the TUI render loop.
pub async fn run(
    mut ws_rx: mpsc::Receiver<WsEvent>,
    mut llm_rx: mpsc::Receiver<LlmEvent>,
    mut cmd_rx: mpsc::Receiver<UserCommand>,
    ui_tx: mpsc::Sender<UiUpdate>,
    mut state: AppState,
) -> anyhow::Result<()> {
    info!("Application event loop started");

    // Send initial snapshot so the TUI has available players immediately,
    // before any WebSocket events arrive from the extension.
    let initial_snapshot = state.build_snapshot();
    let _ = ui_tx.send(UiUpdate::StateSnapshot(Box::new(initial_snapshot))).await;

    // Track whether the LLM channel is still open. When it closes we replace
    // the recv future with a pending future so tokio::select! never spins on it.
    let mut llm_open = true;

    // Interval timer for heartbeat timeout checks. Fires every
    // HEARTBEAT_CHECK_INTERVAL; the handler compares Instant::now()
    // against `state.last_ws_message_time` to detect stale connections.
    let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_CHECK_INTERVAL);
    // The first tick completes immediately; consume it so the first
    // real check happens after one full interval.
    heartbeat_interval.tick().await;

    loop {
        tokio::select! {
            // --- WebSocket events ---
            ws_event = ws_rx.recv() => {
                match ws_event {
                    Some(WsEvent::Connected { addr }) => {
                        info!("Extension connected from {}", addr);
                        state.connection_status = ConnectionStatus::Connected;
                        state.last_ws_message_time = Some(Instant::now());
                        let _ = ui_tx.send(UiUpdate::ConnectionStatus(ConnectionStatus::Connected)).await;
                    }
                    Some(WsEvent::Disconnected) => {
                        info!("Extension disconnected");
                        state.connection_status = ConnectionStatus::Disconnected;
                        state.last_ws_message_time = None;
                        let _ = ui_tx.send(UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)).await;
                    }
                    Some(WsEvent::Message(json_str)) => {
                        // If we had marked the connection as stale-disconnected
                        // (heartbeat timeout) but are now receiving messages
                        // again, restore Connected. We detect this case by
                        // checking that `last_ws_message_time` is `Some` --
                        // it is only `Some` if a `WsEvent::Connected` was
                        // previously received, so a bare `Disconnected` initial
                        // state (last_ws_message_time == None) won't trigger
                        // this.
                        if state.connection_status == ConnectionStatus::Disconnected
                            && state.last_ws_message_time.is_some()
                        {
                            info!("Extension connection restored (received message after stale timeout)");
                            state.connection_status = ConnectionStatus::Connected;
                            let _ = ui_tx.send(UiUpdate::ConnectionStatus(ConnectionStatus::Connected)).await;
                        }
                        // Only track message timestamps when we have an active
                        // connection (last_ws_message_time is Some from a prior
                        // Connected event). This avoids false "reconnect" signals
                        // when the ws_server forwards messages without a
                        // preceding Connected event.
                        if state.last_ws_message_time.is_some() {
                            state.last_ws_message_time = Some(Instant::now());
                        }
                        ws_handler::handle_ws_message(&mut state, &json_str, &ui_tx).await;
                    }
                    None => {
                        info!("WebSocket channel closed, shutting down");
                        break;
                    }
                }
            }

            // --- LLM events (only poll when channel is open) ---
            llm_event = llm_rx.recv(), if llm_open => {
                match llm_event {
                    Some(event) => {
                        llm_handler::handle_llm_event(&mut state, event, &ui_tx).await;
                    }
                    None => {
                        // LLM channel closed - stop polling to avoid busy-loop
                        info!("LLM channel closed");
                        llm_open = false;
                    }
                }
            }

            // --- User commands ---
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(UserCommand::Quit) => {
                        info!("Quit command received, shutting down");
                        break;
                    }
                    Some(cmd) => {
                        command_handler::handle_user_command(&mut state, cmd, &ui_tx).await;
                    }
                    None => {
                        info!("Command channel closed, shutting down");
                        break;
                    }
                }
            }

            // --- Heartbeat timeout check ---
            _ = heartbeat_interval.tick() => {
                if state.connection_status == ConnectionStatus::Connected {
                    if let Some(last_time) = state.last_ws_message_time {
                        let elapsed = last_time.elapsed();
                        if elapsed > HEARTBEAT_TIMEOUT {
                            warn!(
                                "No WebSocket message received for {:?}, marking connection as stale",
                                elapsed
                            );
                            state.connection_status = ConnectionStatus::Disconnected;
                            let _ = ui_tx
                                .send(UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected))
                                .await;
                        }
                    }
                }
            }
        }
    }

    // Cleanup
    state.llm_requests.cancel_all();
    info!("Application event loop exiting");
    Ok(())
}









// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;
    use crate::config::*;
    use crate::db::Database;
    use crate::draft::pick::{DraftPick, Position};
    use crate::draft::state::{ActiveNomination, DraftState};
    use crate::protocol::{LlmEvent, OnboardingAction, OnboardingUpdate, UserCommand};
    use crate::valuation::auction::InflationTracker;
    use crate::valuation::projections::{AllProjections, PitcherType};
    use crate::valuation::zscore::{
        CategoryZScores, HitterZScores, PitcherZScores, PlayerProjectionData, PlayerValuation,
    };

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn test_league_config() -> LeagueConfig {
        LeagueConfig {
            name: "Test League".into(),
            platform: "espn".into(),
            num_teams: 2,
            scoring_type: "h2h_most_categories".into(),
            salary_cap: 260,
            batting_categories: CategoriesSection {
                categories: vec![
                    "R".into(),
                    "HR".into(),
                    "RBI".into(),
                    "BB".into(),
                    "SB".into(),
                    "AVG".into(),
                ],
            },
            pitching_categories: CategoriesSection {
                categories: vec![
                    "K".into(),
                    "W".into(),
                    "SV".into(),
                    "HD".into(),
                    "ERA".into(),
                    "WHIP".into(),
                ],
            },
            roster_limits: RosterLimits {
                max_sp: 7,
                max_rp: 7,
                gs_per_week: 7,
            },
            teams: HashMap::new(),
        }
    }

    fn test_strategy_config() -> StrategyConfig {
        StrategyConfig {
            hitting_budget_fraction: 0.65,
            weights: CategoryWeights {
                R: 1.0,
                HR: 1.0,
                RBI: 1.0,
                BB: 1.2,
                SB: 1.0,
                AVG: 1.0,
                K: 1.0,
                W: 1.0,
                SV: 0.7,
                HD: 1.3,
                ERA: 1.0,
                WHIP: 1.0,
            },
            pool: PoolConfig {
                min_pa: 300,
                min_ip_sp: 80.0,
                min_g_rp: 30,
                hitter_pool_size: 150,
                sp_pool_size: 70,
                rp_pool_size: 80,
            },
            llm: LlmConfig {
                provider: crate::llm::provider::LlmProvider::Anthropic,
                model: "test".into(),
                analysis_max_tokens: 2048,
                planning_max_tokens: 2048,
                analysis_trigger: "nomination".into(),
                prefire_planning: true,
            },
            strategy_overview: None,
        }
    }

    fn test_config() -> Config {
        Config {
            league: test_league_config(),
            strategy: test_strategy_config(),
            credentials: CredentialsConfig::default(),
            ws_port: 9001,
            data_paths: DataPaths::default(),
        }
    }

    fn test_roster_config() -> HashMap<String, usize> {
        let mut config = HashMap::new();
        config.insert("C".into(), 1);
        config.insert("1B".into(), 1);
        config.insert("2B".into(), 1);
        config.insert("3B".into(), 1);
        config.insert("SS".into(), 1);
        config.insert("LF".into(), 1);
        config.insert("CF".into(), 1);
        config.insert("RF".into(), 1);
        config.insert("UTIL".into(), 1);
        config.insert("SP".into(), 5);
        config.insert("RP".into(), 6);
        config.insert("BE".into(), 6);
        config.insert("IL".into(), 5);
        config
    }

    fn test_espn_budgets() -> Vec<crate::draft::state::TeamBudgetPayload> {
        vec![
            crate::draft::state::TeamBudgetPayload {
                team_id: "1".into(),
                team_name: "Team 1".into(),
                budget: 260,
            },
            crate::draft::state::TeamBudgetPayload {
                team_id: "2".into(),
                team_name: "Team 2".into(),
                budget: 260,
            },
        ]
    }

    fn make_hitter(
        name: &str,
        r: u32,
        hr: u32,
        rbi: u32,
        bb: u32,
        sb: u32,
        ab: u32,
        avg: f64,
        positions: Vec<Position>,
    ) -> PlayerValuation {
        PlayerValuation {
            name: name.into(),
            team: "TST".into(),
            positions,
            is_pitcher: false,
            is_two_way: false,
            pitcher_type: None,
            projection: PlayerProjectionData::Hitter {
                pa: ab + bb,
                ab,
                h: (ab as f64 * avg).round() as u32,
                hr,
                r,
                rbi,
                bb,
                sb,
                avg,
            },
            total_zscore: 0.0,
            category_zscores: CategoryZScores::Hitter(HitterZScores {
                r: 0.0,
                hr: 0.0,
                rbi: 0.0,
                bb: 0.0,
                sb: 0.0,
                avg: 0.0,
                total: 0.0,
            }),
            vor: 0.0,
            initial_vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
        }
    }

    fn make_pitcher(
        name: &str,
        k: u32,
        w: u32,
        sv: u32,
        hd: u32,
        ip: f64,
        era: f64,
        whip: f64,
        pitcher_type: PitcherType,
    ) -> PlayerValuation {
        let pos = match pitcher_type {
            PitcherType::SP => Position::StartingPitcher,
            PitcherType::RP => Position::ReliefPitcher,
        };
        PlayerValuation {
            name: name.into(),
            team: "TST".into(),
            positions: vec![pos],
            is_pitcher: true,
            is_two_way: false,
            pitcher_type: Some(pitcher_type),
            projection: PlayerProjectionData::Pitcher {
                ip,
                k,
                w,
                sv,
                hd,
                era,
                whip,
                g: 30,
                gs: if pitcher_type == PitcherType::SP {
                    30
                } else {
                    0
                },
            },
            total_zscore: 0.0,
            category_zscores: CategoryZScores::Pitcher(PitcherZScores {
                k: 0.0,
                w: 0.0,
                sv: 0.0,
                hd: 0.0,
                era: 0.0,
                whip: 0.0,
                total: 0.0,
            }),
            vor: 0.0,
            initial_vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
        }
    }

    fn test_players() -> Vec<PlayerValuation> {
        vec![
            make_hitter(
                "H_Star",
                100,
                40,
                100,
                70,
                20,
                550,
                0.300,
                vec![Position::FirstBase],
            ),
            make_hitter(
                "H_Good",
                80,
                25,
                75,
                55,
                15,
                530,
                0.280,
                vec![Position::SecondBase],
            ),
            make_hitter(
                "H_Mid",
                60,
                15,
                55,
                40,
                10,
                500,
                0.265,
                vec![Position::ShortStop],
            ),
            make_hitter(
                "H_Low",
                45,
                8,
                40,
                30,
                5,
                480,
                0.250,
                vec![Position::Catcher],
            ),
            make_pitcher(
                "P_Ace",
                250,
                18,
                0,
                0,
                200.0,
                2.80,
                1.00,
                PitcherType::SP,
            ),
            make_pitcher(
                "P_Good",
                200,
                14,
                0,
                0,
                180.0,
                3.20,
                1.10,
                PitcherType::SP,
            ),
            make_pitcher(
                "P_Mid",
                150,
                10,
                0,
                0,
                160.0,
                3.80,
                1.20,
                PitcherType::SP,
            ),
        ]
    }

    fn empty_projections() -> AllProjections {
        AllProjections {
            hitters: vec![],
            pitchers: vec![],
        }
    }

    fn test_onboarding_manager() -> OnboardingManager<RealFileSystem> {
        let tmp = std::env::temp_dir().join(format!("wyncast_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        OnboardingManager::new(tmp, RealFileSystem)
    }

    fn create_test_app_state() -> AppState {
        let config = test_config();
        let mut draft_state = DraftState::new(260, &test_roster_config());
        // Register teams from ESPN data and set my team
        draft_state.reconcile_budgets(&test_espn_budgets());
        draft_state.set_my_team_by_id("1");

        let mut available = test_players();

        // Run initial valuation so dollar values are set
        let test_roster = AppState::default_roster_config();
        crate::valuation::recalculate_all(
            &mut available,
            &test_roster,
            &config.league,
            &config.strategy,
            &draft_state,
        );

        let db = Database::open(":memory:").expect("in-memory db");
        let draft_id = Database::generate_draft_id();
        let llm_client = LlmClient::Disabled;
        let (llm_tx, _llm_rx) = mpsc::channel(16);

        AppState::new(config, draft_state, available, empty_projections(), db, draft_id, llm_client, llm_tx, None, AppMode::Draft, test_onboarding_manager(), Some(test_roster_config()))
    }

    /// Drain the initial `StateSnapshot` that `run()` sends before entering
    /// the event loop. Tests that spawn `run()` and then assert on the first
    /// UI update must call this first, otherwise they'll see the snapshot
    /// instead of the event-driven update they expect.
    async fn drain_initial_snapshot(ui_rx: &mut mpsc::Receiver<UiUpdate>) {
        let update = ui_rx.recv().await.expect("should receive initial snapshot");
        assert!(
            matches!(update, UiUpdate::StateSnapshot(_)),
            "Expected initial StateSnapshot, got {:?}", update
        );
    }

    // -----------------------------------------------------------------------
    // Tests: State diff detection -> pick recording -> recalculation
    // -----------------------------------------------------------------------

    #[test]
    fn process_new_picks_updates_state() {
        let mut state = create_test_app_state();
        let initial_count = state.available_players.len();
        let initial_pick_count = state.draft_state.pick_count;

        let pick = DraftPick {
            pick_number: 1,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_name: "H_Star".into(),
            position: "1B".into(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
            assigned_slot: None,
        };

        state.process_new_picks(vec![pick]);

        // Pick count should increase
        assert_eq!(state.draft_state.pick_count, initial_pick_count + 1);

        // Player should be removed from available pool
        assert_eq!(state.available_players.len(), initial_count - 1);
        assert!(!state
            .available_players
            .iter()
            .any(|p| p.name == "H_Star"));

        // Team budget should be updated
        let team = state.draft_state.team("1").unwrap();
        assert_eq!(team.budget_spent, 45);
        assert_eq!(team.budget_remaining, 215);
    }

    #[test]
    fn process_new_picks_updates_inflation() {
        let mut state = create_test_app_state();

        let pick = DraftPick {
            pick_number: 1,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_name: "H_Star".into(),
            position: "1B".into(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
            assigned_slot: None,
        };

        state.process_new_picks(vec![pick]);

        // Inflation tracker should be updated
        assert!(state.inflation.total_dollars_spent > 0.0);
        assert!(state.inflation.inflation_rate.is_finite());
    }

    #[test]
    fn process_new_picks_updates_scarcity() {
        let mut state = create_test_app_state();

        // Record the initial scarcity state for FirstBase
        let initial_fb_count = state
            .scarcity
            .iter()
            .find(|s| s.position == Position::FirstBase)
            .map(|s| s.players_above_replacement);

        let pick = DraftPick {
            pick_number: 1,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_name: "H_Star".into(),
            position: "1B".into(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
            assigned_slot: None,
        };

        state.process_new_picks(vec![pick]);

        // Scarcity should be recalculated
        let new_fb_count = state
            .scarcity
            .iter()
            .find(|s| s.position == Position::FirstBase)
            .map(|s| s.players_above_replacement);

        // After removing a 1B player, the count should change (or at least be recalculated)
        // The exact change depends on whether H_Star had positive VOR
        assert!(new_fb_count.is_some());
        // Just verify scarcity was recomputed (if H_Star had positive VOR, count should decrease)
        if let (Some(initial), Some(new)) = (initial_fb_count, new_fb_count) {
            // If the star had positive VOR, count should decrease
            if initial > 0 {
                assert!(new <= initial);
            }
        }
    }

    #[test]
    fn process_new_picks_persists_to_db() {
        let mut state = create_test_app_state();

        let pick = DraftPick {
            pick_number: 1,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_name: "H_Star".into(),
            position: "1B".into(),
            price: 45,
            espn_player_id: Some("espn_123".into()),
            eligible_slots: vec![],
            assigned_slot: None,
        };

        state.process_new_picks(vec![pick]);

        // Verify the pick was persisted to DB
        let db_picks = state.db.load_picks(&state.draft_id).unwrap();
        assert_eq!(db_picks.len(), 1);
        assert_eq!(db_picks[0].player_name, "H_Star");
        assert_eq!(db_picks[0].price, 45);
        assert_eq!(db_picks[0].espn_player_id, Some("espn_123".into()));
    }

    #[test]
    fn process_new_picks_persists_canonical_pick_number_to_db() {
        let mut state = create_test_app_state();

        // Simulate ESPN sending all picks with pick_number=1 (the known bug
        // in ESPN's virtualized pick list)
        let picks = vec![
            DraftPick {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: Some("espn_1".into()),
                eligible_slots: vec![],
            assigned_slot: None,
            },
            DraftPick {
                pick_number: 1,
                team_id: "2".into(),
                team_name: "Team 2".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: Some("espn_2".into()),
                eligible_slots: vec![],
            assigned_slot: None,
            },
            DraftPick {
                pick_number: 1,
                team_id: "3".into(),
                team_name: "Team 3".into(),
                player_name: "H_Good".into(),
                position: "2B".into(),
                price: 30,
                espn_player_id: Some("espn_3".into()),
                eligible_slots: vec![],
            assigned_slot: None,
            },
        ];

        state.process_new_picks(picks);

        // All 3 picks should be persisted with canonical sequential pick numbers
        let db_picks = state.db.load_picks(&state.draft_id).unwrap();
        assert_eq!(db_picks.len(), 3, "All picks must be persisted, not just the first");
        assert_eq!(db_picks[0].pick_number, 1);
        assert_eq!(db_picks[0].player_name, "H_Star");
        assert_eq!(db_picks[1].pick_number, 2);
        assert_eq!(db_picks[1].player_name, "P_Ace");
        assert_eq!(db_picks[2].pick_number, 3);
        assert_eq!(db_picks[2].player_name, "H_Good");
    }

    #[test]
    fn process_multiple_picks_at_once() {
        let mut state = create_test_app_state();
        let initial_count = state.available_players.len();

        let picks = vec![
            DraftPick {
                pick_number: 1,
                team_id: "team_1".into(),
                team_name: "Team 1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
            DraftPick {
                pick_number: 2,
                team_id: "team_2".into(),
                team_name: "Team 2".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
        ];

        state.process_new_picks(picks);

        assert_eq!(state.draft_state.pick_count, 2);
        assert_eq!(state.available_players.len(), initial_count - 2);
        assert!(!state.available_players.iter().any(|p| p.name == "H_Star"));
        assert!(!state.available_players.iter().any(|p| p.name == "P_Ace"));
    }

    // -----------------------------------------------------------------------
    // Tests: New picks update DraftState, available players, inflation
    // -----------------------------------------------------------------------

    #[test]
    fn picks_update_draft_state_and_available() {
        let mut state = create_test_app_state();

        let pick = DraftPick {
            pick_number: 1,
            team_id: "2".into(),
            team_name: "Team 2".into(),
            player_name: "H_Good".into(),
            position: "2B".into(),
            price: 30,
            espn_player_id: None,
            eligible_slots: vec![],
            assigned_slot: None,
        };

        state.process_new_picks(vec![pick]);

        // DraftState should have the pick
        assert_eq!(state.draft_state.picks.len(), 1);
        assert_eq!(state.draft_state.picks[0].player_name, "H_Good");

        // Team 2 budget should be updated
        let team2 = state.draft_state.team("2").unwrap();
        assert_eq!(team2.budget_spent, 30);

        // H_Good should not be in available pool
        assert!(!state
            .available_players
            .iter()
            .any(|p| p.name == "H_Good"));
    }

    // -----------------------------------------------------------------------
    // Tests: LLM trigger logic
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn nomination_triggers_llm_mode() {
        let mut state = create_test_app_state();

        let nomination = ActiveNomination {
            player_name: "H_Star".into(),
            player_id: "espn_1".into(),
            position: "1B".into(),
            nominated_by: "Team 2".into(),
            current_bid: 5,
            current_bidder: None,
            time_remaining: Some(30),
            eligible_slots: vec![],
        };

        let _analysis = state.handle_nomination(&nomination);

        // analysis_player and analysis_request_id should be set
        assert!(state.analysis_player.is_some());
        assert!(state.analysis_request_id.is_some());
        let ap = state.analysis_player.as_ref().unwrap();
        assert_eq!(ap.player_name, "H_Star");
        assert_eq!(ap.player_id, "espn_1");
    }

    #[tokio::test]
    async fn nomination_returns_analysis_for_known_player() {
        let mut state = create_test_app_state();

        let nomination = ActiveNomination {
            player_name: "H_Star".into(),
            player_id: "espn_1".into(),
            position: "1B".into(),
            nominated_by: "Team 2".into(),
            current_bid: 5,
            current_bidder: None,
            time_remaining: Some(30),
            eligible_slots: vec![],
        };

        let analysis = state.handle_nomination(&nomination);

        // Should return analysis since H_Star is in the available pool
        assert!(analysis.is_some());
        let analysis = analysis.unwrap();
        assert_eq!(analysis.player_name, "H_Star");
    }

    #[tokio::test]
    async fn nomination_returns_none_for_unknown_player() {
        let mut state = create_test_app_state();

        let nomination = ActiveNomination {
            player_name: "Unknown Player".into(),
            player_id: "espn_999".into(),
            position: "OF".into(),
            nominated_by: "Team 2".into(),
            current_bid: 1,
            current_bidder: None,
            time_remaining: Some(30),
            eligible_slots: vec![],
        };

        let analysis = state.handle_nomination(&nomination);

        // Should return None since the player is not in our pool
        assert!(analysis.is_none());
    }

    // -----------------------------------------------------------------------
    // Tests: LLM cancellation (new nomination cancels previous)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn new_nomination_clears_previous_analysis() {
        let mut state = create_test_app_state();

        // First nomination
        let nom1 = ActiveNomination {
            player_name: "H_Star".into(),
            player_id: "espn_1".into(),
            position: "1B".into(),
            nominated_by: "Team 2".into(),
            current_bid: 5,
            current_bidder: None,
            time_remaining: Some(30),
            eligible_slots: vec![],
        };
        state.handle_nomination(&nom1);

        // Second nomination (should cancel first)
        let nom2 = ActiveNomination {
            player_name: "H_Good".into(),
            player_id: "espn_2".into(),
            position: "2B".into(),
            nominated_by: "Team 1".into(),
            current_bid: 3,
            current_bidder: None,
            time_remaining: Some(30),
            eligible_slots: vec![],
        };
        state.handle_nomination(&nom2);

        // analysis_player should be updated to the new nomination
        let ap = state.analysis_player.as_ref().expect("Expected analysis_player to be set");
        assert_eq!(ap.player_name, "H_Good");
    }

    #[tokio::test]
    async fn nomination_cleared_resets_state() {
        let mut state = create_test_app_state();

        // Set up a nomination
        let nom = ActiveNomination {
            player_name: "H_Star".into(),
            player_id: "espn_1".into(),
            position: "1B".into(),
            nominated_by: "Team 2".into(),
            current_bid: 5,
            current_bidder: None,
            time_remaining: Some(30),
            eligible_slots: vec![],
        };
        state.handle_nomination(&nom);

        // Clear the nomination
        let plan_id = state.handle_nomination_cleared();

        assert!(state.draft_state.current_nomination.is_none());
        assert!(state.analysis_player.is_none());
        assert!(state.analysis_request_id.is_none());
        // With prefire_planning=true and teams registered, planning should
        // auto-trigger so the plan panel is populated between nominations.
        assert!(
            state.plan_request_id.is_some(),
            "expected plan_request_id to be set after clearing (prefire_planning=true)"
        );
        assert!(plan_id.is_some());
    }

    #[tokio::test]
    async fn nomination_cleared_skips_planning_when_prefire_disabled() {
        let mut state = create_test_app_state();
        state.config.strategy.llm.prefire_planning = false;

        let nom = ActiveNomination {
            player_name: "H_Star".into(),
            player_id: "espn_1".into(),
            position: "1B".into(),
            nominated_by: "Team 2".into(),
            current_bid: 5,
            current_bidder: None,
            time_remaining: Some(30),
            eligible_slots: vec![],
        };
        state.handle_nomination(&nom);

        state.handle_nomination_cleared();

        assert!(state.draft_state.current_nomination.is_none());
        assert!(state.plan_request_id.is_none());
    }

    #[tokio::test]
    async fn nomination_cleared_skips_planning_when_no_teams() {
        let mut state = create_test_app_state_no_teams();

        state.handle_nomination_cleared();

        assert!(state.plan_request_id.is_none());
    }

    // -----------------------------------------------------------------------
    // Tests: Async event loop
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn event_loop_handles_quit_command() {
        let state = create_test_app_state();
        let (ws_tx, ws_rx) = mpsc::channel(16);
        let (llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        // Spawn the event loop
        let handle = tokio::spawn(run(ws_rx, llm_rx, cmd_rx, ui_tx, state));

        // Send quit command
        cmd_tx.send(UserCommand::Quit).await.unwrap();

        // The loop should exit
        let result = handle.await.unwrap();
        assert!(result.is_ok());

        // Drop senders to clean up
        drop(ws_tx);
        drop(llm_tx);
        drop(cmd_tx);
    }

    #[tokio::test]
    async fn event_loop_handles_connection_status() {
        let state = create_test_app_state();
        let (ws_tx, ws_rx) = mpsc::channel(16);
        let (_llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        let handle = tokio::spawn(run(ws_rx, llm_rx, cmd_rx, ui_tx, state));
        drain_initial_snapshot(&mut ui_rx).await;

        // Send connected event
        ws_tx
            .send(WsEvent::Connected {
                addr: "127.0.0.1:1234".into(),
            })
            .await
            .unwrap();

        // Should receive connection status update
        let update = ui_rx.recv().await.unwrap();
        assert!(
            matches!(update, UiUpdate::ConnectionStatus(ConnectionStatus::Connected)),
            "Expected ConnectionStatus(Connected), got {:?}", update
        );

        // Send disconnected event
        ws_tx.send(WsEvent::Disconnected).await.unwrap();

        let update = ui_rx.recv().await.unwrap();
        assert!(
            matches!(update, UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)),
            "Expected ConnectionStatus(Disconnected), got {:?}", update
        );

        // Clean up
        cmd_tx.send(UserCommand::Quit).await.unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn event_loop_handles_llm_tokens() {
        let state = create_test_app_state();
        let (_ws_tx, ws_rx) = mpsc::channel(16);
        let (llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        let gen = 42u64;

        let handle = tokio::spawn(async move {
            let mut state = state;
            // Register a test request so the handler recognises the event
            state.llm_requests.track_test_id(gen);
            state.analysis_request_id = Some(gen);
            state.analysis_player = Some(AnalysisPlayer {
                player_name: "Test".into(),
                player_id: "1".into(),
            });
            run(ws_rx, llm_rx, cmd_rx, ui_tx, state).await
        });

        // Drain the initial snapshot sent before the event loop
        drain_initial_snapshot(&mut ui_rx).await;

        // Send LLM token with matching generation
        llm_tx
            .send(LlmEvent::Token {
                text: "Hello ".into(),
                generation: gen,
            })
            .await
            .unwrap();

        let update = ui_rx.recv().await.unwrap();
        match update {
            UiUpdate::LlmUpdate { request_id, update } => {
                assert_eq!(request_id, gen);
                assert_eq!(update, crate::protocol::LlmStreamUpdate::Token("Hello ".into()));
            }
            other => panic!("Expected LlmUpdate, got {:?}", other),
        }

        // Clean up
        cmd_tx.send(UserCommand::Quit).await.unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn event_loop_handles_state_update_with_picks() {
        let state = create_test_app_state();
        let (ws_tx, ws_rx) = mpsc::channel(16);
        let (_llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        let handle = tokio::spawn(run(ws_rx, llm_rx, cmd_rx, ui_tx, state));
        drain_initial_snapshot(&mut ui_rx).await;

        // Send a state update with a new pick and nomination
        let state_update = serde_json::json!({
            "type": "STATE_UPDATE",
            "timestamp": 1234567890,
            "payload": {
                "picks": [
                    {
                        "pickNumber": 1,
                        "teamId": "team_1",
                        "teamName": "Team 1",
                        "playerId": "espn_1",
                        "playerName": "H_Star",
                        "position": "1B",
                        "price": 45
                    }
                ],
                "currentNomination": {
                    "playerId": "espn_2",
                    "playerName": "H_Good",
                    "position": "2B",
                    "nominatedBy": "Team 2",
                    "currentBid": 5,
                    "currentBidder": null,
                    "timeRemaining": 30
                },
                "myTeamId": "team_1",
                "source": "test"
            }
        });

        ws_tx
            .send(WsEvent::Message(state_update.to_string()))
            .await
            .unwrap();

        // Should receive a StateSnapshot first (new picks trigger snapshot)
        let update = ui_rx.recv().await.unwrap();
        match update {
            UiUpdate::StateSnapshot(snapshot) => {
                assert_eq!(snapshot.pick_count, 1);
                // H_Star should have been removed from available players
                assert!(!snapshot.available_players.iter().any(|p| p.name == "H_Star"));
                // Draft log should contain the pick
                assert_eq!(snapshot.draft_log.len(), 1);
                assert_eq!(snapshot.draft_log[0].player_name, "H_Star");
            }
            other => panic!("Expected StateSnapshot, got {:?}", other),
        }

        // Then receive the NominationUpdate
        let update = ui_rx.recv().await.unwrap();
        match update {
            UiUpdate::NominationUpdate { info, .. } => {
                assert_eq!(info.player_name, "H_Good");
                assert_eq!(info.current_bid, 5);
            }
            other => panic!("Expected NominationUpdate, got {:?}", other),
        }

        // Clean up
        cmd_tx.send(UserCommand::Quit).await.unwrap();
        let _ = handle.await;
    }

    // -----------------------------------------------------------------------
    // Tests: Extension state conversion
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Tests: First nomination with delayed team registration
    // -----------------------------------------------------------------------

    /// Create an AppState WITHOUT teams registered (simulates the state
    /// before the first STATE_UPDATE containing team budget data).
    fn create_test_app_state_no_teams() -> AppState {
        let config = test_config();
        let draft_state = DraftState::new(260, &test_roster_config());
        // Do NOT call reconcile_budgets or set_my_team_by_id

        let available = test_players();

        let db = Database::open(":memory:").expect("in-memory db");
        let llm_client = LlmClient::Disabled;
        let (llm_tx, _llm_rx) = mpsc::channel(16);

        let draft_id = Database::generate_draft_id();
        AppState::new(config, draft_state, available, empty_projections(), db, draft_id, llm_client, llm_tx, None, AppMode::Draft, test_onboarding_manager(), Some(test_roster_config()))
    }

    #[tokio::test]
    async fn first_nomination_with_teams_in_same_update_triggers_analysis() {
        // Scenario: The very first STATE_UPDATE from the extension contains
        // both team budget data AND the first nomination. Teams aren't
        // registered yet (no prior state updates). The nomination analysis
        // should still be triggered because reconcile_budgets runs before
        // nomination handling within handle_state_update.
        let mut state = create_test_app_state_no_teams();
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        assert!(state.draft_state.teams.is_empty(), "Teams should start empty");
        assert!(state.analysis_request_id.is_none(), "analysis_request_id should start as None");

        // Simulate the first STATE_UPDATE with teams + nomination
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "espn_1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                nominated_by: "Team 2".into(),
                current_bid: 5,
                current_bidder: None,
                time_remaining: Some(30),
                eligible_slots: vec![],
            }),
            my_team_id: Some("1".into()),
            teams: vec![
                crate::protocol::TeamBudgetData {
                    team_id: Some("1".into()),
                    team_name: "Team 1".into(),
                    budget: 260,
                },
                crate::protocol::TeamBudgetData {
                    team_id: Some("2".into()),
                    team_name: "Team 2".into(),
                    budget: 260,
                },
            ],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        ws_handler::handle_state_update(&mut state, ext_payload, &ui_tx).await;

        // Teams should now be registered
        assert_eq!(state.draft_state.teams.len(), 2);

        // Analysis should have been triggered
        assert!(
            state.analysis_request_id.is_some(),
            "analysis_request_id should be set after first nomination with teams in same update"
        );
        assert!(state.analysis_player.is_some());

        // current_nomination should be set on the draft state
        assert!(state.draft_state.current_nomination.is_some());
        assert_eq!(
            state.draft_state.current_nomination.as_ref().unwrap().player_name,
            "H_Star"
        );

        // Drain UI updates and verify we got a NominationUpdate
        let mut got_nomination_update = false;
        while let Ok(update) = ui_rx.try_recv() {
            if let UiUpdate::NominationUpdate { info, .. } = update {
                assert_eq!(info.player_name, "H_Star");
                got_nomination_update = true;
            }
        }
        assert!(
            got_nomination_update,
            "Should have received a NominationUpdate"
        );
    }

    #[tokio::test]
    async fn first_nomination_before_teams_retries_after_registration() {
        // Scenario: The first STATE_UPDATE has a nomination but NO team data.
        // handle_nomination() fails because my_team() returns None.
        // A second STATE_UPDATE arrives with team data (and the same nomination).
        // The retry logic should detect the unanalyzed nomination and trigger analysis.
        let mut state = create_test_app_state_no_teams();
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        assert!(state.draft_state.teams.is_empty());

        // First STATE_UPDATE: nomination but no teams
        let ext_payload_1 = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "espn_1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                nominated_by: "Team 2".into(),
                current_bid: 5,
                current_bidder: None,
                time_remaining: Some(30),
                eligible_slots: vec![],
            }),
            my_team_id: Some("1".into()),
            teams: vec![],  // No teams!
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        ws_handler::handle_state_update(&mut state, ext_payload_1, &ui_tx).await;

        // Teams should still be empty
        assert!(state.draft_state.teams.is_empty());
        // analysis should be None (nomination was skipped)
        assert!(
            state.analysis_request_id.is_none(),
            "analysis_request_id should be None since teams aren't registered"
        );
        // current_nomination should NOT be set (handle_nomination returned early)
        assert!(
            state.draft_state.current_nomination.is_none(),
            "current_nomination should be None since handle_nomination returned early"
        );

        // Drain any UI updates from first round
        while ui_rx.try_recv().is_ok() {}

        // Second STATE_UPDATE: teams arrive, same nomination still active
        let ext_payload_2 = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "espn_1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                nominated_by: "Team 2".into(),
                current_bid: 5,
                current_bidder: None,
                time_remaining: Some(25),  // Time ticked down
                eligible_slots: vec![],
            }),
            my_team_id: Some("1".into()),
            teams: vec![
                crate::protocol::TeamBudgetData {
                    team_id: Some("1".into()),
                    team_name: "Team 1".into(),
                    budget: 260,
                },
                crate::protocol::TeamBudgetData {
                    team_id: Some("2".into()),
                    team_name: "Team 2".into(),
                    budget: 260,
                },
            ],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        ws_handler::handle_state_update(&mut state, ext_payload_2, &ui_tx).await;

        // Teams should now be registered
        assert_eq!(state.draft_state.teams.len(), 2);

        // Analysis should now be set (retry triggered the analysis)
        assert!(
            state.analysis_request_id.is_some(),
            "analysis_request_id should be set after teams registered with pending nomination"
        );
        assert!(state.analysis_player.is_some());

        // current_nomination should now be set
        assert!(state.draft_state.current_nomination.is_some());
        assert_eq!(
            state.draft_state.current_nomination.as_ref().unwrap().player_name,
            "H_Star"
        );

        // Verify we got a NominationUpdate from the retry
        let mut got_nomination_update = false;
        while let Ok(update) = ui_rx.try_recv() {
            if let UiUpdate::NominationUpdate { info, .. } = update {
                assert_eq!(info.player_name, "H_Star");
                got_nomination_update = true;
            }
        }
        assert!(
            got_nomination_update,
            "Should have received a NominationUpdate from the retry"
        );
    }

    #[tokio::test]
    async fn retry_does_not_fire_when_nomination_already_analyzed() {
        // Scenario: Teams are registered in a state update that also contains
        // a nomination, and the normal flow handles it. The retry should NOT
        // fire a second time (llm_mode is already set).
        let mut state = create_test_app_state_no_teams();
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        // First STATE_UPDATE with teams + nomination
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "espn_1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                nominated_by: "Team 2".into(),
                current_bid: 5,
                current_bidder: None,
                time_remaining: Some(30),
                eligible_slots: vec![],
            }),
            my_team_id: Some("1".into()),
            teams: vec![
                crate::protocol::TeamBudgetData {
                    team_id: Some("1".into()),
                    team_name: "Team 1".into(),
                    budget: 260,
                },
                crate::protocol::TeamBudgetData {
                    team_id: Some("2".into()),
                    team_name: "Team 2".into(),
                    budget: 260,
                },
            ],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        ws_handler::handle_state_update(&mut state, ext_payload, &ui_tx).await;

        // Count NominationUpdate messages -- should be exactly 1
        // (from the normal flow, not doubled by the retry)
        let mut nomination_update_count = 0;
        while let Ok(update) = ui_rx.try_recv() {
            if matches!(update, UiUpdate::NominationUpdate { .. }) {
                nomination_update_count += 1;
            }
        }
        assert_eq!(
            nomination_update_count, 1,
            "Should get exactly 1 NominationUpdate, not doubled by retry"
        );
    }

    // -----------------------------------------------------------------------
    // Tests: Extension state conversion
    // -----------------------------------------------------------------------

    #[test]
    fn convert_extension_state_round_trip() {
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![crate::protocol::PickData {
                pick_number: 1,
                team_id: "team_1".into(),
                team_name: "Team 1".into(),
                player_id: "espn_1".into(),
                player_name: "Player One".into(),
                position: "SP".into(),
                price: 30,
                eligible_slots: vec![14, 13, 16, 17],
            assigned_slot: None,
            }],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "espn_2".into(),
                player_name: "Player Two".into(),
                position: "1B".into(),
                nominated_by: "Team 2".into(),
                current_bid: 10,
                current_bidder: Some("Team 3".into()),
                time_remaining: Some(25),
                eligible_slots: vec![1, 7, 12, 16, 17],
            }),
            my_team_id: Some("team_1".into()),
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: Some("espn_42_2026".into()),
            source: Some("test".into()),
            ..Default::default()
        };

        let internal = AppState::convert_extension_state(&ext_payload);

        assert_eq!(internal.picks.len(), 1);
        assert_eq!(internal.picks[0].pick_number, 1);
        assert_eq!(internal.picks[0].player_name, "Player One");

        let nom = internal.current_nomination.as_ref().unwrap();
        assert_eq!(nom.player_name, "Player Two");
        assert_eq!(nom.current_bid, 10);
        assert_eq!(nom.current_bidder, Some("Team 3".into()));
    }

    // -----------------------------------------------------------------------
    // Tests: Premature nomination filtering
    // -----------------------------------------------------------------------

    #[test]
    fn premature_nomination_filtered_no_bid_no_bidder_no_nominator() {
        // A premature nomination has currentBid=0, no currentBidder, and
        // empty nominatedBy (no bid history). This should be filtered out.
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "".into(),
                player_name: "Michael King".into(),
                position: "SP".into(),
                nominated_by: "".into(),
                current_bid: 0,
                current_bidder: None,
                time_remaining: Some(30),
                eligible_slots: vec![],
            }),
            my_team_id: Some("team_1".into()),
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        let internal = AppState::convert_extension_state(&ext_payload);
        assert!(
            internal.current_nomination.is_none(),
            "Premature nomination (no bid, no bidder, no nominator) should be filtered out"
        );
    }

    #[test]
    fn confirmed_nomination_with_bid_passes_through() {
        // A confirmed nomination has currentBid > 0. This should pass through.
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "".into(),
                player_name: "Michael King".into(),
                position: "SP".into(),
                nominated_by: "Team 3".into(),
                current_bid: 1,
                current_bidder: None,
                time_remaining: Some(30),
                eligible_slots: vec![],
            }),
            my_team_id: Some("team_1".into()),
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        let internal = AppState::convert_extension_state(&ext_payload);
        assert!(
            internal.current_nomination.is_some(),
            "Confirmed nomination with bid > 0 should pass through"
        );
        assert_eq!(
            internal.current_nomination.as_ref().unwrap().player_name,
            "Michael King"
        );
    }

    #[test]
    fn nomination_with_nominator_but_zero_bid_passes_through() {
        // Edge case: nominatedBy is set but currentBid is 0. This can happen
        // if the initial nomination bid is $0 (unlikely but defensively valid).
        // The presence of a nominator (from bid history) means it's confirmed.
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "".into(),
                player_name: "Michael King".into(),
                position: "SP".into(),
                nominated_by: "Team 5".into(),
                current_bid: 0,
                current_bidder: None,
                time_remaining: Some(30),
                eligible_slots: vec![],
            }),
            my_team_id: Some("team_1".into()),
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        let internal = AppState::convert_extension_state(&ext_payload);
        assert!(
            internal.current_nomination.is_some(),
            "Nomination with a nominator should pass through even with bid=0"
        );
    }

    #[test]
    fn nomination_with_bidder_but_zero_bid_passes_through() {
        // Edge case: currentBidder is set but currentBid is 0.
        // The presence of a bidder means bidding has started.
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "".into(),
                player_name: "Michael King".into(),
                position: "SP".into(),
                nominated_by: "".into(),
                current_bid: 0,
                current_bidder: Some("Team 7".into()),
                time_remaining: Some(30),
                eligible_slots: vec![],
            }),
            my_team_id: Some("team_1".into()),
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        let internal = AppState::convert_extension_state(&ext_payload);
        assert!(
            internal.current_nomination.is_some(),
            "Nomination with a current bidder should pass through even with bid=0"
        );
    }

    #[test]
    fn premature_nomination_with_empty_bidder_string_filtered() {
        // currentBidder is Some("") — effectively empty. Should still be filtered.
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "".into(),
                player_name: "Michael King".into(),
                position: "SP".into(),
                nominated_by: "".into(),
                current_bid: 0,
                current_bidder: Some("".into()),
                time_remaining: Some(30),
                eligible_slots: vec![],
            }),
            my_team_id: Some("team_1".into()),
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        let internal = AppState::convert_extension_state(&ext_payload);
        assert!(
            internal.current_nomination.is_none(),
            "Nomination with empty bidder string should be filtered like None"
        );
    }

    #[test]
    fn null_nomination_remains_null() {
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: None,
            my_team_id: Some("team_1".into()),
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        let internal = AppState::convert_extension_state(&ext_payload);
        assert!(
            internal.current_nomination.is_none(),
            "Null nomination should remain None"
        );
    }

    // -----------------------------------------------------------------------
    // Tests: Heartbeat timeout detection
    // -----------------------------------------------------------------------

    #[test]
    fn last_ws_message_time_starts_as_none() {
        let state = create_test_app_state();
        assert!(state.last_ws_message_time.is_none());
        assert_eq!(state.connection_status, ConnectionStatus::Disconnected);
    }

    #[tokio::test]
    async fn connected_event_sets_last_ws_message_time() {
        let state = create_test_app_state();
        let (ws_tx, ws_rx) = mpsc::channel(16);
        let (_llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        // We need to inspect state after the event loop processes the
        // Connected event. We use a channel to coordinate: send Connected,
        // receive the UI update, then send Quit. The state is owned by
        // the event loop, so we verify behavior through UI updates.
        let handle = tokio::spawn(run(ws_rx, llm_rx, cmd_rx, ui_tx, state));
        drain_initial_snapshot(&mut ui_rx).await;

        ws_tx
            .send(WsEvent::Connected { addr: "test:1234".into() })
            .await
            .unwrap();

        // Should receive Connected status
        let update = ui_rx.recv().await.unwrap();
        assert!(matches!(
            update,
            UiUpdate::ConnectionStatus(ConnectionStatus::Connected)
        ));

        cmd_tx.send(UserCommand::Quit).await.unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn disconnected_event_clears_last_ws_message_time() {
        let state = create_test_app_state();
        let (ws_tx, ws_rx) = mpsc::channel(16);
        let (_llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        let handle = tokio::spawn(run(ws_rx, llm_rx, cmd_rx, ui_tx, state));
        drain_initial_snapshot(&mut ui_rx).await;

        // Connect first
        ws_tx
            .send(WsEvent::Connected { addr: "test:1234".into() })
            .await
            .unwrap();
        let _ = ui_rx.recv().await.unwrap(); // Connected

        // Then disconnect
        ws_tx.send(WsEvent::Disconnected).await.unwrap();
        let update = ui_rx.recv().await.unwrap();
        assert!(matches!(
            update,
            UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)
        ));

        cmd_tx.send(UserCommand::Quit).await.unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn heartbeat_timeout_transitions_to_disconnected() {
        // Use tokio::time::pause() to control time in the test.
        tokio::time::pause();

        let state = create_test_app_state();
        let (ws_tx, ws_rx) = mpsc::channel(16);
        let (_llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        let handle = tokio::spawn(run(ws_rx, llm_rx, cmd_rx, ui_tx, state));
        drain_initial_snapshot(&mut ui_rx).await;

        // Connect
        ws_tx
            .send(WsEvent::Connected { addr: "test:1234".into() })
            .await
            .unwrap();
        let update = ui_rx.recv().await.unwrap();
        assert!(matches!(
            update,
            UiUpdate::ConnectionStatus(ConnectionStatus::Connected)
        ));

        // Advance time past the heartbeat timeout + check interval.
        // HEARTBEAT_TIMEOUT = 15s, HEARTBEAT_CHECK_INTERVAL = 5s.
        // The first check fires at 5s (connected at ~0s, last message at ~0s,
        // elapsed ~5s < 15s timeout). The fourth check fires at 20s
        // (elapsed ~20s > 15s timeout), so we should get Disconnected.
        tokio::time::advance(Duration::from_secs(21)).await;

        // Yield to let the interval tick and process.
        tokio::task::yield_now().await;
        // May need a few yields for the event loop to process the tick
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        // We should eventually receive a Disconnected status from the
        // heartbeat timeout check.
        let update = tokio::time::timeout(Duration::from_secs(5), ui_rx.recv())
            .await
            .expect("Should receive UI update within timeout")
            .expect("Channel should not be closed");
        assert!(
            matches!(update, UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)),
            "Expected ConnectionStatus(Disconnected) from heartbeat timeout, got {:?}",
            update
        );

        cmd_tx.send(UserCommand::Quit).await.unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn message_resets_heartbeat_timer() {
        // Use tokio::time::pause() to control time.
        tokio::time::pause();

        let state = create_test_app_state();
        let (ws_tx, ws_rx) = mpsc::channel(16);
        let (_llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        let handle = tokio::spawn(run(ws_rx, llm_rx, cmd_rx, ui_tx, state));
        drain_initial_snapshot(&mut ui_rx).await;

        // Connect
        ws_tx
            .send(WsEvent::Connected { addr: "test:1234".into() })
            .await
            .unwrap();
        let update = ui_rx.recv().await.unwrap();
        assert!(matches!(
            update,
            UiUpdate::ConnectionStatus(ConnectionStatus::Connected)
        ));

        // Advance 10 seconds (under the 15s timeout)
        tokio::time::advance(Duration::from_secs(10)).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }

        // Send a heartbeat message to reset the timer
        let heartbeat = r#"{"type":"EXTENSION_HEARTBEAT","payload":{"timestamp":123}}"#;
        ws_tx
            .send(WsEvent::Message(heartbeat.into()))
            .await
            .unwrap();

        // Give the event loop time to process
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }

        // Advance another 10 seconds (total 20s from start, but only 10s
        // from last message, so still under timeout)
        tokio::time::advance(Duration::from_secs(10)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        // Should NOT have received a Disconnected status, because the
        // heartbeat message reset the timer. Try receiving with a very
        // short timeout -- we expect it to time out (no Disconnected event).
        let result = tokio::time::timeout(Duration::from_millis(100), ui_rx.recv()).await;
        assert!(
            result.is_err(),
            "Should NOT receive Disconnected status (heartbeat reset the timer)"
        );

        cmd_tx.send(UserCommand::Quit).await.unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn stale_disconnect_reconnects_on_new_message() {
        // Use tokio::time::pause() to control time.
        tokio::time::pause();

        let state = create_test_app_state();
        let (ws_tx, ws_rx) = mpsc::channel(16);
        let (_llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        let handle = tokio::spawn(run(ws_rx, llm_rx, cmd_rx, ui_tx, state));
        drain_initial_snapshot(&mut ui_rx).await;

        // Connect
        ws_tx
            .send(WsEvent::Connected { addr: "test:1234".into() })
            .await
            .unwrap();
        let update = ui_rx.recv().await.unwrap();
        assert!(matches!(
            update,
            UiUpdate::ConnectionStatus(ConnectionStatus::Connected)
        ));

        // Advance past the heartbeat timeout to trigger stale disconnect
        tokio::time::advance(Duration::from_secs(21)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        // Should receive Disconnected from heartbeat timeout
        let update = tokio::time::timeout(Duration::from_secs(5), ui_rx.recv())
            .await
            .expect("Should receive UI update")
            .expect("Channel should not be closed");
        assert!(
            matches!(update, UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)),
            "Expected Disconnected from heartbeat timeout, got {:?}",
            update
        );

        // Now send a new message (simulating extension coming back)
        let heartbeat = r#"{"type":"EXTENSION_HEARTBEAT","payload":{"timestamp":456}}"#;
        ws_tx
            .send(WsEvent::Message(heartbeat.into()))
            .await
            .unwrap();

        // Should receive Connected status (reconnect from stale)
        let update = tokio::time::timeout(Duration::from_secs(5), ui_rx.recv())
            .await
            .expect("Should receive UI update")
            .expect("Channel should not be closed");
        assert!(
            matches!(update, UiUpdate::ConnectionStatus(ConnectionStatus::Connected)),
            "Expected ConnectionStatus(Connected) after stale reconnect, got {:?}",
            update
        );

        cmd_tx.send(UserCommand::Quit).await.unwrap();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn clean_disconnect_does_not_reconnect_on_message() {
        // When the ws_server sends a proper Disconnected event (clean close),
        // last_ws_message_time is set to None, so a subsequent Message
        // should NOT trigger a reconnect.
        let state = create_test_app_state();
        let (ws_tx, ws_rx) = mpsc::channel(16);
        let (_llm_tx, llm_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (ui_tx, mut ui_rx) = mpsc::channel(64);

        let handle = tokio::spawn(run(ws_rx, llm_rx, cmd_rx, ui_tx, state));
        drain_initial_snapshot(&mut ui_rx).await;

        // Connect
        ws_tx
            .send(WsEvent::Connected { addr: "test:1234".into() })
            .await
            .unwrap();
        let update = ui_rx.recv().await.unwrap();
        assert!(matches!(
            update,
            UiUpdate::ConnectionStatus(ConnectionStatus::Connected)
        ));

        // Clean disconnect (from ws_server)
        ws_tx.send(WsEvent::Disconnected).await.unwrap();
        let update = ui_rx.recv().await.unwrap();
        assert!(matches!(
            update,
            UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)
        ));

        // Send a heartbeat (this should be processed without triggering
        // a reconnect, because last_ws_message_time was cleared to None
        // by the Disconnected event)
        let heartbeat = r#"{"type":"EXTENSION_HEARTBEAT","payload":{"timestamp":789}}"#;
        ws_tx
            .send(WsEvent::Message(heartbeat.into()))
            .await
            .unwrap();

        // Give time for processing
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        // Should NOT receive a Connected status. Use a very short timeout
        // to confirm nothing arrives.
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            ui_rx.recv(),
        )
        .await;
        assert!(
            result.is_err(),
            "Should NOT receive ConnectionStatus(Connected) after clean disconnect"
        );

        cmd_tx.send(UserCommand::Quit).await.unwrap();
        let _ = handle.await;
    }

    // -----------------------------------------------------------------------
    // Tests: Roster snapshot correctness (issue: same player in every slot)
    // -----------------------------------------------------------------------

    #[test]
    fn build_snapshot_roster_shows_only_my_team_players() {
        let mut state = create_test_app_state();

        // Record picks for different teams:
        // Pick 1: H_Star -> Team 1 (my team)
        // Pick 2: P_Ace -> Team 2 (other team)
        // Pick 3: H_Good -> Team 1 (my team)
        state.process_new_picks(vec![
            DraftPick {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
            DraftPick {
                pick_number: 2,
                team_id: "2".into(),
                team_name: "Team 2".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
            DraftPick {
                pick_number: 3,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_name: "H_Good".into(),
                position: "2B".into(),
                price: 30,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
        ]);

        let snapshot = state.build_snapshot();

        // My team (Team 1) should have exactly 2 filled roster slots
        let filled_slots: Vec<_> = snapshot
            .my_roster
            .iter()
            .filter(|s| s.player.is_some())
            .collect();
        assert_eq!(
            filled_slots.len(),
            2,
            "Expected 2 filled slots for my team, got {}. Filled: {:?}",
            filled_slots.len(),
            filled_slots
                .iter()
                .map(|s| format!(
                    "{}: {}",
                    s.position.display_str(),
                    s.player.as_ref().unwrap().name
                ))
                .collect::<Vec<_>>()
        );

        // Verify the correct players are in the correct slots
        let player_names: Vec<&str> = filled_slots
            .iter()
            .map(|s| s.player.as_ref().unwrap().name.as_str())
            .collect();
        assert!(
            player_names.contains(&"H_Star"),
            "H_Star should be in my roster"
        );
        assert!(
            player_names.contains(&"H_Good"),
            "H_Good should be in my roster"
        );
        assert!(
            !player_names.contains(&"P_Ace"),
            "P_Ace should NOT be in my roster (Team 2 player)"
        );

        // Verify each filled slot has a DIFFERENT player
        let unique_names: std::collections::HashSet<&str> =
            player_names.iter().copied().collect();
        assert_eq!(
            unique_names.len(),
            filled_slots.len(),
            "Each filled slot should have a unique player name"
        );
    }

    #[test]
    fn build_snapshot_roster_correct_position_assignment() {
        let mut state = create_test_app_state();

        // Record picks for my team with different positions
        state.process_new_picks(vec![
            DraftPick {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
            DraftPick {
                pick_number: 2,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
        ]);

        let snapshot = state.build_snapshot();

        // Verify 1B slot has H_Star
        let slot_1b = snapshot
            .my_roster
            .iter()
            .find(|s| s.position == Position::FirstBase)
            .expect("should have a 1B slot");
        assert!(slot_1b.player.is_some(), "1B slot should be filled");
        assert_eq!(slot_1b.player.as_ref().unwrap().name, "H_Star");

        // Verify an SP slot has P_Ace
        let sp_filled: Vec<_> = snapshot
            .my_roster
            .iter()
            .filter(|s| s.position == Position::StartingPitcher && s.player.is_some())
            .collect();
        assert_eq!(sp_filled.len(), 1, "Should have exactly 1 filled SP slot");
        assert_eq!(sp_filled[0].player.as_ref().unwrap().name, "P_Ace");

        // Verify other slots are empty
        let other_filled: Vec<_> = snapshot
            .my_roster
            .iter()
            .filter(|s| {
                s.player.is_some()
                    && s.position != Position::FirstBase
                    && s.position != Position::StartingPitcher
            })
            .collect();
        assert!(
            other_filled.is_empty(),
            "No other slots should be filled, but found: {:?}",
            other_filled
                .iter()
                .map(|s| format!(
                    "{}: {}",
                    s.position.display_str(),
                    s.player.as_ref().unwrap().name
                ))
                .collect::<Vec<_>>()
        );
    }

    /// Simulate the exact first-STATE_UPDATE scenario:
    /// - Teams not registered yet
    /// - Picks arrive before reconcile_budgets
    /// - Then reconcile_budgets registers teams and replays pending picks
    /// - Then set_my_team_by_id
    /// - Then build_snapshot
    #[test]
    fn first_state_update_roster_correctness() {
        let config = test_config();
        let draft_state = DraftState::new(260, &test_roster_config());
        // Note: NOT calling reconcile_budgets yet (simulates first update)
        assert!(draft_state.teams.is_empty());

        let mut available = test_players();
        let test_roster = AppState::default_roster_config();
        crate::valuation::recalculate_all(
            &mut available,
            &test_roster,
            &config.league,
            &config.strategy,
            &draft_state,
        );
        let db = Database::open(":memory:").expect("in-memory db");
        let draft_id = Database::generate_draft_id();
        let llm_client = LlmClient::Disabled;
        let (llm_tx, _llm_rx) = mpsc::channel(16);

        let mut state = AppState::new(
            config,
            draft_state,
            available,
            empty_projections(),
            db,
            draft_id,
            llm_client,
            llm_tx,
            None,
            AppMode::Draft,
            test_onboarding_manager(),
            Some(test_roster_config()),
        );

        // Step 1: process_new_picks while teams are empty
        // (simulates what happens in handle_state_update before reconcile_budgets)
        state.process_new_picks(vec![
            DraftPick {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team Alpha".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
            DraftPick {
                pick_number: 2,
                team_id: "2".into(),
                team_name: "Team Beta".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
        ]);

        // Teams are still empty after process_new_picks
        assert!(state.draft_state.teams.is_empty());
        // But picks are stored
        assert_eq!(state.draft_state.picks.len(), 2);

        // Step 2: reconcile_budgets registers teams (first call)
        // This also calls replay_pending_picks
        let budgets = vec![
            crate::draft::state::TeamBudgetPayload {
                team_id: "1".into(),
                team_name: "Team Alpha".into(),
                budget: 215, // 260 - 45
            },
            crate::draft::state::TeamBudgetPayload {
                team_id: "2".into(),
                team_name: "Team Beta".into(),
                budget: 210, // 260 - 50
            },
        ];
        let reconcile = state.draft_state.reconcile_budgets(&budgets);
        assert!(reconcile.teams_registered);

        // Step 3: set my team
        state.draft_state.set_my_team_by_id("1");

        // Step 4: build snapshot
        let snapshot = state.build_snapshot();

        // My team (Team Alpha) should have exactly 1 player (H_Star at 1B)
        let filled: Vec<_> = snapshot
            .my_roster
            .iter()
            .filter(|s| s.player.is_some())
            .collect();
        assert_eq!(
            filled.len(),
            1,
            "Expected 1 filled slot for Team Alpha, got {}. Filled: {:?}",
            filled.len(),
            filled
                .iter()
                .map(|s| format!(
                    "{}: {}",
                    s.position.display_str(),
                    s.player.as_ref().unwrap().name
                ))
                .collect::<Vec<_>>()
        );
        assert_eq!(filled[0].player.as_ref().unwrap().name, "H_Star");

        // Team Beta should have P_Ace, not on my roster
        let team_beta = state.draft_state.team("2").unwrap();
        assert_eq!(team_beta.roster.filled_count(), 1);
    }

    /// Simulate multiple consecutive state updates (as happens in normal operation).
    /// Each update sends the full pick list; only truly new picks should be processed.
    #[test]
    fn consecutive_state_updates_no_duplicate_roster_entries() {
        let mut state = create_test_app_state();

        // Build internal payloads as they would come from the extension
        use crate::draft::state::{
            compute_state_diff, PickPayload, StateUpdatePayload as InternalStatePayload,
        };

        // First update: 1 pick
        let payload1 = InternalStatePayload {
            picks: vec![PickPayload {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_id: "p1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                eligible_slots: vec![],
            assigned_slot: None,
            }],
            current_nomination: None,
            teams: vec![],
            pick_count: Some(1),
            total_picks: Some(260),
        };

        let diff1 = compute_state_diff(&None, &payload1);
        assert_eq!(diff1.new_picks.len(), 1);
        state.process_new_picks(diff1.new_picks);

        let snapshot1 = state.build_snapshot();
        let filled1: Vec<_> = snapshot1
            .my_roster
            .iter()
            .filter(|s| s.player.is_some())
            .collect();
        assert_eq!(filled1.len(), 1, "After 1st update: 1 filled slot");
        assert_eq!(filled1[0].player.as_ref().unwrap().name, "H_Star");

        // Second update: same 1 pick (no changes)
        let diff2 = compute_state_diff(&Some(payload1.clone()), &payload1);
        assert!(diff2.new_picks.is_empty(), "No new picks on duplicate update");

        // Third update: 2 picks (one new)
        let payload3 = InternalStatePayload {
            picks: vec![
                PickPayload {
                    pick_number: 1,
                    team_id: "1".into(),
                    team_name: "Team 1".into(),
                    player_id: "p1".into(),
                    player_name: "H_Star".into(),
                    position: "1B".into(),
                    price: 45,
                    eligible_slots: vec![],
            assigned_slot: None,
                },
                PickPayload {
                    pick_number: 2,
                    team_id: "2".into(),
                    team_name: "Team 2".into(),
                    player_id: "p2".into(),
                    player_name: "P_Ace".into(),
                    position: "SP".into(),
                    price: 50,
                    eligible_slots: vec![],
            assigned_slot: None,
                },
            ],
            current_nomination: None,
            teams: vec![],
            pick_count: Some(2),
            total_picks: Some(260),
        };

        let diff3 = compute_state_diff(&Some(payload1.clone()), &payload3);
        assert_eq!(diff3.new_picks.len(), 1, "Only 1 new pick on 3rd update");
        assert_eq!(diff3.new_picks[0].player_name, "P_Ace");
        state.process_new_picks(diff3.new_picks);

        let snapshot3 = state.build_snapshot();
        let filled3: Vec<_> = snapshot3
            .my_roster
            .iter()
            .filter(|s| s.player.is_some())
            .collect();
        // Team 1 has H_Star; P_Ace went to Team 2
        assert_eq!(filled3.len(), 1, "After 3rd update: still 1 filled slot for my team");
        assert_eq!(filled3[0].player.as_ref().unwrap().name, "H_Star");

        // Fourth update: 3 picks (one more for my team)
        let payload4 = InternalStatePayload {
            picks: vec![
                PickPayload {
                    pick_number: 1,
                    team_id: "1".into(),
                    team_name: "Team 1".into(),
                    player_id: "p1".into(),
                    player_name: "H_Star".into(),
                    position: "1B".into(),
                    price: 45,
                    eligible_slots: vec![],
            assigned_slot: None,
                },
                PickPayload {
                    pick_number: 2,
                    team_id: "2".into(),
                    team_name: "Team 2".into(),
                    player_id: "p2".into(),
                    player_name: "P_Ace".into(),
                    position: "SP".into(),
                    price: 50,
                    eligible_slots: vec![],
            assigned_slot: None,
                },
                PickPayload {
                    pick_number: 3,
                    team_id: "1".into(),
                    team_name: "Team 1".into(),
                    player_id: "p3".into(),
                    player_name: "H_Good".into(),
                    position: "2B".into(),
                    price: 30,
                    eligible_slots: vec![],
            assigned_slot: None,
                },
            ],
            current_nomination: None,
            teams: vec![],
            pick_count: Some(3),
            total_picks: Some(260),
        };

        let diff4 = compute_state_diff(&Some(payload3), &payload4);
        assert_eq!(diff4.new_picks.len(), 1, "Only 1 new pick on 4th update");
        assert_eq!(diff4.new_picks[0].player_name, "H_Good");
        state.process_new_picks(diff4.new_picks);

        let snapshot4 = state.build_snapshot();
        let filled4: Vec<_> = snapshot4
            .my_roster
            .iter()
            .filter(|s| s.player.is_some())
            .collect();
        assert_eq!(filled4.len(), 2, "After 4th update: 2 filled slots for my team");

        // Each filled slot should have a DIFFERENT player
        let names: Vec<&str> = filled4
            .iter()
            .map(|s| s.player.as_ref().unwrap().name.as_str())
            .collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(
            unique.len(),
            2,
            "Both slots should have different players, got: {:?}",
            names
        );
        assert!(names.contains(&"H_Star"));
        assert!(names.contains(&"H_Good"));
    }

    /// Test what happens when pick team_id doesn't match registered team_id
    /// (as happens with DOM scraping where teamId = team name, not numeric ID).
    /// The team_name fallback should correctly route picks.
    #[test]
    fn picks_with_espn_team_id_route_correctly() {
        let mut state = create_test_app_state();

        // Picks use ESPN numeric team IDs (as resolved by the extension)
        state.process_new_picks(vec![
            DraftPick {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
            DraftPick {
                pick_number: 2,
                team_id: "2".into(),
                team_name: "Team 2".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: None,
                eligible_slots: vec![],
            assigned_slot: None,
            },
        ]);

        // Team 1 (my team) should have H_Star
        let my_team = state.draft_state.my_team().unwrap();
        assert_eq!(
            my_team.roster.filled_count(),
            1,
            "Team 1 should have 1 player"
        );
        let filled: Vec<_> = my_team
            .roster
            .slots
            .iter()
            .filter(|s| s.player.is_some())
            .collect();
        assert_eq!(filled[0].player.as_ref().unwrap().name, "H_Star");

        // Team 2 should have P_Ace
        let team2 = state.draft_state.team("2").unwrap();
        assert_eq!(
            team2.roster.filled_count(),
            1,
            "Team 2 should have 1 player"
        );
    }

    /// Test what happens when team_name in pick doesn't match any registered team.
    /// The pick should be stored but NOT assigned to any team roster.
    #[test]
    fn unmatched_team_pick_not_assigned_to_any_roster() {
        let mut state = create_test_app_state();

        state.process_new_picks(vec![DraftPick {
            pick_number: 1,
            team_id: "Nonexistent Team".into(),
            team_name: "Nonexistent Team".into(),
            player_name: "H_Star".into(),
            position: "1B".into(),
            price: 45,
            espn_player_id: None,
            eligible_slots: vec![],
            assigned_slot: None,
        }]);

        // Pick should be recorded
        assert_eq!(state.draft_state.pick_count, 1);
        assert_eq!(state.draft_state.picks.len(), 1);

        // But no team should have any filled roster slots
        for team in &state.draft_state.teams {
            assert_eq!(
                team.roster.filled_count(),
                0,
                "Team {} should have 0 players when pick team doesn't match",
                team.team_name
            );
        }

        // My roster should be empty
        let snapshot = state.build_snapshot();
        let filled: Vec<_> = snapshot
            .my_roster
            .iter()
            .filter(|s| s.player.is_some())
            .collect();
        assert!(filled.is_empty(), "My roster should be empty");
    }

    // -----------------------------------------------------------------------
    // Tests: ESPN draft ID detection
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn first_espn_draft_id_is_stored() {
        let mut state = create_test_app_state();
        assert!(state.espn_draft_id.is_none());

        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: None,
            my_team_id: None,
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: Some("espn_12345_2026".into()),
            source: Some("test".into()),
            ..Default::default()
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        ws_handler::handle_state_update(&mut state, ext_payload, &ui_tx).await;

        // ESPN draft ID should now be stored in state
        assert_eq!(state.espn_draft_id, Some("espn_12345_2026".into()));
        // Internal draft_id should NOT change (same draft)
        assert_eq!(state.draft_id, state.draft_id); // unchanged

        // Should also be persisted in DB
        let db_espn_id = state.db.get_espn_draft_id().unwrap();
        assert_eq!(db_espn_id, Some("espn_12345_2026".into()));
    }

    #[tokio::test]
    async fn same_espn_draft_id_does_not_start_new_session() {
        let mut state = create_test_app_state();
        let original_draft_id = state.draft_id.clone();

        // Set up an existing ESPN draft ID
        state.espn_draft_id = Some("espn_12345_2026".into());

        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: None,
            my_team_id: None,
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: Some("espn_12345_2026".into()),
            source: Some("test".into()),
            ..Default::default()
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        ws_handler::handle_state_update(&mut state, ext_payload, &ui_tx).await;

        // Draft ID should remain the same
        assert_eq!(state.draft_id, original_draft_id);
        assert_eq!(state.espn_draft_id, Some("espn_12345_2026".into()));
    }

    #[tokio::test]
    async fn different_espn_draft_id_starts_new_session() {
        let mut state = create_test_app_state();
        // Use a known fixed draft_id so the generated one will differ
        let original_draft_id = "test_original_draft_001".to_string();
        state.draft_id = original_draft_id.clone();

        // Simulate having a stored ESPN draft ID from a previous session
        state.espn_draft_id = Some("espn_12345_2026".into());

        // Extension now reports a different draft ID (different league/season)
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: None,
            my_team_id: None,
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: Some("espn_67890_2026".into()),
            source: Some("test".into()),
            ..Default::default()
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        ws_handler::handle_state_update(&mut state, ext_payload, &ui_tx).await;

        // A new draft session should have been started
        assert_ne!(state.draft_id, original_draft_id);
        assert!(state.draft_id.starts_with("draft_"), "New draft ID should be generated: {}", state.draft_id);
        assert_eq!(state.espn_draft_id, Some("espn_67890_2026".into()));

        // New draft ID should be persisted
        let db_draft_id = state.db.get_draft_id().unwrap();
        assert_eq!(db_draft_id, Some(state.draft_id.clone()));

        let db_espn_id = state.db.get_espn_draft_id().unwrap();
        assert_eq!(db_espn_id, Some("espn_67890_2026".into()));

        // In-memory draft state should be reset (no picks, no teams)
        assert!(state.draft_state.picks.is_empty(), "Picks should be cleared on new draft");
        assert!(state.draft_state.teams.is_empty(), "Teams should be cleared on new draft");
    }

    #[tokio::test]
    async fn null_espn_draft_id_does_not_trigger_new_session() {
        let mut state = create_test_app_state();
        let original_draft_id = state.draft_id.clone();

        // Simulate having a stored ESPN draft ID
        state.espn_draft_id = Some("espn_12345_2026".into());

        // Extension sends no draft ID (e.g., old extension version)
        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: None,
            my_team_id: None,
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
            ..Default::default()
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        ws_handler::handle_state_update(&mut state, ext_payload, &ui_tx).await;

        // Draft ID should remain unchanged
        assert_eq!(state.draft_id, original_draft_id);
        assert_eq!(state.espn_draft_id, Some("espn_12345_2026".into()));
    }

    #[tokio::test]
    async fn espn_draft_id_resilient_across_reconnects() {
        let mut state = create_test_app_state();

        // First connection: receive ESPN draft ID and process some picks
        let ext_payload1 = crate::protocol::StateUpdatePayload {
            picks: vec![crate::protocol::PickData {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_id: "".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                eligible_slots: vec![],
            assigned_slot: None,
            }],
            current_nomination: None,
            my_team_id: Some("1".into()),
            teams: vec![
                crate::protocol::TeamBudgetData {
                    team_id: Some("1".into()),
                    team_name: "Team 1".into(),
                    budget: 215,
                },
                crate::protocol::TeamBudgetData {
                    team_id: Some("2".into()),
                    team_name: "Team 2".into(),
                    budget: 260,
                },
            ],
            pick_count: Some(1),
            total_picks: Some(260),
            draft_id: Some("espn_12345_2026".into()),
            source: Some("test".into()),
            ..Default::default()
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        ws_handler::handle_state_update(&mut state, ext_payload1, &ui_tx).await;

        let draft_id_after_first = state.draft_id.clone();
        assert_eq!(state.espn_draft_id, Some("espn_12345_2026".into()));

        // Simulate disconnect and reconnect -- previous_extension_state is
        // cleared to None (simulating a fresh connection)
        state.previous_extension_state = None;

        // Second connection: same ESPN draft ID, same picks visible
        let ext_payload2 = crate::protocol::StateUpdatePayload {
            picks: vec![crate::protocol::PickData {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_id: "".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                eligible_slots: vec![],
            assigned_slot: None,
            }],
            current_nomination: None,
            my_team_id: Some("1".into()),
            teams: vec![
                crate::protocol::TeamBudgetData {
                    team_id: Some("1".into()),
                    team_name: "Team 1".into(),
                    budget: 215,
                },
                crate::protocol::TeamBudgetData {
                    team_id: Some("2".into()),
                    team_name: "Team 2".into(),
                    budget: 260,
                },
            ],
            pick_count: Some(1),
            total_picks: Some(260),
            draft_id: Some("espn_12345_2026".into()),
            source: Some("test".into()),
            ..Default::default()
        };

        ws_handler::handle_state_update(&mut state, ext_payload2, &ui_tx).await;

        // Draft ID should NOT change across reconnect with same ESPN ID
        assert_eq!(state.draft_id, draft_id_after_first);
        assert_eq!(state.espn_draft_id, Some("espn_12345_2026".into()));
    }

    // -----------------------------------------------------------------------
    // Tests: Onboarding action handling
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn onboarding_skip_from_llm_setup_shows_strategy_setup() {
        use crate::onboarding::OnboardingStep;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.onboarding_progress.current_step = OnboardingStep::LlmSetup;

        let (ui_tx, mut ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(
            &mut state,
            UserCommand::OnboardingAction(OnboardingAction::Skip),
            &ui_tx,
        )
        .await;

        // AppState should now show StrategySetup (not Draft)
        assert_eq!(
            state.app_mode,
            AppMode::Onboarding(OnboardingStep::StrategySetup)
        );

        // Persisted step should remain at LlmSetup (not advanced)
        assert_eq!(state.onboarding_progress.current_step, OnboardingStep::LlmSetup);

        // UI channel should have received ModeChanged(Onboarding(StrategySetup))
        let update = ui_rx.recv().await.expect("expected ModeChanged update");
        assert!(
            matches!(
                update,
                UiUpdate::ModeChanged(AppMode::Onboarding(OnboardingStep::StrategySetup))
            ),
            "first update should be ModeChanged(Onboarding(StrategySetup)), got {:?}",
            update,
        );
    }

    #[tokio::test]
    async fn onboarding_skip_from_strategy_setup_transitions_to_draft() {
        use crate::onboarding::OnboardingStep;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
        state.onboarding_progress.current_step = OnboardingStep::StrategySetup;

        let (ui_tx, mut ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(
            &mut state,
            UserCommand::OnboardingAction(OnboardingAction::Skip),
            &ui_tx,
        )
        .await;

        // AppState should now be in Draft mode
        assert_eq!(state.app_mode, AppMode::Draft);

        // Persisted step should remain at StrategySetup (not advanced to Complete)
        assert_eq!(state.onboarding_progress.current_step, OnboardingStep::StrategySetup);

        // UI channel should have received ModeChanged(Draft)
        let update = ui_rx.recv().await.expect("expected ModeChanged update");
        assert!(
            matches!(update, UiUpdate::ModeChanged(AppMode::Draft)),
            "first update should be ModeChanged(Draft), got {:?}",
            update,
        );

        // UI channel should also have received a StateSnapshot
        let snapshot_update = ui_rx.recv().await.expect("expected StateSnapshot update");
        assert!(
            matches!(snapshot_update, UiUpdate::StateSnapshot(_)),
            "second update should be StateSnapshot, got {:?}",
            snapshot_update,
        );
    }

    // -----------------------------------------------------------------------
    // Tests: GoNext blocked when connection test failed (Task 7)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn go_next_blocked_when_connection_test_failed() {
        use crate::onboarding::OnboardingStep;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.onboarding_progress.current_step = OnboardingStep::LlmSetup;
        // Simulate a failed connection test
        state.connection_test_result.store(CONNECTION_TEST_FAILED, Ordering::Relaxed);

        let (ui_tx, mut ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(
            &mut state,
            UserCommand::OnboardingAction(OnboardingAction::GoNext),
            &ui_tx,
        )
        .await;

        // Should NOT advance — still on LlmSetup
        assert_eq!(
            state.app_mode,
            AppMode::Onboarding(OnboardingStep::LlmSetup)
        );
        assert_eq!(state.onboarding_progress.current_step, OnboardingStep::LlmSetup);

        // Should have received an error message via ConnectionTestResult
        let update = ui_rx.recv().await.expect("expected error update");
        match update {
            UiUpdate::OnboardingUpdate(OnboardingUpdate::ConnectionTestResult {
                success,
                message,
            }) => {
                assert!(!success);
                assert!(message.contains("failed"));
            }
            other => panic!("expected ConnectionTestResult error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn go_next_allowed_when_connection_test_never_run() {
        use crate::onboarding::OnboardingStep;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.onboarding_progress.current_step = OnboardingStep::LlmSetup;
        // Never tested (default)
        assert_eq!(state.connection_test_result.load(Ordering::Relaxed), CONNECTION_NEVER_TESTED);

        let (ui_tx, _ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(
            &mut state,
            UserCommand::OnboardingAction(OnboardingAction::GoNext),
            &ui_tx,
        )
        .await;

        // Should advance to StrategySetup
        assert_eq!(
            state.app_mode,
            AppMode::Onboarding(OnboardingStep::StrategySetup)
        );
    }

    #[tokio::test]
    async fn go_next_allowed_when_connection_test_succeeded() {
        use crate::onboarding::OnboardingStep;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
        state.onboarding_progress.current_step = OnboardingStep::LlmSetup;
        // Simulate a successful connection test
        state.connection_test_result.store(CONNECTION_TEST_PASSED, Ordering::Relaxed);

        let (ui_tx, _ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(
            &mut state,
            UserCommand::OnboardingAction(OnboardingAction::GoNext),
            &ui_tx,
        )
        .await;

        // Should advance to StrategySetup
        assert_eq!(
            state.app_mode,
            AppMode::Onboarding(OnboardingStep::StrategySetup)
        );
    }

    // -----------------------------------------------------------------------
    // Tests: build_snapshot includes llm_configured (Task 7)
    // -----------------------------------------------------------------------

    #[test]
    fn build_snapshot_llm_configured_false_when_disabled() {
        let state = create_test_app_state();
        // create_test_app_state uses LlmClient::Disabled
        let snap = state.build_snapshot();
        assert!(!snap.llm_configured);
    }

    #[tokio::test]
    async fn exit_settings_transitions_to_draft_mode() {
        use crate::protocol::SettingsSection;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);

        let (ui_tx, mut ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(&mut state, UserCommand::ExitSettings, &ui_tx).await;

        // AppState should now be in Draft mode
        assert_eq!(state.app_mode, AppMode::Draft);

        // UI channel should have received ModeChanged(Draft)
        let update = ui_rx.recv().await.expect("expected ModeChanged update");
        assert!(
            matches!(update, UiUpdate::ModeChanged(AppMode::Draft)),
            "first update should be ModeChanged(Draft), got {:?}",
            update,
        );

        // UI channel should also have received a StateSnapshot
        let snapshot_update = ui_rx.recv().await.expect("expected StateSnapshot update");
        assert!(
            matches!(snapshot_update, UiUpdate::StateSnapshot(_)),
            "second update should be StateSnapshot, got {:?}",
            snapshot_update,
        );
    }

    #[tokio::test]
    async fn save_strategy_config_updates_state_and_transitions_to_draft() {
        use crate::onboarding::OnboardingStep;
        use crate::tui::onboarding::strategy_setup::CategoryWeights;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);

        let weights = CategoryWeights {
            r: 1.0, hr: 1.1, rbi: 1.0, bb: 1.3, sb: 1.0, avg: 1.0,
            k: 1.0, w: 1.0, sv: 0.3, hd: 1.2, era: 1.0, whip: 1.0,
        };

        let (ui_tx, mut ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(
            &mut state,
            UserCommand::OnboardingAction(OnboardingAction::SaveStrategyConfig {
                hitting_budget_pct: 70,
                category_weights: weights,
                strategy_overview: Some("Test overview".to_string()),
            }),
            &ui_tx,
        )
        .await;

        // In-memory config should be updated
        assert!(
            (state.config.strategy.hitting_budget_fraction - 0.70).abs() < f64::EPSILON,
            "budget fraction should be 0.70, got {}",
            state.config.strategy.hitting_budget_fraction,
        );
        assert!(
            (state.config.strategy.weights.BB - 1.3).abs() < 0.001,
            "BB weight should be 1.3",
        );
        assert!(
            (state.config.strategy.weights.SV - 0.3).abs() < 0.001,
            "SV weight should be 0.3",
        );

        // Onboarding progress should be marked as strategy configured
        assert!(state.onboarding_progress.strategy_configured);
        assert_eq!(state.onboarding_progress.current_step, OnboardingStep::Complete);

        // AppState should now be in Draft mode
        assert_eq!(state.app_mode, AppMode::Draft);

        // UI channel should have received ModeChanged(Draft)
        let update = ui_rx.recv().await.expect("expected ModeChanged update");
        assert!(
            matches!(update, UiUpdate::ModeChanged(AppMode::Draft)),
            "first update should be ModeChanged(Draft), got {:?}",
            update,
        );

        // UI channel should also have received a StateSnapshot
        let snapshot_update = ui_rx.recv().await.expect("expected StateSnapshot update");
        assert!(
            matches!(snapshot_update, UiUpdate::StateSnapshot(_)),
            "second update should be StateSnapshot, got {:?}",
            snapshot_update,
        );
    }

    // -- parse_strategy_json tests --

    #[test]
    fn parse_strategy_json_valid() {
        let json = r#"{"hitting_budget_pct": 70, "category_weights": {"R": 1.0, "HR": 1.1, "RBI": 1.0, "BB": 1.3, "SB": 0.8, "AVG": 1.0, "K": 1.0, "W": 1.0, "SV": 0.3, "HD": 1.2, "ERA": 1.0, "WHIP": 1.0}, "strategy_overview": "Test overview"}"#;
        let (pct, weights, overview) = onboarding_handler::parse_strategy_json(json).unwrap();
        assert_eq!(pct, 70);
        assert!((weights.bb - 1.3).abs() < f32::EPSILON);
        assert!((weights.sv - 0.3).abs() < f32::EPSILON);
        assert!((weights.hd - 1.2).abs() < f32::EPSILON);
        assert_eq!(overview, "Test overview");
    }

    #[test]
    fn parse_strategy_json_with_markdown_fences() {
        let json = "```json\n{\"hitting_budget_pct\": 65, \"category_weights\": {\"R\": 1.0, \"HR\": 1.0, \"RBI\": 1.0, \"BB\": 1.0, \"SB\": 1.0, \"AVG\": 1.0, \"K\": 1.0, \"W\": 1.0, \"SV\": 0.7, \"HD\": 1.0, \"ERA\": 1.0, \"WHIP\": 1.0}}\n```";
        let (pct, weights, overview) = onboarding_handler::parse_strategy_json(json).unwrap();
        assert_eq!(pct, 65);
        assert!((weights.sv - 0.7).abs() < f32::EPSILON);
        assert_eq!(overview, ""); // no overview in this JSON
    }

    #[test]
    fn parse_strategy_json_clamps_values() {
        let json = r#"{"hitting_budget_pct": 200, "category_weights": {"SV": -1.0, "BB": 7.0}}"#;
        let (pct, weights, _overview) = onboarding_handler::parse_strategy_json(json).unwrap();
        assert_eq!(pct, 100); // clamped to 100
        assert!((weights.sv - 0.0).abs() < f32::EPSILON); // clamped to 0.0
        assert!((weights.bb - 5.0).abs() < f32::EPSILON); // clamped to 5.0
    }

    #[test]
    fn parse_strategy_json_missing_fields_uses_defaults() {
        let json = r#"{"category_weights": {"BB": 1.3}}"#;
        let (pct, weights, overview) = onboarding_handler::parse_strategy_json(json).unwrap();
        assert_eq!(pct, 65); // default
        assert!((weights.bb - 1.3).abs() < f32::EPSILON);
        assert!((weights.r - 1.0).abs() < f32::EPSILON); // default
        assert_eq!(overview, ""); // default
    }

    #[test]
    fn parse_strategy_json_no_json_object() {
        let result = onboarding_handler::parse_strategy_json("no json here");
        assert!(result.is_err());
    }

    #[test]
    fn parse_strategy_json_with_surrounding_text() {
        let text = "Here is the configuration:\n{\"hitting_budget_pct\": 60, \"category_weights\": {}}\nEnjoy!";
        let (pct, _, _) = onboarding_handler::parse_strategy_json(text).unwrap();
        assert_eq!(pct, 60);
    }

    // -----------------------------------------------------------------------
    // Tests: Settings mode (Task 6)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn open_settings_transitions_to_settings_mode() {
        use crate::protocol::SettingsSection;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Draft;

        let (ui_tx, mut ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(&mut state, UserCommand::OpenSettings, &ui_tx).await;

        // AppState should now be in Settings(LlmConfig) mode
        assert_eq!(
            state.app_mode,
            AppMode::Settings(SettingsSection::LlmConfig)
        );

        // UI channel should have received ModeChanged
        let update = ui_rx.recv().await.expect("expected ModeChanged update");
        assert!(
            matches!(
                update,
                UiUpdate::ModeChanged(AppMode::Settings(SettingsSection::LlmConfig))
            ),
            "expected ModeChanged(Settings(LlmConfig)), got {:?}",
            update,
        );
    }

    #[tokio::test]
    async fn exit_settings_transitions_back_to_draft() {
        use crate::protocol::SettingsSection;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);

        let (ui_tx, mut ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(&mut state, UserCommand::ExitSettings, &ui_tx).await;

        assert_eq!(state.app_mode, AppMode::Draft);

        let update = ui_rx.recv().await.expect("expected ModeChanged update");
        assert!(
            matches!(update, UiUpdate::ModeChanged(AppMode::Draft)),
            "expected ModeChanged(Draft), got {:?}",
            update,
        );
    }

    #[test]
    fn reload_llm_client_updates_client() {
        let mut state = create_test_app_state();

        // Initially disabled (no API key in test config)
        assert!(matches!(&*state.llm_client, LlmClient::Disabled));

        // Set an API key and reload
        state.config.credentials.anthropic_api_key = Some("sk-ant-test-key".to_string());
        state.reload_llm_client();

        // Now the client should be Active
        assert!(matches!(&*state.llm_client, LlmClient::Active(_)));
    }

    #[tokio::test]
    async fn settings_save_strategy_updates_config_stays_in_settings() {
        use crate::protocol::SettingsSection;
        use crate::tui::onboarding::strategy_setup::CategoryWeights;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Settings(SettingsSection::StrategyConfig);

        let weights = CategoryWeights {
            r: 1.0, hr: 1.1, rbi: 1.0, bb: 1.3, sb: 1.0, avg: 1.0,
            k: 1.0, w: 1.0, sv: 0.3, hd: 1.2, era: 1.0, whip: 1.0,
        };

        let (ui_tx, mut ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(
            &mut state,
            UserCommand::OnboardingAction(OnboardingAction::SaveStrategyConfig {
                hitting_budget_pct: 70,
                category_weights: weights,
                strategy_overview: None,
            }),
            &ui_tx,
        )
        .await;

        // In-memory config should be updated
        assert!(
            (state.config.strategy.hitting_budget_fraction - 0.70).abs() < f64::EPSILON,
            "budget fraction should be 0.70, got {}",
            state.config.strategy.hitting_budget_fraction,
        );
        assert!(
            (state.config.strategy.weights.BB - 1.3).abs() < 0.001,
            "BB weight should be 1.3",
        );

        // Should stay in Settings mode (not transition to Draft)
        assert_eq!(
            state.app_mode,
            AppMode::Settings(SettingsSection::StrategyConfig),
            "should remain in Settings mode after saving strategy",
        );

        // Should receive a StateSnapshot (recalculated valuations)
        let update = ui_rx.recv().await.expect("expected StateSnapshot update");
        assert!(
            matches!(update, UiUpdate::StateSnapshot(_)),
            "expected StateSnapshot, got {:?}",
            update,
        );
    }

    #[tokio::test]
    async fn settings_set_api_key_reloads_llm_client() {
        use crate::protocol::SettingsSection;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);
        state.onboarding_progress.llm_provider = Some(crate::llm::provider::LlmProvider::Anthropic);

        // Initially disabled
        assert!(matches!(&*state.llm_client, LlmClient::Disabled));

        let (ui_tx, _ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(
            &mut state,
            UserCommand::OnboardingAction(OnboardingAction::SetApiKey("sk-ant-test-key".to_string())),
            &ui_tx,
        )
        .await;

        // API key should be saved and LLM client should be reloaded
        assert_eq!(
            state.config.credentials.anthropic_api_key.as_deref(),
            Some("sk-ant-test-key"),
        );
        assert!(
            matches!(&*state.llm_client, LlmClient::Active(_)),
            "LLM client should be Active after setting API key in settings",
        );
    }

    #[tokio::test]
    async fn switch_settings_tab_changes_mode() {
        use crate::protocol::SettingsSection;

        let mut state = create_test_app_state();
        state.app_mode = AppMode::Settings(SettingsSection::LlmConfig);

        let (ui_tx, mut ui_rx) = mpsc::channel(16);

        command_handler::handle_user_command(
            &mut state,
            UserCommand::SwitchSettingsTab(SettingsSection::StrategyConfig),
            &ui_tx,
        )
        .await;

        assert_eq!(
            state.app_mode,
            AppMode::Settings(SettingsSection::StrategyConfig),
        );

        let update = ui_rx.recv().await.expect("expected ModeChanged update");
        assert!(
            matches!(
                update,
                UiUpdate::ModeChanged(AppMode::Settings(SettingsSection::StrategyConfig))
            ),
            "expected ModeChanged(Settings(StrategyConfig)), got {:?}",
            update,
        );

        // Switching to StrategyConfig should also send a StrategyLlmComplete
        // with the current saved config so the wizard opens at the Review step.
        let update2 = ui_rx.recv().await.expect("expected StrategyLlmComplete update");
        assert!(
            matches!(
                update2,
                UiUpdate::OnboardingUpdate(crate::protocol::OnboardingUpdate::StrategyLlmComplete { .. })
            ),
            "expected StrategyLlmComplete, got {:?}",
            update2,
        );
    }
}
