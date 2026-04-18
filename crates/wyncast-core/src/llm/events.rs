// LLM streaming event types (no HTTP/tokio deps).

/// Events produced by the LLM streaming client.
///
/// Each event carries a `generation` counter that identifies which LLM task
/// produced it. The app orchestrator increments the generation each time it
/// spawns a new LLM task, and discards events whose generation doesn't match
/// the current one. This prevents stale tokens from a cancelled task being
/// attributed to a newer analysis.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmEvent {
    /// A single token of streamed output.
    Token { text: String, generation: u64 },
    /// The LLM response is complete.
    Complete {
        full_text: String,
        input_tokens: u32,
        output_tokens: u32,
        /// The stop reason from the API (e.g. "end_turn" or "max_tokens").
        stop_reason: Option<String>,
        generation: u64,
    },
    /// An error occurred during LLM interaction.
    Error { message: String, generation: u64 },
}

impl LlmEvent {
    /// Extract the request ID (generation) from any event variant.
    pub fn request_id(&self) -> u64 {
        match self {
            LlmEvent::Token { generation, .. } => *generation,
            LlmEvent::Complete { generation, .. } => *generation,
            LlmEvent::Error { generation, .. } => *generation,
        }
    }
}
