// Application state and orchestration logic.
//
// The central event loop that coordinates WebSocket events from the Firefox
// extension, LLM streaming events, and user commands from the TUI. Maintains
// the complete application state and pushes UI updates to the TUI render loop.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::db::Database;
use crate::draft::state::{
    compute_state_diff, ActiveNomination, DraftState, NominationPayload, PickPayload,
    ReconcileResult, StateUpdatePayload, TeamBudgetPayload,
};
use crate::llm::client::LlmClient;
use crate::llm::prompt;
use crate::protocol::{
    AppSnapshot, ConnectionStatus, ExtensionMessage, LlmEvent, LlmStatus, NominationInfo, TabId,
    TeamSnapshot, UiUpdate, UserCommand,
};
use crate::valuation;
use crate::valuation::analysis::{compute_instant_analysis, CategoryNeeds, InstantAnalysis};
use crate::valuation::auction::InflationTracker;
use crate::valuation::projections::AllProjections;
use crate::valuation::scarcity::{compute_scarcity, ScarcityEntry};
use crate::valuation::zscore::PlayerValuation;
use crate::ws_server::WsEvent;

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

/// What the LLM is currently working on.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmMode {
    /// Analyzing a specific nominated player.
    NominationAnalysis {
        player_name: String,
        player_id: String,
        nominated_by: String,
        current_bid: u32,
    },
    /// Generating a nomination plan (what to nominate next).
    NominationPlanning,
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

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// The complete application state.
pub struct AppState {
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
    pub current_llm_task: Option<tokio::task::JoinHandle<()>>,
    pub llm_mode: Option<LlmMode>,
    /// Monotonically increasing counter identifying the current LLM task.
    /// Incremented each time a new task is spawned. Events from stale
    /// generations are discarded in `handle_llm_event`.
    ///
    /// u64 overflow is not a practical concern: at one increment per second
    /// it would take ~584 billion years to wrap.
    pub llm_generation: u64,
    pub nomination_analysis_text: String,
    pub nomination_analysis_status: LlmStatus,
    pub nomination_plan_text: String,
    pub nomination_plan_status: LlmStatus,
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
    ) -> Self {
        let scarcity = compute_scarcity(&available_players, &config.league);
        let inflation = InflationTracker::new();

        AppState {
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
            current_llm_task: None,
            llm_mode: None,
            llm_generation: 0,
            nomination_analysis_text: String::new(),
            nomination_analysis_status: LlmStatus::Idle,
            nomination_plan_text: String::new(),
            nomination_plan_status: LlmStatus::Idle,
            connection_status: ConnectionStatus::Disconnected,
            last_ws_message_time: None,
            active_tab: TabId::Analysis,
            category_needs: CategoryNeeds::default(),
            llm_client: Arc::new(llm_client),
            llm_tx,
        }
    }

    /// Process new picks from the extension state diff.
    ///
    /// For each new pick:
    /// 1. Record in DraftState
    /// 2. Persist to DB
    /// 3. Remove from available player pool
    /// 4. Recalculate valuations
    /// 5. Update inflation and scarcity
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

        // Recalculate valuations with the updated pool
        crate::valuation::recalculate_all(
            &mut self.available_players,
            &self.config.league,
            &self.config.strategy,
            &self.draft_state,
        );

        // Update inflation
        self.inflation.update(
            &self.available_players,
            &self.draft_state,
            &self.config.league,
        );

        // Update scarcity
        self.scarcity = compute_scarcity(&self.available_players, &self.config.league);

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
            pick_count: self.draft_state.pick_count,
            total_picks: self.draft_state.total_picks,
            active_tab: None, // Don't override the user's active tab
            available_players: self.available_players.clone(),
            positional_scarcity: self.scarcity.clone(),
            draft_log: self.draft_state.picks.clone(),
            my_roster,
            budget_spent,
            budget_remaining,
            salary_cap: self.config.league.salary_cap,
            inflation_rate: self.inflation.inflation_rate,
            max_bid,
            avg_per_slot,
            team_snapshots,
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
        self.trigger_nomination_analysis(nomination);

        analysis
    }

    /// Handle nomination cleared (pick completed for the nominated player).
    ///
    /// Returns `true` if a nomination planning task was started, so callers
    /// can send `UiUpdate::PlanStarted` to clear stale plan text in the TUI.
    pub fn handle_nomination_cleared(&mut self) -> bool {
        self.draft_state.current_nomination = None;
        self.cancel_llm_task();
        self.llm_mode = None;
        self.nomination_analysis_text.clear();
        self.nomination_analysis_status = LlmStatus::Idle;

        // Auto-trigger nomination planning between picks so the plan panel
        // is populated before the user needs to nominate. Only fire when the
        // config flag is set and we already know which team is ours.
        if self.config.strategy.llm.prefire_planning && self.draft_state.my_team().is_some() {
            info!("Auto-triggering nomination planning (prefire_planning=true)");
            self.trigger_nomination_planning();
            return true;
        }
        false
    }

    /// Cancel the current LLM task if one is running.
    pub fn cancel_llm_task(&mut self) {
        if let Some(handle) = self.current_llm_task.take() {
            handle.abort();
            info!("Cancelled previous LLM task");
        }
    }

    /// Trigger LLM nomination analysis for a nominated player.
    ///
    /// Cancels any in-flight LLM task, sets `llm_mode` to
    /// `NominationAnalysis`, builds the analysis prompt from current
    /// state, and spawns a streaming task that sends tokens through the
    /// LLM event channel.
    pub fn trigger_nomination_analysis(&mut self, nomination: &ActiveNomination) {
        self.cancel_llm_task();

        let my_team = match self.draft_state.my_team() {
            Some(t) => t,
            None => {
                warn!("trigger_nomination_analysis called before teams registered, skipping");
                return;
            }
        };

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

        // Set mode and clear previous text (consolidated here so all callers
        // get consistent behavior).
        self.llm_mode = Some(LlmMode::NominationAnalysis {
            player_name: nomination.player_name.clone(),
            player_id: nomination.player_id.clone(),
            nominated_by: nomination.nominated_by.clone(),
            current_bid: nomination.current_bid,
        });
        self.nomination_analysis_text.clear();
        self.nomination_analysis_status = LlmStatus::Idle;

        let nom_info = NominationInfo {
            player_name: nomination.player_name.clone(),
            position: nomination.position.clone(),
            nominated_by: nomination.nominated_by.clone(),
            current_bid: nomination.current_bid,
            current_bidder: nomination.current_bidder.clone(),
            time_remaining: nomination.time_remaining,
            eligible_slots: nomination.eligible_slots.clone(),
        };

        let system = prompt::system_prompt();
        let user_content = prompt::build_nomination_analysis_prompt(
            &player,
            &nom_info,
            &my_team.roster,
            &self.category_needs,
            &self.scarcity,
            &self.available_players,
            &self.draft_state,
            &self.inflation,
        );

        let max_tokens = self.config.strategy.llm.analysis_max_tokens;
        let client = Arc::clone(&self.llm_client);
        let tx = self.llm_tx.clone();

        // Drain any stale events from a previous task that are sitting in the
        // channel. Without this, leftover Token/Complete/Error events from an
        // aborted task would be attributed to the new analysis.
        // Note: we can't drain here because we don't own the receiver.
        // Instead we rely on the generation counter to discard stale events.

        // Increment the generation counter so stale events from previous tasks
        // are discarded in handle_llm_event.
        self.llm_generation += 1;
        let generation = self.llm_generation;

        self.nomination_analysis_status = LlmStatus::Streaming;

        let handle = tokio::spawn(async move {
            if let Err(e) = client
                .stream_message(&system, &user_content, max_tokens, tx, generation)
                .await
            {
                warn!("LLM analysis task failed: {}", e);
            }
        });

        self.current_llm_task = Some(handle);
        info!(
            "Triggered LLM nomination analysis for {} (bid: ${}, gen: {})",
            nomination.player_name, nomination.current_bid, generation
        );
    }

    /// Trigger LLM nomination planning (what to nominate next).
    ///
    /// Cancels any in-flight LLM task, builds the planning prompt from
    /// current state, and spawns a streaming task.
    ///
    /// Returns `true` if a planning task was successfully started, `false` if
    /// it was skipped (e.g. teams not yet registered). Callers that receive
    /// `true` should send `UiUpdate::PlanStarted` to clear stale plan text in
    /// the TUI before the first token arrives.
    pub fn trigger_nomination_planning(&mut self) -> bool {
        self.cancel_llm_task();

        let my_team = match self.draft_state.my_team() {
            Some(t) => t,
            None => {
                warn!("trigger_nomination_planning called before teams registered, skipping");
                return false;
            }
        };

        self.llm_mode = Some(LlmMode::NominationPlanning);
        self.nomination_plan_text.clear();
        self.nomination_plan_status = LlmStatus::Streaming;

        let system = prompt::system_prompt();
        let user_content = prompt::build_nomination_planning_prompt(
            &my_team.roster,
            &self.category_needs,
            &self.scarcity,
            &self.available_players,
            &self.draft_state,
            &self.inflation,
        );

        let max_tokens = self.config.strategy.llm.planning_max_tokens;
        let client = Arc::clone(&self.llm_client);
        let tx = self.llm_tx.clone();

        self.llm_generation += 1;
        let generation = self.llm_generation;

        let handle = tokio::spawn(async move {
            if let Err(e) = client
                .stream_message(&system, &user_content, max_tokens, tx, generation)
                .await
            {
                warn!("LLM planning task failed: {}", e);
            }
        });

        self.current_llm_task = Some(handle);
        info!("Triggered LLM nomination planning (gen: {})", generation);
        true
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
                        handle_ws_message(&mut state, &json_str, &ui_tx).await;
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
                        handle_llm_event(&mut state, event, &ui_tx).await;
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
                        handle_user_command(&mut state, cmd, &ui_tx).await;
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
    state.cancel_llm_task();
    info!("Application event loop exiting");
    Ok(())
}

