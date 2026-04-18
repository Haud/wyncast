// wyncast-llm: LLM client infrastructure (HTTP/SSE streaming).

pub mod client;

// Re-export commonly used types from wyncast-core for convenience
pub use wyncast_core::llm::events::LlmEvent;
pub use wyncast_core::llm::provider::{
    LlmProvider, ModelOption, ModelTier, SUPPORTED_MODELS, find_model, models_for_provider,
};
