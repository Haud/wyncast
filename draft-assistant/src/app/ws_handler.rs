use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::db::Database;
use crate::draft::state::{
    compute_state_diff, ActiveNomination, DraftState, NominationPayload, ReconcileResult,
    StateUpdatePayload,
};
use crate::protocol::{
    ExtensionMessage, LlmStatus, NominationInfo, UiUpdate,
};
use crate::valuation;
use crate::valuation::analysis::CategoryNeeds;
use crate::valuation::auction::InflationTracker;
use crate::valuation::scarcity::compute_scarcity;

use super::AppState;

/// Handle an incoming WebSocket message (JSON from the extension).
pub(super) async fn handle_ws_message(
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
    info!(
        "Received FULL_STATE_SYNC with {} picks — resetting draft state",
        ext_payload.picks.len()
    );

    // Detect if the incoming nomination is the same player as what's currently
    // being analyzed. The extension sends FULL_STATE_SYNC every 10 seconds as
    // a periodic keyframe; if the nomination is unchanged, we should NOT cancel
    // the in-progress LLM analysis or allow it to restart.
    let incoming_nom = ext_payload.current_nomination.as_ref();
    let preserve_llm = match (&state.llm_mode, incoming_nom) {
        (Some(super::LlmMode::NominationAnalysis { player_name, player_id, .. }), Some(inc)) => {
            if !player_id.is_empty() && !inc.player_id.is_empty() {
                player_id == &inc.player_id
            } else {
                player_name == &inc.player_name
            }
        }
        _ => false,
    };

    // Save current nomination before resetting so we can restore it (and use
    // it as a stub baseline for compute_state_diff) when the nomination is
    // unchanged.
    let saved_nomination = state.draft_state.current_nomination.clone();

    // Reset in-memory draft state so the snapshot is applied from scratch.
    // Preserve salary_cap and roster_config (stored inside DraftState).
    state.draft_state = DraftState::new(
        state.config.league.salary_cap,
        &state.config.league.roster,
    );

    // Reset valuation pool and derived state so they're rebuilt cleanly
    // after all snapshot picks are applied.
    state.available_players =
        valuation::compute_initial(&state.all_projections, &state.config)
            .unwrap_or_default();
    state.scarcity = compute_scarcity(&state.available_players, &state.config.league);
    state.inflation = InflationTracker::new();
    state.category_needs = CategoryNeeds::default();

    if preserve_llm {
        // Same nomination is still active: keep the LLM task and mode so
        // streaming continues uninterrupted.
        //
        // Build a stub previous state for compute_state_diff.  Empty picks
        // ensures all snapshot picks are treated as new (correct for rebuild).
        // Preserved nomination prevents compute_state_diff from treating the
        // same player as a new nomination (which would fire NominationUpdate
        // and restart the LLM analysis).
        state.previous_extension_state = saved_nomination.as_ref().map(|nom| StateUpdatePayload {
            picks: vec![],
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
    } else {
        // Nomination changed or no active analysis: clear all LLM state so
        // handle_state_update can start fresh.
        state.previous_extension_state = None;
        state.cancel_llm_task();
        state.llm_mode = None;
        state.nomination_analysis_text.clear();
        state.nomination_analysis_status = LlmStatus::Idle;
        state.nomination_plan_text.clear();
        state.nomination_plan_status = LlmStatus::Idle;
    }

    // Now process the snapshot as a regular state update.  When preserve_llm
    // is true, compute_state_diff will see the stub previous state and treat
    // the nomination as unchanged, so nomination_changed will not fire and
    // NominationUpdate will not be sent to the TUI.
    handle_state_update(state, ext_payload, ui_tx).await;

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
