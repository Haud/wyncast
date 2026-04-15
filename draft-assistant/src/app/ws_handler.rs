use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::db::Database;
use crate::draft::pick::{espn_slot_from_position_str, DraftPick};
use crate::draft::roster::Roster;
use crate::draft::state::{
    compute_state_diff, ActiveNomination, DraftState, NominationPayload, PickPayload,
    ReconcileResult, StateUpdatePayload, TeamState,
};
use crate::matchup::{
    CategoryScore, CategoryState, DailyPlayerRow, DailyTotals, MatchupInfo, MatchupSnapshot,
    ScoringDay, TeamMatchupState, TeamRecord,
};
use crate::protocol::{
    AppMode, DraftBoardData, ExtensionMessage, MatchupStatePayload, NominationInfo,
    PickHistoryEntry, TeamIdMapping, UiUpdate,
};
use crate::valuation;
use crate::stats::CategoryValues;
use crate::valuation::auction::InflationTracker;
use crate::valuation::scarcity::compute_scarcity;

use std::collections::HashMap;

use super::AppState;

/// Infer the roster configuration from the ESPN draft board grid.
///
/// Each team in the draft board has a set of roster slots (e.g. "C", "1B",
/// "OF", "MI", "SP", "BE"). We count the slots from the first team to
/// build the roster config HashMap.
fn infer_roster_config(board: &DraftBoardData) -> Option<HashMap<String, usize>> {
    let team = board.teams.first()?;
    let mut config: HashMap<String, usize> = HashMap::new();
    for slot in &team.slots {
        if !slot.roster_slot.is_empty() {
            *config.entry(slot.roster_slot.clone()).or_insert(0) += 1;
        }
    }
    if config.is_empty() {
        None
    } else {
        Some(config)
    }
}

