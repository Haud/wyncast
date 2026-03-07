use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::protocol::{LlmEvent, LlmStatus, LlmTaskKind, UiUpdate};

use super::AppState;

/// Handle an LLM streaming event.
///
/// Routes tokens and completions to the appropriate text buffer
/// based on the `kind` field carried by each event.
///
/// **Generation check**: Every event carries a generation counter set when
/// the task was spawned, and a kind identifying which task (Analysis or Plan).
/// If the event's generation doesn't match the appropriate generation counter
/// for its kind, it's a stale event from a cancelled task and is silently
/// discarded. This prevents leftover tokens from a cancelled task bleeding
/// into a newer one.
///
/// **Concurrent tasks**: Analysis and planning tasks now run independently.
/// Each has its own generation counter (`analysis_generation` /
/// `plan_generation`) and its own task handle. Spawning a new analysis task
/// no longer cancels the planning task, and vice versa.
///
/// **Mode reset on completion/error**: After an Analysis `Complete` or `Error`
/// event is processed, `llm_mode` is set back to `None`. Planning completion
/// does not touch `llm_mode` because the analysis mode may still be active.
pub(super) async fn handle_llm_event(
    state: &mut AppState,
    event: LlmEvent,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    // Extract generation and kind from the event.
    let (event_generation, kind) = match &event {
        LlmEvent::Token { generation, kind, .. } => (*generation, kind.clone()),
        LlmEvent::Complete { generation, kind, .. } => (*generation, kind.clone()),
        LlmEvent::Error { generation, kind, .. } => (*generation, kind.clone()),
    };

    // Check against the appropriate generation counter for this task type.
    let current_generation = match kind {
        LlmTaskKind::Analysis => state.analysis_generation,
        LlmTaskKind::Plan => state.plan_generation,
    };

    if event_generation != current_generation {
        debug!(
            "Discarding stale LLM event (kind: {:?}, event gen: {}, current gen: {})",
            kind, event_generation, current_generation
        );
        return;
    }

    match (kind, event) {
        (LlmTaskKind::Analysis, LlmEvent::Token { text, .. }) => {
            state.nomination_analysis_text.push_str(&text);
            state.nomination_analysis_status = LlmStatus::Streaming;
            let _ = ui_tx.send(UiUpdate::AnalysisToken(text)).await;
        }
        (LlmTaskKind::Analysis, LlmEvent::Complete { full_text, stop_reason, .. }) => {
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
        (LlmTaskKind::Analysis, LlmEvent::Error { message, .. }) => {
            warn!("LLM analysis error: {}", message);
            state.nomination_analysis_status = LlmStatus::Error;
            state.llm_mode = None;
            let _ = ui_tx.send(UiUpdate::AnalysisError(message)).await;
        }
        (LlmTaskKind::Plan, LlmEvent::Token { text, .. }) => {
            state.nomination_plan_text.push_str(&text);
            state.nomination_plan_status = LlmStatus::Streaming;
            let _ = ui_tx.send(UiUpdate::PlanToken(text)).await;
        }
        (LlmTaskKind::Plan, LlmEvent::Complete { full_text, stop_reason, .. }) => {
            let text = if stop_reason.as_deref() == Some("max_tokens") {
                format!("{full_text}\n\n[Response truncated due to token limit]")
            } else {
                full_text
            };
            state.nomination_plan_text = text.clone();
            state.nomination_plan_status = LlmStatus::Complete;
            let _ = ui_tx.send(UiUpdate::PlanComplete(text)).await;
        }
        (LlmTaskKind::Plan, LlmEvent::Error { message, .. }) => {
            warn!("LLM planning error: {}", message);
            state.nomination_plan_status = LlmStatus::Error;
            let _ = ui_tx.send(UiUpdate::PlanError(message)).await;
        }
    }
}
