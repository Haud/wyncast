// Application state and orchestration logic.
//
// The central event loop that coordinates WebSocket events from the Firefox
// extension, LLM streaming events, and user commands from the TUI. Maintains
// the complete application state and pushes UI updates to the TUI render loop.

use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::Config;
use crate::db::Database;
use crate::draft::state::{
    compute_state_diff, ActiveNomination, DraftState, NominationPayload, PickPayload,
    StateUpdatePayload,
};
use crate::protocol::{
    ConnectionStatus, ExtensionMessage, LlmEvent, LlmStatus, NominationInfo, TabId, UiUpdate,
    UserCommand,
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
    pub previous_extension_state: Option<StateUpdatePayload>,
    pub current_llm_task: Option<tokio::task::JoinHandle<()>>,
    pub llm_mode: Option<LlmMode>,
    pub nomination_analysis_text: String,
    pub nomination_analysis_status: LlmStatus,
    pub nomination_plan_text: String,
    pub nomination_plan_status: LlmStatus,
    pub connection_status: ConnectionStatus,
    pub active_tab: TabId,
    pub category_needs: CategoryNeeds,
}

impl AppState {
    /// Create a new AppState with the given components.
    pub fn new(
        config: Config,
        draft_state: DraftState,
        available_players: Vec<PlayerValuation>,
        all_projections: AllProjections,
        db: Database,
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
            previous_extension_state: None,
            current_llm_task: None,
            llm_mode: None,
            nomination_analysis_text: String::new(),
            nomination_analysis_status: LlmStatus::Idle,
            nomination_plan_text: String::new(),
            nomination_plan_status: LlmStatus::Idle,
            connection_status: ConnectionStatus::Disconnected,
            active_tab: TabId::Analysis,
            category_needs: CategoryNeeds::default(),
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

            // Record in DraftState
            self.draft_state.record_pick(pick.clone());

            // Persist to DB
            if let Err(e) = self.db.record_pick(pick) {
                warn!("Failed to persist pick to DB: {}", e);
            }

            // Remove from available player pool (match by name since we don't
            // have a unified player ID)
            self.available_players
                .retain(|p| p.name != pick.player_name);
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

    /// Handle a new or changed nomination.
    ///
    /// Computes instant analysis and triggers LLM analysis (stub for now).
    pub fn handle_nomination(
        &mut self,
        nomination: &ActiveNomination,
    ) -> Option<InstantAnalysis> {
        // Find the nominated player in our available pool
        let player = self
            .available_players
            .iter()
            .find(|p| p.name == nomination.player_name);

        let analysis = player.map(|p| {
            compute_instant_analysis(
                p,
                &self.draft_state.my_team().roster,
                &self.available_players,
                &self.scarcity,
                &self.inflation,
                &self.category_needs,
            )
        });

        // Update DraftState nomination
        self.draft_state.current_nomination = Some(nomination.clone());

        // Cancel any existing LLM task
        self.cancel_llm_task();

        // Set up LLM mode for nomination analysis (stub)
        self.llm_mode = Some(LlmMode::NominationAnalysis {
            player_name: nomination.player_name.clone(),
            player_id: nomination.player_id.clone(),
            nominated_by: nomination.nominated_by.clone(),
            current_bid: nomination.current_bid,
        });

        // Clear previous analysis text
        self.nomination_analysis_text.clear();
        self.nomination_analysis_status = LlmStatus::Idle;

        // LLM call is a STUB - just log it. Real integration is Task 14/16.
        info!(
            "LLM analysis would be triggered for nomination: {} (bid: ${})",
            nomination.player_name, nomination.current_bid
        );

        analysis
    }

    /// Handle nomination cleared (pick completed for the nominated player).
    pub fn handle_nomination_cleared(&mut self) {
        self.draft_state.current_nomination = None;
        self.cancel_llm_task();
        self.llm_mode = None;
        self.nomination_analysis_text.clear();
        self.nomination_analysis_status = LlmStatus::Idle;
    }

    /// Cancel the current LLM task if one is running.
    pub fn cancel_llm_task(&mut self) {
        if let Some(handle) = self.current_llm_task.take() {
            handle.abort();
            info!("Cancelled previous LLM task");
        }
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
                })
                .collect(),
            current_nomination: payload.current_nomination.as_ref().map(|n| {
                NominationPayload {
                    player_id: n.player_id.clone(),
                    player_name: n.player_name.clone(),
                    position: n.position.clone(),
                    nominated_by: n.nominated_by.clone(),
                    current_bid: n.current_bid,
                    current_bidder: n.current_bidder.clone(),
                    time_remaining: n.time_remaining,
                }
            }),
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

