use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::protocol::{LlmEvent, LlmStatus, UiUpdate};

use super::{AppState, LlmMode};

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
pub(super) async fn handle_llm_event(
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
