// LLM provider abstractions: provider enum, model catalog, and model tiers.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// LlmProvider
// ---------------------------------------------------------------------------

/// The supported LLM provider backends.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    Anthropic,
    Google,
    OpenAI,
}

impl LlmProvider {
    /// Human-readable display name for the provider.
    pub fn display_name(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "Anthropic Claude",
            LlmProvider::Google => "Google Gemini",
            LlmProvider::OpenAI => "OpenAI",
        }
    }
}

impl std::fmt::Display for LlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

// ---------------------------------------------------------------------------
// ModelTier
// ---------------------------------------------------------------------------

/// Performance / cost tier classification for a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    /// High-capability model optimised for complex reasoning.
    Thinking,
    /// Balanced speed-and-quality model for everyday use.
    Fast,
    /// Low-cost model suited for high-volume or simple tasks.
    Cheap,
}

impl std::fmt::Display for ModelTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ModelTier::Thinking => "Thinking",
            ModelTier::Fast => "Fast",
            ModelTier::Cheap => "Cheap",
        };
        write!(f, "{s}")
    }
}

// ---------------------------------------------------------------------------
// ModelOption
// ---------------------------------------------------------------------------

/// A single selectable model in the model catalog.
#[derive(Debug, Clone)]
pub struct ModelOption {
    pub provider: LlmProvider,
    /// The model identifier used in API requests.
    pub model_id: &'static str,
    /// Short human-readable name shown in the UI.
    pub display_name: &'static str,
    pub tier: ModelTier,
}

// ---------------------------------------------------------------------------
// Static model catalog
// ---------------------------------------------------------------------------

/// All models supported by the application, in display order.
pub const SUPPORTED_MODELS: &[ModelOption] = &[
    // Anthropic
    ModelOption {
        provider: LlmProvider::Anthropic,
        model_id: "claude-opus-4-6",
        display_name: "Claude Opus 4.6",
        tier: ModelTier::Thinking,
    },
    ModelOption {
        provider: LlmProvider::Anthropic,
        model_id: "claude-sonnet-4-6",
        display_name: "Claude Sonnet 4.6",
        tier: ModelTier::Fast,
    },
    // Google
    ModelOption {
        provider: LlmProvider::Google,
        model_id: "gemini-2.5-pro",
        display_name: "Gemini 2.5 Pro",
        tier: ModelTier::Thinking,
    },
    ModelOption {
        provider: LlmProvider::Google,
        model_id: "gemini-2.0-flash",
        display_name: "Gemini 2.0 Flash",
        tier: ModelTier::Fast,
    },
    // OpenAI
    ModelOption {
        provider: LlmProvider::OpenAI,
        model_id: "gpt-4.1",
        display_name: "GPT-4.1",
        tier: ModelTier::Thinking,
    },
    ModelOption {
        provider: LlmProvider::OpenAI,
        model_id: "gpt-4o",
        display_name: "GPT-4o",
        tier: ModelTier::Fast,
    },
];

/// Return models available for a given provider.
pub fn models_for_provider(provider: &LlmProvider) -> Vec<&'static ModelOption> {
    SUPPORTED_MODELS
        .iter()
        .filter(|m| &m.provider == provider)
        .collect()
}

/// Look up a model option by provider and model ID.
pub fn find_model(provider: &LlmProvider, model_id: &str) -> Option<&'static ModelOption> {
    SUPPORTED_MODELS
        .iter()
        .find(|m| &m.provider == provider && m.model_id == model_id)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_providers_have_at_least_two_models() {
        for provider in [LlmProvider::Anthropic, LlmProvider::Google, LlmProvider::OpenAI] {
            let models = models_for_provider(&provider);
            assert!(
                models.len() >= 2,
                "{} should have at least 2 models, found {}",
                provider.display_name(),
                models.len()
            );
        }
    }

    #[test]
    fn find_model_returns_correct_entry() {
        let m = find_model(&LlmProvider::Anthropic, "claude-opus-4-6").unwrap();
        assert_eq!(m.model_id, "claude-opus-4-6");
        assert_eq!(m.tier, ModelTier::Thinking);
    }

    #[test]
    fn find_model_returns_none_for_unknown() {
        assert!(find_model(&LlmProvider::Anthropic, "does-not-exist").is_none());
    }

    #[test]
    fn provider_display_name() {
        assert_eq!(LlmProvider::Anthropic.display_name(), "Anthropic Claude");
        assert_eq!(LlmProvider::Google.display_name(), "Google Gemini");
        assert_eq!(LlmProvider::OpenAI.display_name(), "OpenAI");
    }

    #[test]
    fn model_tier_display() {
        assert_eq!(ModelTier::Thinking.to_string(), "Thinking");
        assert_eq!(ModelTier::Fast.to_string(), "Fast");
        assert_eq!(ModelTier::Cheap.to_string(), "Cheap");
    }

    #[test]
    fn provider_serde_roundtrip() {
        let json = serde_json::to_string(&LlmProvider::Anthropic).unwrap();
        assert_eq!(json, r#""anthropic""#);
        let back: LlmProvider = serde_json::from_str(&json).unwrap();
        assert_eq!(back, LlmProvider::Anthropic);
    }

    #[test]
    fn provider_serde_roundtrip_google() {
        let json = serde_json::to_string(&LlmProvider::Google).unwrap();
        assert_eq!(json, r#""google""#);
        let back: LlmProvider = serde_json::from_str(&json).unwrap();
        assert_eq!(back, LlmProvider::Google);
    }

    #[test]
    fn provider_serde_roundtrip_openai() {
        let json = serde_json::to_string(&LlmProvider::OpenAI).unwrap();
        assert_eq!(json, r#""openai""#);
        let back: LlmProvider = serde_json::from_str(&json).unwrap();
        assert_eq!(back, LlmProvider::OpenAI);
    }
}