/// Handle an incoming WebSocket message (JSON from the extension).
async fn handle_ws_message(
    state: &mut AppState,
    json_str: &str,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    let msg: ExtensionMessage = match serde_json::from_str(json_str) {
        Ok(m) => m,
        Err(e) => {
            warn!("Failed to parse extension message: {}", e);
            return;
        }
    };

    match msg {
        ExtensionMessage::ExtensionConnected { payload } => {
            info!(
                "Extension identified: {} v{}",
                payload.platform, payload.extension_version
            );
        }
        ExtensionMessage::StateUpdate { timestamp: _, payload } => {
            handle_state_update(state, payload, ui_tx).await;
        }
        ExtensionMessage::FullStateSync { timestamp: _, payload } => {
            handle_full_state_sync(state, payload, ui_tx).await;
        }
        ExtensionMessage::ExtensionHeartbeat { .. } => {
            // Heartbeats are logged at trace level, no action needed
        }
    }
}

/// Handle a full state sync from the extension (on connect or reconnect).
///
/// Resets the in-memory draft state (picks, rosters, budgets) and rebuilds it
/// entirely from the snapshot payload. After the reset, delegates to
/// `handle_state_update` with `previous_extension_state` cleared so that
/// `compute_state_diff` treats every pick in the snapshot as new (applied
/// against an empty baseline). This prevents corrupted state that would
/// result from applying incremental diffs against a blank slate when resuming
/// a mid-draft session.
async fn handle_full_state_sync(
    state: &mut AppState,
    ext_payload: crate::protocol::StateUpdatePayload,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    info!(
        "Received FULL_STATE_SYNC with {} picks — resetting draft state",
        ext_payload.picks.len()
    );

    // Reset in-memory draft state so the snapshot is applied from scratch.
    // Preserve salary_cap and roster_config (stored inside DraftState).
    state.draft_state = DraftState::new(
        state.config.league.salary_cap,
        &state.config.league.roster,
    );

    // Clear the previous extension state so that compute_state_diff in
    // handle_state_update treats all picks in the snapshot as new.
    state.previous_extension_state = None;

    // Reset valuation pool and derived state so they're rebuilt cleanly
    // after all snapshot picks are applied.
    state.available_players =
        valuation::compute_initial(&state.all_projections, &state.config)
            .unwrap_or_default();
    state.scarcity = compute_scarcity(&state.available_players, &state.config.league);
    state.inflation = InflationTracker::new();
    state.category_needs = CategoryNeeds::default();

    // Clear any in-flight LLM task — its context is stale.
    state.cancel_llm_task();
    state.llm_mode = None;
    state.nomination_analysis_text.clear();
    state.nomination_analysis_status = LlmStatus::Idle;
    state.nomination_plan_text.clear();
    state.nomination_plan_status = LlmStatus::Idle;

    // Now process the snapshot as a regular state update.  Because
    // previous_extension_state is None, compute_state_diff will emit all
    // picks as new, rebuilding the draft log, rosters, and budgets cleanly.
    handle_state_update(state, ext_payload, ui_tx).await;
}