    loop {
        tokio::select! {
            // --- WebSocket events ---
            ws_event = ws_rx.recv() => {
                match ws_event {
                    Some(WsEvent::Connected { addr }) => {
                        info!("Extension connected from {}", addr);
                        state.connection_status = ConnectionStatus::Connected;
                        let _ = ui_tx.send(UiUpdate::ConnectionStatus(ConnectionStatus::Connected)).await;
                    }
                    Some(WsEvent::Disconnected) => {
                        info!("Extension disconnected");
                        state.connection_status = ConnectionStatus::Disconnected;
                        let _ = ui_tx.send(UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)).await;
                    }
                    Some(WsEvent::Message(json_str)) => {
                        handle_ws_message(&mut state, &json_str, &ui_tx).await;
                    }
                    None => {
                        info!("WebSocket channel closed, shutting down");
                        break;
                    }
                }
            }

            // --- LLM events ---
            llm_event = llm_rx.recv() => {
                match llm_event {
                    Some(event) => {
                        handle_llm_event(&mut state, event, &ui_tx).await;
                    }
                    None => {
                        // LLM channel closed - not fatal, just log
                        info!("LLM channel closed");
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
        ExtensionMessage::ExtensionHeartbeat { .. } => {
            // Heartbeats are logged at trace level, no action needed
        }
    }
}

/// Handle a state update from the extension.
///
/// Performs differential state detection, processes new picks,
/// and handles nomination changes.
async fn handle_state_update(
    state: &mut AppState,
    ext_payload: crate::protocol::StateUpdatePayload,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    let internal_payload = AppState::convert_extension_state(&ext_payload);

    // Compute diff against previous state
    let diff = compute_state_diff(&state.previous_extension_state, &internal_payload);

    // Process new picks
    if !diff.new_picks.is_empty() {
        info!("Processing {} new picks", diff.new_picks.len());
        state.process_new_picks(diff.new_picks);
    }

    // Handle nomination changes
    if diff.nomination_changed {
        if diff.nomination_cleared {
            info!("Nomination cleared");
            state.handle_nomination_cleared();
            let _ = ui_tx.send(UiUpdate::NominationCleared).await;
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
        // Same player, bid updated - update the nomination info
        if let Some(ref nomination) = diff.new_nomination {
            state.draft_state.current_nomination = Some(nomination.clone());

            let nom_info = NominationInfo {
                player_name: nomination.player_name.clone(),
                position: nomination.position.clone(),
                nominated_by: nomination.nominated_by.clone(),
                current_bid: nomination.current_bid,
                current_bidder: nomination.current_bidder.clone(),
                time_remaining: nomination.time_remaining,
            };
            let _ = ui_tx
                .send(UiUpdate::NominationUpdate(Box::new(nom_info)))
                .await;
        }
    }

    // Store current state for next diff
    state.previous_extension_state = Some(internal_payload);
}

/// Handle an LLM streaming event.
///
/// Routes tokens and completions to the appropriate text buffer
/// based on the current LLM mode.
async fn handle_llm_event(
    state: &mut AppState,
    event: LlmEvent,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    match (&state.llm_mode, event) {
        (Some(LlmMode::NominationAnalysis { .. }), LlmEvent::Token(token)) => {
            state.nomination_analysis_text.push_str(&token);
            state.nomination_analysis_status = LlmStatus::Streaming;
            let _ = ui_tx.send(UiUpdate::AnalysisToken(token)).await;
        }
        (Some(LlmMode::NominationAnalysis { .. }), LlmEvent::Complete { full_text, .. }) => {
            state.nomination_analysis_text = full_text;
            state.nomination_analysis_status = LlmStatus::Complete;
            let _ = ui_tx.send(UiUpdate::AnalysisComplete).await;
        }
        (Some(LlmMode::NominationAnalysis { .. }), LlmEvent::Error(e)) => {
            warn!("LLM analysis error: {}", e);
            state.nomination_analysis_status = LlmStatus::Error;
        }
        (Some(LlmMode::NominationPlanning), LlmEvent::Token(token)) => {
            state.nomination_plan_text.push_str(&token);
            state.nomination_plan_status = LlmStatus::Streaming;
            let _ = ui_tx.send(UiUpdate::PlanToken(token)).await;
        }
        (Some(LlmMode::NominationPlanning), LlmEvent::Complete { full_text, .. }) => {
            state.nomination_plan_text = full_text;
            state.nomination_plan_status = LlmStatus::Complete;
            let _ = ui_tx.send(UiUpdate::PlanComplete).await;
        }
        (Some(LlmMode::NominationPlanning), LlmEvent::Error(e)) => {
            warn!("LLM planning error: {}", e);
            state.nomination_plan_status = LlmStatus::Error;
        }
        (None, _) => {
            // No active LLM mode - discard the event
            warn!("Received LLM event with no active mode, discarding");
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
            if let Some(ref nom) = state.draft_state.current_nomination {
                info!("Refreshing analysis for {}", nom.player_name);
                // LLM stub - just log
                state.nomination_analysis_text.clear();
                state.nomination_analysis_status = LlmStatus::Idle;
            }
        }
        UserCommand::RefreshPlan => {
            info!("Refreshing nomination plan");
            // LLM stub - just log
            state.nomination_plan_text.clear();
            state.nomination_plan_status = LlmStatus::Idle;
            state.llm_mode = Some(LlmMode::NominationPlanning);
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
                };
                state.process_new_picks(vec![pick]);
            }
        }
        UserCommand::Scroll { .. } => {
            // Scroll is handled by the TUI directly, no app-level action needed
        }
        UserCommand::Quit => {
            // Handled in the main loop
        }
    }

    // Send a state snapshot after handling the command (for relevant commands)
    let _ = ui_tx.send(UiUpdate::ConnectionStatus(state.connection_status)).await;
}

// ---------------------------------------------------------------------------
// Crash recovery
// ---------------------------------------------------------------------------

/// Restore application state from the database after a crash/restart.
///
/// If the DB has draft picks recorded, loads them and replays them
/// into the DraftState, then recalculates valuations.
pub fn recover_from_db(state: &mut AppState) -> anyhow::Result<bool> {
    if !state.db.has_draft_in_progress()? {
        info!("No draft in progress, starting fresh");
        return Ok(false);
    }

    let picks = state.db.load_picks()?;
    let pick_count = picks.len();
    info!("Crash recovery: restoring {} picks from DB", pick_count);

    // Restore picks into DraftState
    state.draft_state.restore_from_picks(picks.clone());

    // Remove drafted players from available pool
    for pick in &picks {
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
            teams: {
                let mut t = HashMap::new();
                t.insert("team_1".into(), "Team 1".into());
                t.insert("team_2".into(), "Team 2".into());
                t
            },
            my_team: MyTeam {
                team_id: "team_1".into(),
            },
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
            holds_estimation: HoldsEstimationConfig {
                default_hold_rate: 0.25,
            },
            llm: LlmConfig {
                model: "test".into(),
                analysis_max_tokens: 400,
                planning_max_tokens: 600,
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
                pitchers_sp: "data/projections/pitchers_sp.csv".into(),
                pitchers_rp: "data/projections/pitchers_rp.csv".into(),
                holds_overlay: "data/holds_overlay.csv".into(),
                adp: "data/adp.csv".into(),
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

    fn test_teams() -> Vec<(String, String)> {
        vec![
            ("team_1".into(), "Team 1".into()),
            ("team_2".into(), "Team 2".into()),
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
            adp: None,
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
            adp: None,
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
            adp: HashMap::new(),
        }
    }

    fn create_test_app_state() -> AppState {
        let config = test_config();
        let draft_state =
            DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        let mut available = test_players();

        // Run initial valuation so dollar values are set
        crate::valuation::recalculate_all(
            &mut available,
            &config.league,
            &config.strategy,
            &draft_state,
        );

        let db = Database::open(":memory:").expect("in-memory db");

        AppState::new(config, draft_state, available, empty_projections(), db)
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
            team_id: "team_1".into(),
            team_name: "Team 1".into(),
            player_name: "H_Star".into(),
            position: "1B".into(),
            price: 45,
            espn_player_id: None,
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
        let team = state.draft_state.team("team_1").unwrap();
        assert_eq!(team.budget_spent, 45);
        assert_eq!(team.budget_remaining, 215);
    }

    #[test]
    fn process_new_picks_updates_inflation() {
        let mut state = create_test_app_state();

        let pick = DraftPick {
            pick_number: 1,
            team_id: "team_1".into(),
            team_name: "Team 1".into(),
            player_name: "H_Star".into(),
            position: "1B".into(),
            price: 45,
            espn_player_id: None,
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
            team_id: "team_1".into(),
            team_name: "Team 1".into(),
            player_name: "H_Star".into(),
            position: "1B".into(),
            price: 45,
            espn_player_id: None,
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
            team_id: "team_1".into(),
            team_name: "Team 1".into(),
            player_name: "H_Star".into(),
            position: "1B".into(),
            price: 45,
            espn_player_id: Some("espn_123".into()),
        };

        state.process_new_picks(vec![pick]);

        // Verify the pick was persisted to DB
        let db_picks = state.db.load_picks().unwrap();
        assert_eq!(db_picks.len(), 1);
        assert_eq!(db_picks[0].player_name, "H_Star");
        assert_eq!(db_picks[0].price, 45);
        assert_eq!(db_picks[0].espn_player_id, Some("espn_123".into()));
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
            },
            DraftPick {
                pick_number: 2,
                team_id: "team_2".into(),
                team_name: "Team 2".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: None,
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
            team_id: "team_2".into(),
            team_name: "Team 2".into(),
            player_name: "H_Good".into(),
            position: "2B".into(),
            price: 30,
            espn_player_id: None,
        };

        state.process_new_picks(vec![pick]);

        // DraftState should have the pick
        assert_eq!(state.draft_state.picks.len(), 1);
        assert_eq!(state.draft_state.picks[0].player_name, "H_Good");

        // Team 2 budget should be updated
        let team2 = state.draft_state.team("team_2").unwrap();
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

    #[test]
    fn nomination_triggers_llm_mode() {
        let mut state = create_test_app_state();

        let nomination = ActiveNomination {
            player_name: "H_Star".into(),
            player_id: "espn_1".into(),
            position: "1B".into(),
            nominated_by: "Team 2".into(),
            current_bid: 5,
            current_bidder: None,
            time_remaining: Some(30),
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

    #[test]
    fn nomination_returns_analysis_for_known_player() {
        let mut state = create_test_app_state();

        let nomination = ActiveNomination {
            player_name: "H_Star".into(),
            player_id: "espn_1".into(),
            position: "1B".into(),
            nominated_by: "Team 2".into(),
            current_bid: 5,
            current_bidder: None,
            time_remaining: Some(30),
        };

        let analysis = state.handle_nomination(&nomination);

        // Should return analysis since H_Star is in the available pool
        assert!(analysis.is_some());
        let analysis = analysis.unwrap();
        assert_eq!(analysis.player_name, "H_Star");
    }

    #[test]
    fn nomination_returns_none_for_unknown_player() {
        let mut state = create_test_app_state();

        let nomination = ActiveNomination {
            player_name: "Unknown Player".into(),
            player_id: "espn_999".into(),
            position: "OF".into(),
            nominated_by: "Team 2".into(),
            current_bid: 1,
            current_bidder: None,
            time_remaining: Some(30),
        };

        let analysis = state.handle_nomination(&nomination);

        // Should return None since the player is not in our pool
        assert!(analysis.is_none());
    }

    // -----------------------------------------------------------------------
    // Tests: LLM cancellation (new nomination cancels previous)
    // -----------------------------------------------------------------------

    #[test]
    fn new_nomination_clears_previous_analysis() {
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
        };
        state.handle_nomination(&nom2);

        // Analysis text should be cleared
        assert!(state.nomination_analysis_text.is_empty());
        assert_eq!(state.nomination_analysis_status, LlmStatus::Idle);

        // LLM mode should be updated to the new nomination
        if let Some(LlmMode::NominationAnalysis { player_name, .. }) = &state.llm_mode {
            assert_eq!(player_name, "H_Good");
        } else {
            panic!("Expected NominationAnalysis mode");
        }
    }

    #[test]
    fn nomination_cleared_resets_state() {
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
        };
        state.handle_nomination(&nom);
        state.nomination_analysis_text = "Analysis text".into();
        state.nomination_analysis_status = LlmStatus::Streaming;

        // Clear the nomination
        state.handle_nomination_cleared();

        assert!(state.draft_state.current_nomination.is_none());
        assert!(state.llm_mode.is_none());
        assert!(state.nomination_analysis_text.is_empty());
        assert_eq!(state.nomination_analysis_status, LlmStatus::Idle);
    }

    // -----------------------------------------------------------------------
    // Tests: Crash recovery
    // -----------------------------------------------------------------------

    #[test]
    fn crash_recovery_restores_state() {
        let config = test_config();
        let db = Database::open(":memory:").expect("in-memory db");

        // Record some picks into the database (simulating a previous session)
        let picks = vec![
            DraftPick {
                pick_number: 1,
                team_id: "team_1".into(),
                team_name: "Team 1".into(),
                player_name: "H_Star".into(),
                position: "1B".into(),
                price: 45,
                espn_player_id: None,
            },
            DraftPick {
                pick_number: 2,
                team_id: "team_2".into(),
                team_name: "Team 2".into(),
                player_name: "P_Ace".into(),
                position: "SP".into(),
                price: 50,
                espn_player_id: None,
            },
        ];
        for pick in &picks {
            db.record_pick(pick).unwrap();
        }
        assert!(db.has_draft_in_progress().unwrap());

        // Create a fresh AppState (simulating restart)
        let draft_state =
            DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        let mut available = test_players();
        crate::valuation::recalculate_all(
            &mut available,
            &config.league,
            &config.strategy,
            &draft_state,
        );
        let initial_player_count = available.len();

        let mut state = AppState::new(config, draft_state, available, empty_projections(), db);

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
        let team1 = state.draft_state.team("team_1").unwrap();
        assert_eq!(team1.budget_spent, 45);
        let team2 = state.draft_state.team("team_2").unwrap();
        assert_eq!(team2.budget_spent, 50);
    }

    #[test]
    fn crash_recovery_no_picks_returns_false() {
        let config = test_config();
        let db = Database::open(":memory:").expect("in-memory db");
        let draft_state =
            DraftState::new(test_teams(), "team_1", 260, &test_roster_config());
        let available = test_players();

        let mut state = AppState::new(config, draft_state, available, empty_projections(), db);

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
        assert_eq!(
            update,
            UiUpdate::ConnectionStatus(ConnectionStatus::Connected)
        );

        // Send disconnected event
        ws_tx.send(WsEvent::Disconnected).await.unwrap();

        let update = ui_rx.recv().await.unwrap();
        assert_eq!(
            update,
            UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)
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

        // Send LLM token
        llm_tx.send(LlmEvent::Token("Hello ".into())).await.unwrap();

        let update = ui_rx.recv().await.unwrap();
        assert_eq!(update, UiUpdate::AnalysisToken("Hello ".into()));

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

        // Should receive a NominationUpdate
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
            }],
            current_nomination: Some(crate::protocol::NominationData {
                player_id: "espn_2".into(),
                player_name: "Player Two".into(),
                position: "1B".into(),
                nominated_by: "Team 2".into(),
                current_bid: 10,
                current_bidder: Some("Team 3".into()),
                time_remaining: Some(25),
            }),
            my_team_id: Some("team_1".into()),
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
}
