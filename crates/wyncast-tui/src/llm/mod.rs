// LLM integration: prompt construction.
// The client module moved to wyncast-llm; re-export it for backward compat.
// The prompt module moved to wyncast-baseball; re-export it for backward compat.

// Re-exports from wyncast-core, wyncast-llm, and wyncast-baseball
pub use wyncast_core::llm::provider;
pub use wyncast_llm::client;
pub use wyncast_baseball::llm::prompt;