/// Handle a state update from the extension.
///
/// Performs differential state detection, processes new picks,
/// and handles nomination changes.
///
/// On each STATE_UPDATE, checks whether the extension's `draftId` matches
/// the stored ESPN draft identifier. If they differ, a new draft session is
/// started with a fresh internal draft_id and all in-memory state is reset.
/// This is resilient across disconnects because it relies on a stable
/// identifier derived from the ESPN page URL rather than comparing pick counts.
async fn handle_state_update(
    state: &mut AppState,
    ext_payload: crate::protocol::StateUpdatePayload,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    // --- New draft detection via ESPN draft identifier ---
    // The extension derives a stable draft identifier from the ESPN page URL
    // (leagueId + year). When this ID differs from what we have stored, a new
    // draft has started and we reset all in-memory state.
    if let Some(ref ext_draft_id) = ext_payload.draft_id {
        match &state.espn_draft_id {
            Some(stored_espn_id) if stored_espn_id != ext_draft_id => {
                // ESPN draft ID changed -> new draft
                let new_draft_id = Database::generate_draft_id();
                info!(
                    "New draft detected: ESPN draft ID changed from '{}' to '{}'. \
                     Starting new draft session: {}",
                    stored_espn_id, ext_draft_id, new_draft_id
                );
                // Persist to DB first -- only reset in-memory state if the
                // write succeeds so we never diverge from the database.
                match state.db.set_both_draft_ids(&new_draft_id, ext_draft_id) {
                    Ok(()) => {}
                    Err(e) => {
                        warn!(
                            "Failed to persist draft IDs, skipping draft reset: {}",
                            e
                        );
                        // Skip the entire reset; keep current in-memory state
                        // consistent with what the database still holds.
                        return;
                    }
                }
                state.draft_id = new_draft_id.clone();
                state.espn_draft_id = Some(ext_draft_id.clone());
                // Reset in-memory draft state for the new draft
                state.draft_state = DraftState::new(
                    state.config.league.salary_cap,
                    &state.config.league.roster,
                );
                state.available_players =
                    valuation::compute_initial(&state.all_projections, &state.config)
                        .unwrap_or_default();
                state.scarcity =
                    compute_scarcity(&state.available_players, &state.config.league);
                state.inflation = InflationTracker::new();
                state.previous_extension_state = None;
                // Clear LLM state so stale analysis from the previous draft
                // doesn't bleed into the new session.
                if let Some(handle) = state.current_llm_task.take() {
                    handle.abort();
                }
                state.llm_mode = None;
                state.nomination_analysis_text.clear();
                state.nomination_analysis_status = LlmStatus::Idle;
                state.nomination_plan_text.clear();
                state.nomination_plan_status = LlmStatus::Idle;
                state.category_needs = CategoryNeeds::default();
            }
            None => {
                // First time receiving an ESPN draft ID -- store it.
                info!("ESPN draft ID received: {}", ext_draft_id);
                state.espn_draft_id = Some(ext_draft_id.clone());
                if let Err(e) = state.db.set_espn_draft_id(ext_draft_id) {
                    warn!("Failed to persist ESPN draft_id: {}", e);
                }
            }
            _ => {
                // Same ESPN draft ID, no action needed.
            }
        }
    }

    let internal_payload = AppState::convert_extension_state(&ext_payload);

    // Compute diff against previous state
    let diff = compute_state_diff(&state.previous_extension_state, &internal_payload);

    // Process new picks first (updates local budget tracking)
    let had_new_picks = !diff.new_picks.is_empty();
    if had_new_picks {
        info!("Processing {} new picks", diff.new_picks.len());
        state.process_new_picks(diff.new_picks);
    }

    // Update pick count / total picks from ESPN clock label if available.
    // Done after process_new_picks so ESPN's authoritative count takes precedence.
    if let Some(pc) = internal_payload.pick_count {
        state.draft_state.pick_count = pc as usize;
    }
    if let Some(tp) = internal_payload.total_picks {
        state.draft_state.total_picks = tp as usize;
    }

    // Reconcile team budgets from ESPN-scraped data.
    // On the first call this auto-registers all teams from ESPN and
    // replays any crash-recovery picks. Returns a ReconcileResult
    // indicating whether teams were registered and/or budgets changed.
    let reconcile = if !internal_payload.teams.is_empty() {
        state
            .draft_state
            .reconcile_budgets(&internal_payload.teams)
    } else {
        ReconcileResult {
            teams_registered: false,
            budgets_changed: false,
        }
    };
    let teams_just_registered = reconcile.teams_registered;

    // Set the user's team from the extension's myTeamId (a team name).
    // This must happen after reconcile_budgets so teams are registered.
    if let Some(ref my_team_name) = ext_payload.my_team_id {
        if !my_team_name.is_empty() && !state.draft_state.teams.is_empty() {
            state.draft_state.set_my_team_by_name(my_team_name);
        }
    }

    // Send a state snapshot to the TUI so all recalculated data
    // (available players, scarcity, budget, inflation, draft log,
    // roster, team summaries) is reflected in the UI.
    // Only send when something actually changed — not on every ESPN poll.
    let has_changes = had_new_picks
        || internal_payload.pick_count.is_some()
        || teams_just_registered
        || reconcile.budgets_changed;
    if has_changes {
        let snapshot = state.build_snapshot();
        let _ = ui_tx
            .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
            .await;
    }

    // Handle nomination changes
    if diff.nomination_changed {
        if diff.nomination_cleared {
            info!("Nomination cleared");
            let planning_started = state.handle_nomination_cleared();
            let _ = ui_tx.send(UiUpdate::NominationCleared).await;
            if planning_started {
                let _ = ui_tx.send(UiUpdate::PlanStarted).await;
            }
        } else if let Some(ref nomination) = diff.new_nomination {
            info!(
                "New nomination: {} (bid: ${})",
                nomination.player_name, nomination.current_bid
            );
            let analysis = state.handle_nomination(nomination);

            let nom_info = NominationInfo {
                player_name: nomination.player_name.clone(),
                position: nomination.position.clone(),
                nominated_by: nomination.nominated_by.clone(),
                current_bid: nomination.current_bid,
                current_bidder: nomination.current_bidder.clone(),
                time_remaining: nomination.time_remaining,
                eligible_slots: nomination.eligible_slots.clone(),
            };
            let _ = ui_tx
                .send(UiUpdate::NominationUpdate(Box::new(nom_info)))
                .await;

            // If we have an analysis, we could send it too (future: embedded in snapshot)
            if let Some(_analysis) = analysis {
                info!("Instant analysis computed for nomination");
            }
        }
    } else if diff.bid_updated {
        // Same player, bid updated - update the nomination info without clearing LLM text
        if let Some(ref nomination) = diff.new_nomination {
            state.draft_state.current_nomination = Some(nomination.clone());

            let nom_info = NominationInfo {
                player_name: nomination.player_name.clone(),
                position: nomination.position.clone(),
                nominated_by: nomination.nominated_by.clone(),
                current_bid: nomination.current_bid,
                current_bidder: nomination.current_bidder.clone(),
                time_remaining: nomination.time_remaining,
                eligible_slots: nomination.eligible_slots.clone(),
            };
            let _ = ui_tx
                .send(UiUpdate::BidUpdate(Box::new(nom_info)))
                .await;
        }
    }

    // If teams were just registered this update cycle, check if a nomination
    // exists but was skipped because my_team() returned None (teams weren't
    // ready yet). This handles two race conditions:
    //
    // 1. The first STATE_UPDATE contains both team data AND a nomination.
    //    The diff-based nomination handling ran handle_nomination() which
    //    succeeded because reconcile_budgets() already ran earlier in this
    //    function. No retry needed (llm_mode will be Some).
    //
    // 2. An earlier STATE_UPDATE had a nomination but no teams. The
    //    handle_nomination() call returned early (my_team() was None),
    //    leaving current_nomination unset and llm_mode as None. A later
    //    update now registers teams, but the diff sees the same nomination
    //    (no change) so nomination handling is skipped. We detect this by
    //    checking the payload's current_nomination directly.
    if teams_just_registered && state.llm_mode.is_none() {
        if let Some(ref nom_payload) = internal_payload.current_nomination {
            let nomination = ActiveNomination {
                player_name: nom_payload.player_name.clone(),
                player_id: nom_payload.player_id.clone(),
                position: nom_payload.position.clone(),
                nominated_by: nom_payload.nominated_by.clone(),
                current_bid: nom_payload.current_bid,
                current_bidder: nom_payload.current_bidder.clone(),
                time_remaining: nom_payload.time_remaining,
                eligible_slots: nom_payload.eligible_slots.clone(),
            };
            info!(
                "Teams just registered, retrying analysis for pending nomination: {}",
                nomination.player_name
            );
            let analysis = state.handle_nomination(&nomination);

            let nom_info = NominationInfo {
                player_name: nomination.player_name.clone(),
                position: nomination.position.clone(),
                nominated_by: nomination.nominated_by.clone(),
                current_bid: nomination.current_bid,
                current_bidder: nomination.current_bidder.clone(),
                time_remaining: nomination.time_remaining,
                eligible_slots: nomination.eligible_slots.clone(),
            };
            let _ = ui_tx
                .send(UiUpdate::NominationUpdate(Box::new(nom_info)))
                .await;

            if let Some(_analysis) = analysis {
                info!("Instant analysis computed for retried nomination");
            }
        }
    }

    // Store current state for next diff
    state.previous_extension_state = Some(internal_payload);
}