/// Handle an incoming WebSocket message (JSON from the extension).
pub(super) async fn handle_ws_message(
    state: &mut AppState,
    json_str: &str,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    let msg: ExtensionMessage = match serde_json::from_str(json_str) {
        Ok(m) => m,
        Err(e) => {
            // Logged at error level so the default log filter surfaces it —
            // otherwise a schema drift between extension and Rust protocol
            // silently drops every message. The JSON snippet is capped at
            // 200 chars to make the `type` + first few payload keys visible
            // without flooding the log with a full matchup payload.
            let snippet: String = json_str.chars().take(200).collect();
            error!(
                "Failed to parse extension message: {} (first 200 chars: {})",
                e, snippet
            );
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
        ExtensionMessage::PlayerProjections { timestamp: _, payload } => {
            handle_player_projections(state, payload, ui_tx).await;
        }
        ExtensionMessage::MatchupState { timestamp: _, payload } => {
            handle_matchup_state(state, payload, ui_tx).await;
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
///
/// The extension also sends FULL_STATE_SYNC every 10 seconds as a periodic
/// keyframe. To avoid restarting a streaming LLM analysis every time one of
/// these keyframes arrives, we detect when the incoming nomination is the same
/// player as what is currently being analyzed and preserve the LLM task.
pub(super) async fn handle_full_state_sync(
    state: &mut AppState,
    ext_payload: crate::protocol::StateUpdatePayload,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    // Switch back to Draft mode if we were in Matchup mode. This mirrors the
    // guard in handle_state_update — FULL_STATE_SYNC is a draft message, so
    // receiving one means the active tab is a draft page.
    if state.app_mode == AppMode::Matchup {
        info!("FULL_STATE_SYNC received while in Matchup mode, switching back to Draft");
        state.app_mode = AppMode::Draft;
        let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Draft)).await;
    }

    info!(
        "Received FULL_STATE_SYNC with {} picks — resetting draft state",
        ext_payload.picks.len()
    );

    // Detect if the incoming nomination is the same player as what's currently
    // being analyzed. The extension sends FULL_STATE_SYNC every 10 seconds as
    // a periodic keyframe; if the nomination is unchanged, we should NOT cancel
    // the in-progress LLM analysis or allow it to restart.
    let incoming_nom = ext_payload.current_nomination.as_ref();
    let preserve_llm = match (&state.analysis_player, &state.analysis_request_id, incoming_nom) {
        (Some(ap), Some(_), Some(inc)) => {
            if !ap.player_id.is_empty() && !inc.player_id.is_empty() {
                ap.player_id == inc.player_id
            } else {
                ap.player_name == inc.player_name
            }
        }
        _ => false,
    };

    // Save current nomination before resetting so we can restore it (and use
    // it as a stub baseline for compute_state_diff) when the nomination is
    // unchanged.
    let saved_nomination = state.draft_state.current_nomination.clone();

    // Infer roster config from the draft board if we haven't yet.
    if state.roster_config.is_none() {
        if let Some(ref board) = ext_payload.draft_board {
            if let Some(inferred) = infer_roster_config(board) {
                info!(
                    "Inferred roster config from ESPN draft board: {:?}",
                    inferred
                );
                state.apply_roster_config(inferred);
            }
        }
    }

    // Reset in-memory draft state so the snapshot is applied from scratch.
    // Preserve salary_cap and roster_config (stored inside DraftState).
    let roster = state.roster_config.clone().unwrap_or_else(AppState::default_roster_config);
    state.draft_state = DraftState::new(
        state.config.league.salary_cap,
        &roster,
    );

    // Reset valuation pool and derived state so they're rebuilt cleanly
    // after all snapshot picks are applied.
    if let Some(ref projections) = state.all_projections {
        state.available_players =
            valuation::compute_initial(projections, &state.config, &roster, &state.stat_registry)
                .unwrap_or_default();
    } else {
        state.available_players = Vec::new();
    }
    state.scarcity = compute_scarcity(&state.available_players, &roster);
    state.inflation = InflationTracker::new();
    state.category_needs = CategoryValues::uniform(state.stat_registry.len(), 0.5);

    // --- Grid-based state building (when draft board + pick history available) ---
    //
    // When the extension provides draft board grid data (always fully rendered,
    // never virtualized), we build the entire team/roster/pick state directly
    // from it instead of relying on the pick log (which is virtualized and
    // may only contain ~106 of 188 picks). This is the key fix for mid-draft
    // resume reliability.
    let grid_based_rebuild = build_state_from_grid(
        state,
        &ext_payload.draft_board,
        &ext_payload.pick_history,
        &ext_payload.team_id_mapping,
    );

    if grid_based_rebuild {
        info!(
            "FULL_STATE_SYNC: built state from draft board grid ({} teams, {} picks)",
            state.draft_state.teams.len(),
            state.draft_state.picks.len(),
        );

        // Update inflation and scarcity
        state.inflation.update(
            &state.available_players,
            &state.draft_state,
            &state.config.league,
        );
        let roster = state.roster_config.clone().unwrap_or_else(AppState::default_roster_config);
        state.scarcity = compute_scarcity(&state.available_players, &roster);
    } else {
        info!(
            "FULL_STATE_SYNC: grid data unavailable, requesting keyframe retry"
        );
        // Request a retry — by the time the extension responds, the grid
        // may have rendered.
        if let Some(ref ws_tx) = state.ws_outbound_tx {
            let request = serde_json::json!({ "type": "REQUEST_KEYFRAME" });
            if let Err(e) = ws_tx.try_send(request.to_string()) {
                warn!("Failed to send REQUEST_KEYFRAME for grid retry: {}", e);
            }
        }
    }

    // Build stub previous_extension_state for compute_state_diff so that
    // handle_state_update doesn't re-process picks that we already loaded.
    //
    // When grid_based_rebuild is true, picks are sourced from the complete
    // grid/pick-history data (not the virtualized extension pick log). We
    // convert them to PickPayloads for the stub so compute_state_diff sees
    // every pick as "already known" and doesn't double-process any.
    //
    // When preserve_llm is true, include the saved nomination in the stub
    // so compute_state_diff doesn't treat it as a new nomination (which
    // would fire NominationUpdate and restart the LLM analysis).
    let stub_picks: Vec<PickPayload> = if grid_based_rebuild {
        // Build stub picks from the complete grid-sourced picks (not the
        // virtualized extension pick log) so compute_state_diff in
        // handle_state_update recognizes ALL picks as already processed.
        state.draft_state.picks.iter().map(PickPayload::from).collect()
    } else {
        vec![]
    };

    if preserve_llm {
        // Same nomination is still active: keep the LLM task and mode so
        // streaming continues uninterrupted.
        state.previous_extension_state = saved_nomination.as_ref().map(|nom| StateUpdatePayload {
            picks: stub_picks.clone(),
            current_nomination: Some(NominationPayload {
                player_id: nom.player_id.clone(),
                player_name: nom.player_name.clone(),
                position: nom.position.clone(),
                nominated_by: nom.nominated_by.clone(),
                current_bid: nom.current_bid,
                current_bidder: nom.current_bidder.clone(),
                time_remaining: nom.time_remaining,
                eligible_slots: nom.eligible_slots.clone(),
            }),
            teams: vec![],
            pick_count: None,
            total_picks: None,
        });
        info!(
            "FULL_STATE_SYNC: preserving in-progress LLM analysis (same nomination: {})",
            saved_nomination.as_ref().map(|n| n.player_name.as_str()).unwrap_or("unknown")
        );
    } else if grid_based_rebuild {
        // Grid-based rebuild: picks are already processed, set stub so
        // handle_state_update doesn't re-process them.
        state.previous_extension_state = Some(StateUpdatePayload {
            picks: stub_picks,
            current_nomination: None,
            teams: vec![],
            pick_count: None,
            total_picks: None,
        });
        state.cancel_llm_tasks();
    } else {
        // Nomination changed or no active analysis: clear all LLM state so
        // handle_state_update can start fresh.
        state.previous_extension_state = None;
        state.cancel_llm_tasks();
    }

    // Now process the snapshot as a regular state update.  When preserve_llm
    // is true, compute_state_diff will see the stub previous state and treat
    // the nomination as unchanged, so nomination_changed will not fire and
    // NominationUpdate will not be sent to the TUI.
    //
    // When grid_based_rebuild is true, the stub includes all picks so they
    // won't be re-processed. handle_state_update still handles: draft ID
    // detection, nomination changes, team budget reconciliation, and sending
    // UI snapshots.
    handle_state_update(state, ext_payload, ui_tx).await;

    // A grid-based rebuild resets and reconstructs ALL state (teams, picks,
    // rosters, budgets, inflation, scarcity). Always push a snapshot to the
    // TUI so that everything — including hitting/pitching budget split — is
    // up to date. handle_state_update may or may not have sent one depending
    // on its has_changes guard, but a full rebuild is always a "changed" event.
    if grid_based_rebuild {
        let snapshot = state.build_snapshot();
        let _ = ui_tx
            .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
            .await;
    }

    // Restore current_nomination after the draft state reset if it wasn't set
    // by handle_state_update (happens when bid/bidder also didn't change, so
    // neither nomination_changed nor bid_updated fired).
    if preserve_llm && state.draft_state.current_nomination.is_none() {
        state.draft_state.current_nomination = saved_nomination;
    }
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
pub(super) async fn handle_state_update(
    state: &mut AppState,
    ext_payload: crate::protocol::StateUpdatePayload,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    // --- Switch back to Draft mode if we were in Matchup mode ---
    if state.app_mode == AppMode::Matchup {
        info!("Draft message received while in Matchup mode, switching back to Draft");
        state.app_mode = AppMode::Draft;
        let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Draft)).await;
    }

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
                let roster = state.roster_config.clone().unwrap_or_else(AppState::default_roster_config);
                state.draft_state = DraftState::new(
                    state.config.league.salary_cap,
                    &roster,
                );
                state.available_players = if let Some(ref projections) = state.all_projections {
                    valuation::compute_initial(projections, &state.config, &roster, &state.stat_registry)
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                state.scarcity =
                    compute_scarcity(&state.available_players, &roster);
                state.inflation = InflationTracker::new();
                state.previous_extension_state = None;
                // Clear LLM state so stale analysis from the previous draft
                // doesn't bleed into the new session.
                state.llm_requests.cancel_all();
                state.analysis_request_id = None;
                state.plan_request_id = None;
                state.analysis_player = None;
                state.category_needs = CategoryValues::uniform(state.stat_registry.len(), 0.5);
                state.grid_picks_persisted = false;
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

    // Set the user's team from ESPN team ID.
    // Priority: grid isMyTeam flag -> extension myTeamId -> pick history is_my_pick
    if !state.draft_state.teams.is_empty() {
        // Prefer draft board's isMyTeam — resolve to team_id via board data
        let my_team_id_from_grid = ext_payload
            .draft_board
            .as_ref()
            .and_then(|db| {
                db.teams
                    .iter()
                    .find(|t| t.is_my_team)
                    .and_then(|t| if t.team_id.is_empty() { None } else { Some(t.team_id.clone()) })
            });

        if let Some(ref team_id) = my_team_id_from_grid {
            state.draft_state.set_my_team_by_id(team_id);
        } else if let Some(ref my_team_id) = ext_payload.my_team_id {
            if !my_team_id.is_empty() {
                state.draft_state.set_my_team_by_id(my_team_id);
            }
        } else {
            // Fallback: use is_my_pick from pick history
            let my_team_id_from_history = ext_payload.pick_history.as_ref().and_then(|history| {
                history.iter().find(|p| p.is_my_pick).and_then(|p| {
                    if p.team_id.is_empty() { None } else { Some(p.team_id.clone()) }
                })
            });
            if let Some(ref team_id) = my_team_id_from_history {
                info!("Identified my team from pick history is_my_pick fallback: team_id={}", team_id);
                state.draft_state.set_my_team_by_id(team_id);
            }
        }
    }

    // Draft board reconciliation check: if the grid shows more filled slots
    // than we have picks, something is out of sync.
    if let Some(ref draft_board) = ext_payload.draft_board {
        let grid_filled: usize = draft_board
            .teams
            .iter()
            .flat_map(|t| &t.slots)
            .filter(|s| s.filled)
            .count();
        let our_picks = state.draft_state.picks.len();
        if grid_filled > 0 && our_picks > 0 && grid_filled != our_picks {
            warn!(
                "Draft board grid shows {} filled slots but we have {} picks — \
                 state may be out of sync (will be corrected on next FULL_STATE_SYNC)",
                grid_filled, our_picks
            );
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
            if let Some(plan_id) = planning_started {
                let _ = ui_tx.send(UiUpdate::PlanStarted { request_id: plan_id }).await;
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
                .send(UiUpdate::NominationUpdate { info: Box::new(nom_info), analysis_request_id: state.analysis_request_id })
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
    if teams_just_registered && state.analysis_request_id.is_none() {
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
                .send(UiUpdate::NominationUpdate { info: Box::new(nom_info), analysis_request_id: state.analysis_request_id })
                .await;

            if let Some(_analysis) = analysis {
                info!("Instant analysis computed for retried nomination");
            }
        }
    }

    // Store current state for next diff
    state.previous_extension_state = Some(internal_payload);
}

// ---------------------------------------------------------------------------
// Matchup state handling
// ---------------------------------------------------------------------------

/// Parse a "W-L-T" record string (e.g. "3-2-7") into a `TeamRecord`.
fn parse_record(s: &str) -> TeamRecord {
    let parts: Vec<u16> = s.split('-').filter_map(|p| p.trim().parse().ok()).collect();
    TeamRecord {
        wins: parts.first().copied().unwrap_or(0),
        losses: parts.get(1).copied().unwrap_or(0),
        ties: parts.get(2).copied().unwrap_or(0),
    }
}

/// Determine category win/loss/tie state from values and sort direction.
///
/// `None` values (ESPN renders `"--"` for rate stats with a zero denominator)
/// are treated as Tied — neither side earns credit for a category that has
/// not yet produced a real value.
fn category_state(
    home_value: Option<f64>,
    away_value: Option<f64>,
    lower_is_better: bool,
) -> CategoryState {
    let (home_value, away_value) = match (home_value, away_value) {
        (Some(h), Some(a)) => (h, a),
        _ => return CategoryState::Tied,
    };
    if (home_value - away_value).abs() < f64::EPSILON {
        CategoryState::Tied
    } else if lower_is_better {
        if home_value < away_value { CategoryState::Winning } else { CategoryState::Losing }
    } else if home_value > away_value {
        CategoryState::Winning
    } else {
        CategoryState::Losing
    }
}

/// Handle an incoming matchup state message from the extension.
///
/// Converts the raw payload into a `MatchupSnapshot`, switches to
/// `AppMode::Matchup` if needed, and sends the snapshot to the TUI.
async fn handle_matchup_state(
    state: &mut AppState,
    payload: MatchupStatePayload,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    info!(
        "Processing matchup state: {} vs {} (period {})",
        payload.home_team.name, payload.away_team.name, payload.matchup_period
    );

    let my_record = parse_record(&payload.home_team.record);
    let opp_record = parse_record(&payload.away_team.record);
    let my_category_score = parse_record(&payload.home_team.matchup_score);
    let opp_category_score = parse_record(&payload.away_team.matchup_score);

    // Convert categories with win/loss state
    let category_scores: Vec<CategoryScore> = payload
        .categories
        .iter()
        .map(|cat| CategoryScore {
            stat_abbrev: cat.abbrev.clone(),
            // Coerce None to 0.0 for the domain type — the `state` field still
            // captures "no comparison possible" as Tied so downstream widgets
            // don't incorrectly credit either team.
            my_value: cat.home_value.unwrap_or(0.0),
            opp_value: cat.away_value.unwrap_or(0.0),
            state: category_state(cat.home_value, cat.away_value, cat.lower_is_better),
        })
        .collect();

    // Convert player tables into a single ScoringDay.
    // The extension sends one day's worth of data at a time (the selected day).
    let batting_rows: Vec<DailyPlayerRow> = payload
        .batting
        .players
        .iter()
        .map(|p| DailyPlayerRow {
            slot: p.slot.clone(),
            player_name: p.name.clone(),
            team: p.team.clone(),
            positions: p.positions.clone(),
            opponent: p.opponent.clone(),
            game_status: p.status.clone(),
            stats: p.stats.clone(),
        })
        .collect();

    let pitching_rows: Vec<DailyPlayerRow> = payload
        .pitching
        .players
        .iter()
        .map(|p| DailyPlayerRow {
            slot: p.slot.clone(),
            player_name: p.name.clone(),
            team: p.team.clone(),
            positions: p.positions.clone(),
            opponent: p.opponent.clone(),
            game_status: p.status.clone(),
            stats: p.stats.clone(),
        })
        .collect();

    let batting_totals = payload.batting.totals.as_ref().map(|t| DailyTotals {
        stats: t.clone(),
    });
    let pitching_totals = payload.pitching.totals.as_ref().map(|t| DailyTotals {
        stats: t.clone(),
    });

    let scoring_day = ScoringDay {
        date: payload.selected_day.clone(),
        label: payload.selected_day.clone(),
        batting_stat_columns: payload.batting.headers.clone(),
        pitching_stat_columns: payload.pitching.headers.clone(),
        batting_rows,
        pitching_rows,
        batting_totals,
        pitching_totals,
    };

    let scoring_period_days = vec![scoring_day];

    let snapshot = MatchupSnapshot {
        matchup_info: MatchupInfo {
            matchup_period: payload.matchup_period,
            start_date: payload.start_date,
            end_date: payload.end_date,
            my_team_name: payload.home_team.name.clone(),
            opp_team_name: payload.away_team.name.clone(),
            my_record,
            opp_record,
        },
        my_team: TeamMatchupState {
            name: payload.home_team.name.clone(),
            abbrev: abbreviate_team_name(&payload.home_team.name),
            record: parse_record(&payload.home_team.record),
            category_score: my_category_score,
        },
        opp_team: TeamMatchupState {
            name: payload.away_team.name.clone(),
            abbrev: abbreviate_team_name(&payload.away_team.name),
            record: parse_record(&payload.away_team.record),
            category_score: opp_category_score,
        },
        category_scores,
        selected_day: 0,
        scoring_period_days,
    };

    // Store snapshot in app state
    state.matchup_snapshot = Some(snapshot.clone());

    // Switch to Matchup mode if not already there
    if state.app_mode != AppMode::Matchup {
        info!("Switching to Matchup mode");
        state.app_mode = AppMode::Matchup;
        let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Matchup)).await;
    }

    let _ = ui_tx
        .send(UiUpdate::MatchupSnapshot(Box::new(snapshot)))
        .await;
}

/// Create a short abbreviation from a team name.
///
/// Takes up to 3 uppercase initials from words. Falls back to the first 3
/// characters uppercased if no multi-word name is available.
fn abbreviate_team_name(name: &str) -> String {
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if initials.len() >= 2 {
        initials.chars().take(3).collect()
    } else {
        name.chars().take(3).map(|c| c.to_ascii_uppercase()).collect()
    }
}

// ---------------------------------------------------------------------------
// Grid-based state building
// ---------------------------------------------------------------------------

/// Extract a player name from a draft board slot's first/last name fields.
/// Returns `None` if both fields are empty.
fn slot_player_name(slot: &crate::protocol::DraftBoardSlot) -> Option<String> {
    let first = slot.first_name.as_deref().unwrap_or("");
    let last = slot.last_name.as_deref().unwrap_or("");
    let name = format!("{} {}", first, last).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// Build the complete draft state from the draft board grid and pick history.
///
/// When the extension provides draft board grid data (always fully rendered,
/// never virtualized), we can build the entire team/roster/pick state directly
/// from it. This is far more reliable than the pick log when resuming mid-draft,
/// since the pick log is virtualized and may only contain a subset of picks.
///
/// Returns `true` if the state was built from grid data, `false` if the grid
/// data was not available or insufficient.
fn build_state_from_grid(
    state: &mut AppState,
    draft_board: &Option<DraftBoardData>,
    pick_history: &Option<Vec<PickHistoryEntry>>,
    team_id_mapping: &Option<Vec<TeamIdMapping>>,
) -> bool {
    let board = match draft_board {
        Some(b) if !b.teams.is_empty() => b,
        _ => return false,
    };

    // Pre-build a team name -> ESPN team ID lookup map to avoid
    // repeated linear scans through the mapping slice.
    let team_id_map: std::collections::HashMap<&str, &str> = team_id_mapping
        .as_ref()
        .map(|mapping| {
            mapping
                .iter()
                .map(|m| (m.team_name.as_str(), m.espn_team_id.as_str()))
                .collect()
        })
        .unwrap_or_default();

    // Count filled slots — if none are filled, no picks have been made yet
    // and the normal flow works fine.
    let filled_count: usize = board
        .teams
        .iter()
        .flat_map(|t| &t.slots)
        .filter(|s| s.filled)
        .count();
    if filled_count == 0 {
        return false;
    }

    info!(
        "Building state from draft board grid: {} teams, {} filled slots",
        board.teams.len(),
        filled_count
    );

    // 1. Register teams from the draft board header
    let salary_cap = state.config.league.salary_cap;
    for db_team in &board.teams {
        // Calculate budget from filled slots
        let spent: u32 = db_team
            .slots
            .iter()
            .filter(|s| s.filled)
            .filter_map(|s| s.price)
            .sum();

        // Resolve team ID: prefer the ID from the draft board (sent by extension),
        // fall back to the mapping lookup.
        let resolved_team_id = if !db_team.team_id.is_empty() {
            db_team.team_id.clone()
        } else if let Some(id) = team_id_map.get(db_team.team_name.as_str()) {
            id.to_string()
        } else {
            String::new()
        };

        let mut team = TeamState {
            team_id: resolved_team_id,
            team_name: db_team.team_name.clone(),
            roster: Roster::new(&state.roster_config.clone().unwrap_or_else(AppState::default_roster_config)),
            budget_spent: spent,
            budget_remaining: salary_cap.saturating_sub(spent),
            // NOTE: These grid-computed budgets are provisional. reconcile_budgets()
            // in handle_state_update() will overwrite them with ESPN's authoritative
            // pick-train values when available, ensuring consistency.
        };

        // Fill roster slots from the grid
        for slot in &db_team.slots {
            if !slot.filled {
                continue;
            }
            let player_name = match slot_player_name(slot) {
                Some(name) => name,
                None => continue,
            };

            let price = slot.price.unwrap_or(0);
            let roster_slot_str = &slot.roster_slot;
            let assigned_slot = espn_slot_from_position_str(roster_slot_str);

            // Use the roster slot string as the position for placement.
            // The assigned_slot gives us the ESPN slot ID for direct placement.
            team.roster.add_player_with_slots(
                &player_name,
                roster_slot_str,
                price,
                &[], // No eligible_slots from grid — use assigned_slot instead
                assigned_slot,
                None, // No ESPN player ID from grid cells
            );
        }

        state.draft_state.teams.push(team);
    }

    // Set my_team from the isMyTeam flag (set by extension from roster dropdown)
    if let Some(idx) = board.teams.iter().position(|t| t.is_my_team) {
        state.draft_state.my_team_idx = Some(idx);
    }

    // Compute total picks and nomination order now that teams are registered
    let draftable_per_team = state
        .draft_state
        .teams
        .first()
        .map(|t| t.roster.draftable_count())
        .unwrap_or(0);
    state.draft_state.total_picks = draftable_per_team * state.draft_state.teams.len();
    state.draft_state.nomination_order = (0..state.draft_state.teams.len()).collect();

    // 2. Build picks from pick history (if available) for chronological draft log
    if let Some(history) = pick_history {
        for entry in history {
            // Convert eligible position strings to ESPN slot IDs
            let eligible_slots: Vec<u16> = entry
                .eligible_positions
                .iter()
                .filter_map(|s| espn_slot_from_position_str(s))
                .collect();

            // Use the first eligible position as the position string
            let position = entry
                .eligible_positions
                .first()
                .cloned()
                .unwrap_or_default();

            // Resolve team ID: prefer the ID from the pick history entry (sent
            // by extension), fall back to the mapping lookup.
            let resolved_team_id = if !entry.team_id.is_empty() {
                entry.team_id.clone()
            } else if let Some(id) = team_id_map.get(entry.team_name.as_str()) {
                id.to_string()
            } else {
                String::new()
            };

            let pick = DraftPick {
                pick_number: entry.pick_number,
                team_id: resolved_team_id,
                team_name: entry.team_name.clone(),
                player_name: entry.player_name.clone(),
                position,
                price: entry.price,
                espn_player_id: if entry.espn_player_id.is_empty() {
                    None
                } else {
                    Some(entry.espn_player_id.clone())
                },
                eligible_slots,
                assigned_slot: None, // Pick history doesn't have assigned slot
            };

            // Add directly to picks list (bypassing record_pick since rosters
            // are already built from the grid). We still need the picks list
            // for the draft log display.
            state.draft_state.picks.push(pick);
        }
        state.draft_state.pick_count = state.draft_state.picks.len();
    } else {
        // No pick history — count filled slots from the grid as our pick count
        state.draft_state.pick_count = filled_count;

        // Build minimal picks from the grid for the draft log.
        // These won't have chronological order or ESPN player IDs,
        // but at least the count and player names will be correct.
        let mut pick_num = 0u32;
        for db_team in &board.teams {
            for slot in &db_team.slots {
                if !slot.filled {
                    continue;
                }
                let player_name = match slot_player_name(slot) {
                    Some(name) => name,
                    None => continue,
                };

                pick_num += 1;
                let position = slot
                    .natural_position
                    .as_deref()
                    .unwrap_or(&slot.roster_slot)
                    .to_string();

                let team_id = if !db_team.team_id.is_empty() {
                    db_team.team_id.clone()
                } else {
                    team_id_map
                        .get(db_team.team_name.as_str())
                        .map(|id| id.to_string())
                        .unwrap_or_default()
                };

                state.draft_state.picks.push(DraftPick {
                    pick_number: pick_num,
                    team_id,
                    team_name: db_team.team_name.clone(),
                    player_name,
                    position,
                    price: slot.price.unwrap_or(0),
                    espn_player_id: None,
                    eligible_slots: vec![],
                    assigned_slot: espn_slot_from_position_str(&slot.roster_slot),
                });
            }
        }
    }

    // 3. Remove drafted players from available pool
    let drafted_names: std::collections::HashSet<String> = state
        .draft_state
        .picks
        .iter()
        .map(|p| p.player_name.clone())
        .collect();
    state
        .available_players
        .retain(|p| !drafted_names.contains(&p.name));

    // Persist picks to DB for crash recovery.
    // Skip if we've already persisted grid picks this session — FULL_STATE_SYNC
    // fires every 10 seconds and the grid data is the same each time.
    // record_pick uses INSERT OR IGNORE for idempotency on the first call.
    if !state.grid_picks_persisted {
        for pick in &state.draft_state.picks {
            if let Err(e) = state.db.record_pick(pick, &state.draft_id) {
                warn!("Failed to persist grid-sourced pick to DB: {}", e);
            }
        }
        state.grid_picks_persisted = true;
    }

    true
}

// ---------------------------------------------------------------------------
// ESPN projection handling
// ---------------------------------------------------------------------------

/// Handle ESPN player projections received from the extension.
///
/// If the user already configured CSV projections (present in `all_projections`),
/// we skip the ESPN data since CSV takes priority as an explicit override.
/// Otherwise, convert the ESPN projections and apply them to the app state.
async fn handle_player_projections(
    state: &mut AppState,
    payload: crate::protocol::EspnProjectionsPayload,
    ui_tx: &mpsc::Sender<crate::protocol::UiUpdate>,
) {
    if state.all_projections.is_some() {
        info!(
            "Received {} ESPN player projections, but CSV projections already loaded — skipping",
            payload.players.len()
        );
        return;
    }

    info!(
        "Received {} ESPN player projections — converting",
        payload.players.len()
    );
    let projections = valuation::projections::from_espn_projections(&payload.players);
    info!(
        "Converted ESPN projections: {} hitters, {} pitchers",
        projections.hitters.len(),
        projections.pitchers.len()
    );

    if projections.hitters.is_empty() && projections.pitchers.is_empty() {
        warn!("ESPN projections produced zero players — ignoring");
        return;
    }

    state.apply_projections(projections);

    // Send a state snapshot to the TUI to reflect the newly computed valuations
    let snapshot = state.build_snapshot();
    let _ = ui_tx
        .send(crate::protocol::UiUpdate::StateSnapshot(Box::new(snapshot)))
        .await;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matchup::CategoryState;
    use crate::protocol::{
        DraftBoardSlot, DraftBoardTeam, MatchupCategoryPayload, MatchupPlayerPayload,
        MatchupSectionPayload, MatchupStatePayload, MatchupTeamPayload,
    };

    #[test]
    fn infer_roster_config_from_board() {
        let board = DraftBoardData {
            teams: vec![DraftBoardTeam {
                team_id: "1".into(),
                team_name: "Team 1".into(),
                column: 0,
                is_my_team: true,
                is_on_the_clock: false,
                slots: vec![
                    DraftBoardSlot { row: 0, roster_slot: "C".into(), filled: false, first_name: None, last_name: None, pro_team: None, natural_position: None, price: None },
                    DraftBoardSlot { row: 1, roster_slot: "1B".into(), filled: false, first_name: None, last_name: None, pro_team: None, natural_position: None, price: None },
                    DraftBoardSlot { row: 2, roster_slot: "OF".into(), filled: false, first_name: None, last_name: None, pro_team: None, natural_position: None, price: None },
                    DraftBoardSlot { row: 3, roster_slot: "OF".into(), filled: false, first_name: None, last_name: None, pro_team: None, natural_position: None, price: None },
                    DraftBoardSlot { row: 4, roster_slot: "OF".into(), filled: false, first_name: None, last_name: None, pro_team: None, natural_position: None, price: None },
                    DraftBoardSlot { row: 5, roster_slot: "MI".into(), filled: false, first_name: None, last_name: None, pro_team: None, natural_position: None, price: None },
                    DraftBoardSlot { row: 6, roster_slot: "SP".into(), filled: false, first_name: None, last_name: None, pro_team: None, natural_position: None, price: None },
                    DraftBoardSlot { row: 7, roster_slot: "SP".into(), filled: false, first_name: None, last_name: None, pro_team: None, natural_position: None, price: None },
                    DraftBoardSlot { row: 8, roster_slot: "BE".into(), filled: false, first_name: None, last_name: None, pro_team: None, natural_position: None, price: None },
                ],
            }],
            on_the_clock_team: None,
        };

        let config = infer_roster_config(&board).expect("should infer roster config");
        assert_eq!(config.get("C"), Some(&1));
        assert_eq!(config.get("1B"), Some(&1));
        assert_eq!(config.get("OF"), Some(&3));
        assert_eq!(config.get("MI"), Some(&1));
        assert_eq!(config.get("SP"), Some(&2));
        assert_eq!(config.get("BE"), Some(&1));
    }

    #[test]
    fn infer_roster_config_empty_board() {
        let board = DraftBoardData {
            teams: vec![],
            on_the_clock_team: None,
        };
        assert!(infer_roster_config(&board).is_none());
    }

    #[test]
    fn infer_roster_config_empty_slots() {
        let board = DraftBoardData {
            teams: vec![DraftBoardTeam {
                team_id: "1".into(),
                team_name: "Team 1".into(),
                column: 0,
                is_my_team: false,
                is_on_the_clock: false,
                slots: vec![],
            }],
            on_the_clock_team: None,
        };
        assert!(infer_roster_config(&board).is_none());
    }

    // -----------------------------------------------------------------------
    // Matchup handler tests
    // -----------------------------------------------------------------------

    fn make_matchup_payload() -> MatchupStatePayload {
        MatchupStatePayload {
            matchup_period: 1,
            start_date: "2026-03-25".to_string(),
            end_date: "2026-04-05".to_string(),
            selected_day: "2026-03-26".to_string(),
            home_team: MatchupTeamPayload {
                name: "Bob Dole Experience".to_string(),
                record: "3-2-0".to_string(),
                matchup_score: "6-4-2".to_string(),
            },
            away_team: MatchupTeamPayload {
                name: "Certified! Smokified!".to_string(),
                record: "2-3-0".to_string(),
                matchup_score: "4-6-2".to_string(),
            },
            categories: vec![
                MatchupCategoryPayload {
                    stat_id: 20,
                    abbrev: "R".to_string(),
                    home_value: Some(5.0),
                    away_value: Some(3.0),
                    lower_is_better: false,
                },
                MatchupCategoryPayload {
                    stat_id: 5,
                    abbrev: "HR".to_string(),
                    home_value: Some(2.0),
                    away_value: Some(3.0),
                    lower_is_better: false,
                },
                MatchupCategoryPayload {
                    stat_id: 47,
                    abbrev: "ERA".to_string(),
                    home_value: Some(3.50),
                    away_value: Some(4.20),
                    lower_is_better: true,
                },
                MatchupCategoryPayload {
                    stat_id: 41,
                    abbrev: "WHIP".to_string(),
                    home_value: Some(1.20),
                    away_value: Some(1.20),
                    lower_is_better: true,
                },
            ],
            batting: MatchupSectionPayload {
                headers: vec!["AB".to_string(), "H".to_string(), "R".to_string()],
                players: vec![MatchupPlayerPayload {
                    slot: "C".to_string(),
                    name: "Ben Rice".to_string(),
                    team: "NYY".to_string(),
                    positions: vec!["1B".to_string(), "C".to_string()],
                    opponent: Some("@BOS".to_string()),
                    status: None,
                    stats: vec![Some(4.0), Some(1.0), Some(0.0)],
                }],
                totals: Some(vec![Some(29.0), Some(8.0), Some(5.0)]),
            },
            pitching: MatchupSectionPayload {
                headers: vec!["IP".to_string(), "K".to_string()],
                players: vec![
                    MatchupPlayerPayload {
                        slot: "SP".to_string(),
                        name: "Framber Valdez".to_string(),
                        team: "HOU".to_string(),
                        positions: vec!["SP".to_string()],
                        opponent: Some("@TEX".to_string()),
                        status: None,
                        stats: vec![Some(7.0), Some(8.0)],
                    },
                    MatchupPlayerPayload {
                        slot: "SP".to_string(),
                        name: "Tyler Glasnow".to_string(),
                        team: "LAD".to_string(),
                        positions: vec!["SP".to_string()],
                        opponent: Some("SD".to_string()),
                        status: None,
                        stats: vec![Some(6.0), Some(9.0)],
                    },
                    MatchupPlayerPayload {
                        slot: "RP".to_string(),
                        name: "Luke Weaver".to_string(),
                        team: "NYY".to_string(),
                        positions: vec!["RP".to_string()],
                        opponent: Some("@BOS".to_string()),
                        status: None,
                        stats: vec![Some(1.0), Some(2.0)],
                    },
                ],
                totals: Some(vec![Some(14.0), Some(19.0)]),
            },
        }
    }

    #[test]
    fn matchup_state_json_deserializes() {
        let json = r#"{
            "type": "MATCHUP_STATE",
            "timestamp": 1711500000,
            "payload": {
                "matchupPeriod": 1,
                "startDate": "2026-03-25",
                "endDate": "2026-04-05",
                "selectedDay": "2026-03-26",
                "homeTeam": {
                    "name": "Bob Dole Experience",
                    "record": "3-2-0",
                    "matchupScore": "6-4-2"
                },
                "awayTeam": {
                    "name": "Certified! Smokified!",
                    "record": "2-3-0",
                    "matchupScore": "4-6-2"
                },
                "categories": [
                    { "statId": 20, "abbrev": "R", "homeValue": 5.0, "awayValue": 3.0, "lowerIsBetter": false }
                ],
                "batting": { "headers": ["AB", "H"], "players": [], "totals": null },
                "pitching": { "headers": ["IP", "K"], "players": [], "totals": null }
            }
        }"#;

        let msg: crate::protocol::ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            crate::protocol::ExtensionMessage::MatchupState { timestamp, payload } => {
                assert_eq!(timestamp, 1711500000);
                assert_eq!(payload.matchup_period, 1);
                assert_eq!(payload.home_team.name, "Bob Dole Experience");
                assert_eq!(payload.away_team.name, "Certified! Smokified!");
                assert_eq!(payload.categories.len(), 1);
                assert_eq!(payload.categories[0].abbrev, "R");
            }
            other => panic!("Expected MatchupState, got {:?}", other),
        }
    }

    #[test]
    fn category_state_higher_is_better_winning() {
        assert_eq!(
            category_state(Some(5.0), Some(3.0), false),
            CategoryState::Winning
        );
    }

    #[test]
    fn category_state_higher_is_better_losing() {
        assert_eq!(
            category_state(Some(2.0), Some(3.0), false),
            CategoryState::Losing
        );
    }

    #[test]
    fn category_state_lower_is_better_winning() {
        // ERA: lower is better, my 3.50 < opp 4.20 => winning
        assert_eq!(
            category_state(Some(3.50), Some(4.20), true),
            CategoryState::Winning
        );
    }

    #[test]
    fn category_state_lower_is_better_losing() {
        // WHIP: lower is better, my 1.35 > opp 1.20 => losing
        assert_eq!(
            category_state(Some(1.35), Some(1.20), true),
            CategoryState::Losing
        );
    }

    #[test]
    fn category_state_tied() {
        assert_eq!(
            category_state(Some(1.20), Some(1.20), true),
            CategoryState::Tied
        );
        assert_eq!(
            category_state(Some(5.0), Some(5.0), false),
            CategoryState::Tied
        );
    }

    #[test]
    fn category_state_missing_values_are_tied() {
        // ESPN renders "--" for rate stats with zero denominator. A missing
        // value on either side (or both) must not credit a team with a win.
        assert_eq!(
            category_state(None, Some(3.00), true),
            CategoryState::Tied
        );
        assert_eq!(
            category_state(Some(3.00), None, true),
            CategoryState::Tied
        );
        assert_eq!(
            category_state(None, None, false),
            CategoryState::Tied
        );
    }

    #[test]
    fn payload_converts_to_snapshot_with_correct_states() {
        let payload = make_matchup_payload();

        // Manually convert categories to verify states
        let scores: Vec<crate::matchup::CategoryScore> = payload
            .categories
            .iter()
            .map(|cat| crate::matchup::CategoryScore {
                stat_abbrev: cat.abbrev.clone(),
                my_value: cat.home_value.unwrap_or(0.0),
                opp_value: cat.away_value.unwrap_or(0.0),
                state: category_state(cat.home_value, cat.away_value, cat.lower_is_better),
            })
            .collect();

        // R: 5 > 3, higher is better => Winning
        assert_eq!(scores[0].stat_abbrev, "R");
        assert_eq!(scores[0].state, CategoryState::Winning);

        // HR: 2 < 3, higher is better => Losing
        assert_eq!(scores[1].stat_abbrev, "HR");
        assert_eq!(scores[1].state, CategoryState::Losing);

        // ERA: 3.50 < 4.20, lower is better => Winning
        assert_eq!(scores[2].stat_abbrev, "ERA");
        assert_eq!(scores[2].state, CategoryState::Winning);

        // WHIP: 1.20 == 1.20, lower is better => Tied
        assert_eq!(scores[3].stat_abbrev, "WHIP");
        assert_eq!(scores[3].state, CategoryState::Tied);
    }

    #[test]
    fn parse_record_valid() {
        let record = parse_record("3-2-7");
        assert_eq!(record.wins, 3);
        assert_eq!(record.losses, 2);
        assert_eq!(record.ties, 7);
    }

    #[test]
    fn parse_record_zeros() {
        let record = parse_record("0-0-0");
        assert_eq!(record.wins, 0);
        assert_eq!(record.losses, 0);
        assert_eq!(record.ties, 0);
    }

    #[test]
    fn parse_record_invalid_falls_back_to_zeros() {
        let record = parse_record("invalid");
        assert_eq!(record.wins, 0);
        assert_eq!(record.losses, 0);
        assert_eq!(record.ties, 0);
    }

    #[test]
    fn abbreviate_team_name_multi_word() {
        assert_eq!(abbreviate_team_name("Bob Dole Experience"), "BDE");
    }

    #[test]
    fn abbreviate_team_name_single_word() {
        assert_eq!(abbreviate_team_name("Certified!"), "CER");
    }

    #[test]
    fn abbreviate_team_name_long() {
        // Truncates to 3 initials
        assert_eq!(abbreviate_team_name("A B C D E"), "ABC");
    }

    fn test_onboarding_manager() -> crate::onboarding::OnboardingManager<crate::onboarding::RealFileSystem> {
        let tmp = std::env::temp_dir().join(format!("wyncast_ws_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        crate::onboarding::OnboardingManager::new(tmp, crate::onboarding::RealFileSystem)
    }

    fn create_test_app_state(mode: crate::protocol::AppMode) -> AppState {
        let config = crate::config::Config::default();
        let roster = AppState::default_roster_config();
        let draft_state = crate::draft::state::DraftState::new(
            config.league.salary_cap,
            &roster,
        );
        let db = crate::db::Database::open(":memory:").expect("in-memory db");
        let (llm_tx, _llm_rx) = mpsc::channel(1);
        let llm_client = crate::llm::client::LlmClient::Disabled;
        AppState::new(
            config,
            draft_state,
            vec![],
            None,
            db,
            "test-draft".to_string(),
            llm_client,
            llm_tx,
            None,
            mode,
            test_onboarding_manager(),
            Some(AppState::default_roster_config()),
        )
    }

    #[tokio::test]
    async fn matchup_state_switches_to_matchup_mode() {
        let (ui_tx, mut ui_rx) = mpsc::channel(32);
        let payload = make_matchup_payload();
        let mut state = create_test_app_state(crate::protocol::AppMode::Draft);

        assert_eq!(state.app_mode, crate::protocol::AppMode::Draft);

        handle_matchup_state(&mut state, payload, &ui_tx).await;

        assert_eq!(state.app_mode, crate::protocol::AppMode::Matchup);
        assert!(state.matchup_snapshot.is_some());

        // Should receive ModeChanged followed by MatchupSnapshot
        let msg1 = ui_rx.recv().await.unwrap();
        assert!(matches!(msg1, UiUpdate::ModeChanged(crate::protocol::AppMode::Matchup)));

        let msg2 = ui_rx.recv().await.unwrap();
        assert!(matches!(msg2, UiUpdate::MatchupSnapshot(_)));
    }

    #[tokio::test]
    async fn matchup_state_no_mode_change_when_already_matchup() {
        let (ui_tx, mut ui_rx) = mpsc::channel(32);
        let payload = make_matchup_payload();
        let mut state = create_test_app_state(crate::protocol::AppMode::Matchup);

        handle_matchup_state(&mut state, payload, &ui_tx).await;

        // Should only receive MatchupSnapshot, not ModeChanged
        let msg = ui_rx.recv().await.unwrap();
        assert!(matches!(msg, UiUpdate::MatchupSnapshot(_)));

        // No more messages
        assert!(ui_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn full_state_sync_switches_from_matchup_to_draft() {
        let (ui_tx, mut ui_rx) = mpsc::channel(32);
        let mut state = create_test_app_state(crate::protocol::AppMode::Matchup);

        let ext_payload = crate::protocol::StateUpdatePayload {
            picks: vec![],
            current_nomination: None,
            my_team_id: None,
            teams: vec![],
            pick_count: None,
            total_picks: None,
            draft_id: None,
            source: None,
            draft_board: None,
            pick_history: None,
            team_id_mapping: None,
        };

        handle_full_state_sync(&mut state, ext_payload, &ui_tx).await;

        assert_eq!(state.app_mode, crate::protocol::AppMode::Draft);

        // First message should be ModeChanged(Draft)
        let msg = ui_rx.recv().await.unwrap();
        assert!(
            matches!(msg, UiUpdate::ModeChanged(crate::protocol::AppMode::Draft)),
            "expected ModeChanged(Draft), got {:?}",
            msg
        );
    }
}
