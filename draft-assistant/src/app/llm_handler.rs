use tokio::sync::mpsc;
use tracing::debug;

use crate::protocol::{LlmEvent, LlmStreamUpdate, UiUpdate};

use super::AppState;

/// Handle an LLM streaming event.
///
/// Validates the event against the request manager, converts it to
/// a generic `LlmStreamUpdate`, and sends a single `UiUpdate::LlmUpdate`.
/// No mode matching, no text buffering on AppState.
pub(super) async fn handle_llm_event(
    state: &mut AppState,
    event: LlmEvent,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    let request_id = event.request_id();

    if !state.llm_requests.is_active(request_id) {
        debug!(
            "Discarding stale LLM event (request_id: {})",
            request_id
        );
        return;
    }

    let (update, is_terminal) = match event {
        LlmEvent::Token { text, .. } => {
            (LlmStreamUpdate::Token(text), false)
        }
        LlmEvent::Complete { full_text, stop_reason, .. } => {
            let text = if stop_reason.as_deref() == Some("max_tokens") {
                format!("{full_text}\n\n[Response truncated due to token limit]")
            } else {
                full_text
            };
            (LlmStreamUpdate::Complete(text), true)
        }
        LlmEvent::Error { message, .. } => {
            (LlmStreamUpdate::Error(message), true)
        }
    };

    if is_terminal {
        state.llm_requests.complete(request_id);
    }

    let _ = ui_tx.send(UiUpdate::LlmUpdate { request_id, update }).await;
}