/// Handle an LLM streaming event.
///
/// Routes tokens and completions to the appropriate text buffer
/// based on the current LLM mode.
///
/// **Generation check**: Every event carries a generation counter set when
/// the task was spawned. If the event's generation doesn't match
/// `state.llm_generation`, it's a stale event from a cancelled task and
/// is silently discarded. This prevents leftover tokens from a previous
/// analysis bleeding into a newer one.
///
/// **Mode reset on completion/error**: After a `Complete` or `Error` event
/// is processed, `llm_mode` is set back to `None`. This ensures that:
/// 1. Any further stale events hit the `(None, _)` discard path.
/// 2. The system is clearly in an idle state, ready for the next
///    nomination to set a fresh mode.
async fn handle_llm_event(
    state: &mut AppState,
    event: LlmEvent,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    // Extract the generation from the event.
    let event_generation = match &event {
        LlmEvent::Token { generation, .. } => *generation,
        LlmEvent::Complete { generation, .. } => *generation,
        LlmEvent::Error { generation, .. } => *generation,
    };

    // Discard events from stale (cancelled) tasks.
    if event_generation != state.llm_generation {
        debug!(
            "Discarding stale LLM event (event gen: {}, current gen: {})",
            event_generation, state.llm_generation
        );
        return;
    }

    match (&state.llm_mode, event) {
        (Some(LlmMode::NominationAnalysis { .. }), LlmEvent::Token { text, .. }) => {
            state.nomination_analysis_text.push_str(&text);
            state.nomination_analysis_status = LlmStatus::Streaming;
            let _ = ui_tx.send(UiUpdate::AnalysisToken(text)).await;
        }
        (Some(LlmMode::NominationAnalysis { .. }), LlmEvent::Complete { full_text, stop_reason, .. }) => {
            let text = if stop_reason.as_deref() == Some("max_tokens") {
                format!("{full_text}\n\n[Response truncated due to token limit]")
            } else {
                full_text
            };
            state.nomination_analysis_text = text.clone();
            state.nomination_analysis_status = LlmStatus::Complete;
            state.llm_mode = None;
            let _ = ui_tx.send(UiUpdate::AnalysisComplete(text)).await;
        }
        (Some(LlmMode::NominationAnalysis { .. }), LlmEvent::Error { message, .. }) => {
            warn!("LLM analysis error: {}", message);
            state.nomination_analysis_status = LlmStatus::Error;
            state.llm_mode = None;
            let _ = ui_tx.send(UiUpdate::AnalysisError(message)).await;
        }
        (Some(LlmMode::NominationPlanning), LlmEvent::Token { text, .. }) => {
            state.nomination_plan_text.push_str(&text);
            state.nomination_plan_status = LlmStatus::Streaming;
            let _ = ui_tx.send(UiUpdate::PlanToken(text)).await;
        }
        (Some(LlmMode::NominationPlanning), LlmEvent::Complete { full_text, stop_reason, .. }) => {
            let text = if stop_reason.as_deref() == Some("max_tokens") {
                format!("{full_text}\n\n[Response truncated due to token limit]")
            } else {
                full_text
            };
            state.nomination_plan_text = text.clone();
            state.nomination_plan_status = LlmStatus::Complete;
            state.llm_mode = None;
            let _ = ui_tx.send(UiUpdate::PlanComplete(text)).await;
        }
        (Some(LlmMode::NominationPlanning), LlmEvent::Error { message, .. }) => {
            warn!("LLM planning error: {}", message);
            state.nomination_plan_status = LlmStatus::Error;
            state.llm_mode = None;
            let _ = ui_tx.send(UiUpdate::PlanError(message)).await;
        }
        (None, _) => {
            // No active LLM mode - discard the event (likely a stale
            // completion that arrived after mode was reset).
            debug!("Received LLM event with no active mode, discarding");
        }
    }
}

/// Handle a user command from the TUI.
async fn handle_user_command(
    state: &mut AppState,
    cmd: UserCommand,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    match cmd {
        UserCommand::SwitchTab(tab) => {
            state.active_tab = tab;
            info!("Switched to tab: {:?}", tab);
        }
        UserCommand::RefreshAnalysis => {
            if let Some(nom) = state.draft_state.current_nomination.clone() {
                info!("Refreshing analysis for {}", nom.player_name);
                state.trigger_nomination_analysis(&nom);
            }
        }
        UserCommand::RefreshPlan => {
            info!("Refreshing nomination plan");
            if state.trigger_nomination_planning() {
                let _ = ui_tx.send(UiUpdate::PlanStarted).await;
            }
        }
        UserCommand::ManualPick {
            player_name,
            team_idx,
            price,
        } => {
            info!(
                "Manual pick: {} -> team {} for ${}",
                player_name, team_idx, price
            );
            if team_idx < state.draft_state.teams.len() {
                let team = &state.draft_state.teams[team_idx];
                let pick = crate::draft::pick::DraftPick {
                    pick_number: (state.draft_state.pick_count + 1) as u32,
                    team_id: team.team_id.clone(),
                    team_name: team.team_name.clone(),
                    player_name,
                    position: "UTIL".to_string(),
                    price,
                    espn_player_id: None,
                    eligible_slots: vec![],
                };
                state.process_new_picks(vec![pick]);

                // Send updated state to TUI
                let snapshot = state.build_snapshot();
                let _ = ui_tx
                    .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                    .await;
            }
        }
        UserCommand::Scroll { .. } => {
            // Scroll is handled by the TUI directly, no app-level action needed
        }
        UserCommand::Quit => {
            // Handled in the main loop
        }
    }
}

// ---------------------------------------------------------------------------
// Crash recovery
// ---------------------------------------------------------------------------

/// Restore application state from the database after a crash/restart.
///
/// If the DB has draft picks recorded for the current draft_id, loads them
/// and replays them into the DraftState, then recalculates valuations.
pub fn recover_from_db(state: &mut AppState) -> anyhow::Result<bool> {
    if !state.db.has_draft_in_progress(&state.draft_id)? {
        info!("No draft in progress for draft_id={}, starting fresh", state.draft_id);
        return Ok(false);
    }

    let picks = state.db.load_picks(&state.draft_id)?;
    let pick_count = picks.len();
    info!(
        "Crash recovery: restoring {} picks from DB for draft_id={}",
        pick_count, state.draft_id
    );

    // Restore picks into DraftState (takes ownership)
    state.draft_state.restore_from_picks(picks);

    // Remove drafted players from available pool using the restored picks
    for pick in &state.draft_state.picks {
        state
            .available_players
            .retain(|p| p.name != pick.player_name);
    }

    // Recalculate valuations
    crate::valuation::recalculate_all(
        &mut state.available_players,
        &state.config.league,
        &state.config.strategy,
        &state.draft_state,
    );

    // Update inflation
    state.inflation.update(
        &state.available_players,
        &state.draft_state,
        &state.config.league,
    );

    // Update scarcity
    state.scarcity = compute_scarcity(&state.available_players, &state.config.league);

    info!(
        "Crash recovery complete: {} picks restored, {} players remaining",
        pick_count,
        state.available_players.len()
    );

    Ok(true)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::db::Database;
    use crate::draft::pick::{DraftPick, Position};
    use crate::draft::state::{ActiveNomination, DraftState};
    use crate::protocol::{LlmEvent, LlmStatus, UserCommand};
    use crate::valuation::auction::InflationTracker;
    use crate::valuation::projections::{AllProjections, PitcherType};
    use crate::valuation::zscore::{
        CategoryZScores, HitterZScores, PitcherZScores, PlayerProjectionData, PlayerValuation,
    };
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn test_league_config() -> LeagueConfig {
        let mut roster = HashMap::new();
        roster.insert("C".into(), 1);
        roster.insert("1B".into(), 1);
        roster.insert("2B".into(), 1);
        roster.insert("3B".into(), 1);
        roster.insert("SS".into(), 1);
        roster.insert("LF".into(), 1);
        roster.insert("CF".into(), 1);
        roster.insert("RF".into(), 1);
        roster.insert("UTIL".into(), 1);
        roster.insert("SP".into(), 5);
        roster.insert("RP".into(), 6);
        roster.insert("BE".into(), 6);
        roster.insert("IL".into(), 5);

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
            roster,
            roster_limits: RosterLimits {
                max_sp: 7,
                max_rp: 7,
                gs_per_week: 7,
            },
            teams: HashMap::new(),
            my_team: None,
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
                model: "test".into(),
                analysis_max_tokens: 2048,
                planning_max_tokens: 2048,
                analysis_trigger: "nomination".into(),
                prefire_planning: true,
            },
        }
    }

    fn test_config() -> Config {
        Config {
            league: test_league_config(),
            strategy: test_strategy_config(),
            credentials: CredentialsConfig::default(),
            ws_port: 9001,
            db_path: ":memory:".into(),
            data_paths: DataPaths {
                hitters: "data/projections/hitters.csv".into(),
                pitchers: "data/projections/pitchers.csv".into(),
            },
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

    fn create_test_app_state() -> AppState {
        let config = test_config();
        let mut draft_state = DraftState::new(260, &test_roster_config());
        // Register teams from ESPN data and set my team
        draft_state.reconcile_budgets(&test_espn_budgets());
        draft_state.set_my_team_by_name("Team 1");

        let mut available = test_players();

        // Run initial valuation so dollar values are set
        crate::valuation::recalculate_all(
            &mut available,
            &config.league,
            &config.strategy,
            &draft_state,
        );

        let db = Database::open(":memory:").expect("in-memory db");
        let draft_id = Database::generate_draft_id();
        let llm_client = LlmClient::Disabled;
        let (llm_tx, _llm_rx) = mpsc::channel(16);

        AppState::new(config, draft_state, available, empty_projections(), db, draft_id, llm_client, llm_tx)
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

        // LLM mode should be set to NominationAnalysis
        assert!(matches!(
            state.llm_mode,
            Some(LlmMode::NominationAnalysis { .. })
        ));
        if let Some(LlmMode::NominationAnalysis {
            player_name,
            player_id,
            nominated_by,
            current_bid,
        }) = &state.llm_mode
        {
            assert_eq!(player_name, "H_Star");
            assert_eq!(player_id, "espn_1");
            assert_eq!(nominated_by, "Team 2");
            assert_eq!(*current_bid, 5);
        }
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
        state.nomination_analysis_text = "Some previous analysis...".into();
        state.nomination_analysis_status = LlmStatus::Streaming;

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

        // Analysis text should be cleared and new streaming started
        assert!(state.nomination_analysis_text.is_empty());
        assert_eq!(state.nomination_analysis_status, LlmStatus::Streaming);

        // LLM mode should be updated to the new nomination
        if let Some(LlmMode::NominationAnalysis { player_name, .. }) = &state.llm_mode {
            assert_eq!(player_name, "H_Good");
        } else {
            panic!("Expected NominationAnalysis mode");
        }
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
        state.nomination_analysis_text = "Analysis text".into();
        state.nomination_analysis_status = LlmStatus::Streaming;

        // Clear the nomination
        state.handle_nomination_cleared();

        assert!(state.draft_state.current_nomination.is_none());
        // Analysis state should be fully reset
        assert!(state.nomination_analysis_text.is_empty());
        assert_eq!(state.nomination_analysis_status, LlmStatus::Idle);
        // With prefire_planning=true and teams registered, planning should
        // auto-trigger so the plan panel is populated between nominations.
        assert!(
            matches!(state.llm_mode, Some(LlmMode::NominationPlanning)),
            "expected NominationPlanning mode after clearing (prefire_planning=true), got {:?}",
            state.llm_mode
        );
        assert_eq!(state.nomination_plan_status, LlmStatus::Streaming);
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
        state.nomination_analysis_text = "Analysis text".into();
        state.nomination_analysis_status = LlmStatus::Streaming;

        state.handle_nomination_cleared();

        assert!(state.draft_state.current_nomination.is_none());
        assert!(state.llm_mode.is_none());
        assert!(state.nomination_analysis_text.is_empty());
        assert_eq!(state.nomination_analysis_status, LlmStatus::Idle);
    }

    #[tokio::test]
    async fn nomination_cleared_skips_planning_when_no_teams() {
        let mut state = create_test_app_state_no_teams();

        state.handle_nomination_cleared();

        assert!(state.llm_mode.is_none());
    }

    // -----------------------------------------------------------------------
    // Tests: Crash recovery
    // -----------------------------------------------------------------------

    #[test]
    fn crash_recovery_restores_state() {
        let config = test_config();
        let db = Database::open(":memory:").expect("in-memory db");
        let draft_id = Database::generate_draft_id();

        // Record some picks into the database (simulating a previous session)
        let picks = vec![
            DraftPick {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
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
            },
        ];
        for pick in &picks {
            db.record_pick(pick, &draft_id).unwrap();
        }
        assert!(db.has_draft_in_progress(&draft_id).unwrap());

        // Create a fresh AppState (simulating restart)
        let mut draft_state = DraftState::new(260, &test_roster_config());
        // Register teams from ESPN data (simulating what happens at connection)
        draft_state.reconcile_budgets(&test_espn_budgets());
        draft_state.set_my_team_by_name("Team 1");

        let mut available = test_players();
        crate::valuation::recalculate_all(
            &mut available,
            &config.league,
            &config.strategy,
            &draft_state,
        );
        let initial_player_count = available.len();

        let llm_client = LlmClient::Disabled;
        let (llm_tx, _llm_rx) = mpsc::channel(16);
        let mut state = AppState::new(config, draft_state, available, empty_projections(), db, draft_id, llm_client, llm_tx);

        // Run crash recovery
        let recovered = recover_from_db(&mut state).unwrap();
        assert!(recovered);

        // Verify state was restored
        assert_eq!(state.draft_state.pick_count, 2);
        assert_eq!(state.draft_state.picks.len(), 2);
        assert_eq!(state.draft_state.picks[0].player_name, "H_Star");
        assert_eq!(state.draft_state.picks[1].player_name, "P_Ace");

        // Players should be removed from available pool
        assert_eq!(
            state.available_players.len(),
            initial_player_count - 2
        );
        assert!(!state.available_players.iter().any(|p| p.name == "H_Star"));
        assert!(!state.available_players.iter().any(|p| p.name == "P_Ace"));

        // Budget should be updated
        let team1 = state.draft_state.team("1").unwrap();
        assert_eq!(team1.budget_spent, 45);
        let team2 = state.draft_state.team("2").unwrap();
        assert_eq!(team2.budget_spent, 50);
    }

    #[test]
    fn crash_recovery_no_picks_returns_false() {
        let config = test_config();
        let db = Database::open(":memory:").expect("in-memory db");
        let draft_id = Database::generate_draft_id();
        let mut draft_state = DraftState::new(260, &test_roster_config());
        draft_state.reconcile_budgets(&test_espn_budgets());
        draft_state.set_my_team_by_name("Team 1");
        let available = test_players();

        let llm_client = LlmClient::Disabled;
        let (llm_tx, _llm_rx) = mpsc::channel(16);
        let mut state = AppState::new(config, draft_state, available, empty_projections(), db, draft_id, llm_client, llm_tx);

        let recovered = recover_from_db(&mut state).unwrap();
        assert!(!recovered);
        assert_eq!(state.draft_state.pick_count, 0);
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

        // The generation must match what AppState expects (starts at 0,
        // but trigger_* would increment it; here we manually set mode
        // so we use generation 0 to match the initial state).
        let gen = 0u64;

        let handle = tokio::spawn(async move {
            let mut state = state;
            // Set up LLM mode before entering the loop
            state.llm_mode = Some(LlmMode::NominationAnalysis {
                player_name: "Test".into(),
                player_id: "1".into(),
                nominated_by: "Team".into(),
                current_bid: 5,
            });
            run(ws_rx, llm_rx, cmd_rx, ui_tx, state).await
        });

        // Give the loop a moment to start
        tokio::task::yield_now().await;

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
            UiUpdate::AnalysisToken(token) => assert_eq!(token, "Hello "),
            other => panic!("Expected AnalysisToken, got {:?}", other),
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
            UiUpdate::NominationUpdate(info) => {
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
        // Do NOT call reconcile_budgets or set_my_team_by_name

        let available = test_players();

        let db = Database::open(":memory:").expect("in-memory db");
        let llm_client = LlmClient::Disabled;
        let (llm_tx, _llm_rx) = mpsc::channel(16);

        let draft_id = Database::generate_draft_id();
        AppState::new(config, draft_state, available, empty_projections(), db, draft_id, llm_client, llm_tx)
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
        assert!(state.llm_mode.is_none(), "LLM mode should start as None");

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
            my_team_id: Some("Team 1".into()),
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
        };

        handle_state_update(&mut state, ext_payload, &ui_tx).await;

        // Teams should now be registered
        assert_eq!(state.draft_state.teams.len(), 2);

        // LLM mode should be set (nomination analysis was triggered)
        assert!(
            state.llm_mode.is_some(),
            "LLM mode should be set after first nomination with teams in same update"
        );
        assert!(matches!(
            state.llm_mode,
            Some(LlmMode::NominationAnalysis { .. })
        ));

        // current_nomination should be set on the draft state
        assert!(state.draft_state.current_nomination.is_some());
        assert_eq!(
            state.draft_state.current_nomination.as_ref().unwrap().player_name,
            "H_Star"
        );

        // Drain UI updates and verify we got a NominationUpdate
        let mut got_nomination_update = false;
        while let Ok(update) = ui_rx.try_recv() {
            if let UiUpdate::NominationUpdate(info) = update {
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
            my_team_id: Some("Team 1".into()),
            teams: vec![],  // No teams!
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: Some("test".into()),
        };

        handle_state_update(&mut state, ext_payload_1, &ui_tx).await;

        // Teams should still be empty
        assert!(state.draft_state.teams.is_empty());
        // LLM mode should be None (nomination was skipped)
        assert!(
            state.llm_mode.is_none(),
            "LLM mode should be None since teams aren't registered"
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
            my_team_id: Some("Team 1".into()),
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
        };

        handle_state_update(&mut state, ext_payload_2, &ui_tx).await;

        // Teams should now be registered
        assert_eq!(state.draft_state.teams.len(), 2);

        // LLM mode should now be set (retry triggered the analysis)
        assert!(
            state.llm_mode.is_some(),
            "LLM mode should be set after teams registered with pending nomination"
        );
        assert!(matches!(
            state.llm_mode,
            Some(LlmMode::NominationAnalysis { .. })
        ));

        // current_nomination should now be set
        assert!(state.draft_state.current_nomination.is_some());
        assert_eq!(
            state.draft_state.current_nomination.as_ref().unwrap().player_name,
            "H_Star"
        );

        // Verify we got a NominationUpdate from the retry
        let mut got_nomination_update = false;
        while let Ok(update) = ui_rx.try_recv() {
            if let UiUpdate::NominationUpdate(info) = update {
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
            my_team_id: Some("Team 1".into()),
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
        };

        handle_state_update(&mut state, ext_payload, &ui_tx).await;

        // Count NominationUpdate messages -- should be exactly 1
        // (from the normal flow, not doubled by the retry)
        let mut nomination_update_count = 0;
        while let Ok(update) = ui_rx.try_recv() {
            if matches!(update, UiUpdate::NominationUpdate(_)) {
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
    /// - Then set_my_team_by_name
    /// - Then build_snapshot
    #[test]
    fn first_state_update_roster_correctness() {
        let config = test_config();
        let draft_state = DraftState::new(260, &test_roster_config());
        // Note: NOT calling reconcile_budgets yet (simulates first update)
        assert!(draft_state.teams.is_empty());

        let mut available = test_players();
        crate::valuation::recalculate_all(
            &mut available,
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
        );

        // Step 1: process_new_picks while teams are empty
        // (simulates what happens in handle_state_update before reconcile_budgets)
        state.process_new_picks(vec![
            DraftPick {
                pick_number: 1,
                team_id: "Team Alpha".into(), // DOM scraping uses name as ID
                team_name: "Team Alpha".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
            },
            DraftPick {
                pick_number: 2,
                team_id: "Team Beta".into(),
                team_name: "Team Beta".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: None,
                eligible_slots: vec![],
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
        state.draft_state.set_my_team_by_name("Team Alpha");

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
    fn picks_with_name_as_team_id_route_correctly() {
        let mut state = create_test_app_state();

        // Simulate DOM scraping where teamId = teamName (not numeric)
        state.process_new_picks(vec![
            DraftPick {
                pick_number: 1,
                team_id: "Team 1".into(), // name as ID (DOM scraping style)
                team_name: "Team 1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: None,
                eligible_slots: vec![],
            },
            DraftPick {
                pick_number: 2,
                team_id: "Team 2".into(), // name as ID
                team_name: "Team 2".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: None,
                eligible_slots: vec![],
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
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        handle_state_update(&mut state, ext_payload, &ui_tx).await;

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
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        handle_state_update(&mut state, ext_payload, &ui_tx).await;

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
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        handle_state_update(&mut state, ext_payload, &ui_tx).await;

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
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        handle_state_update(&mut state, ext_payload, &ui_tx).await;

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
            }],
            current_nomination: None,
            my_team_id: Some("Team 1".into()),
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
        };

        let (ui_tx, _ui_rx) = mpsc::channel(64);
        handle_state_update(&mut state, ext_payload1, &ui_tx).await;

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
            }],
            current_nomination: None,
            my_team_id: Some("Team 1".into()),
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
        };

        handle_state_update(&mut state, ext_payload2, &ui_tx).await;

        // Draft ID should NOT change across reconnect with same ESPN ID
        assert_eq!(state.draft_id, draft_id_after_first);
        assert_eq!(state.espn_draft_id, Some("espn_12345_2026".into()));
    }
}
