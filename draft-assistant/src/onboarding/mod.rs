// Onboarding state management and first-run detection.
//
// Persists partial onboarding progress to `onboarding.toml` in the app data
// config directory so users can resume if interrupted mid-flow.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::CredentialsConfig;
use crate::llm::provider::LlmProvider;

// ---------------------------------------------------------------------------
// OnboardingStep
// ---------------------------------------------------------------------------

/// The current step in the onboarding wizard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStep {
    LlmSetup,
    StrategySetup,
    Complete,
}

impl Default for OnboardingStep {
    fn default() -> Self {
        OnboardingStep::LlmSetup
    }
}

// ---------------------------------------------------------------------------
// OnboardingProgress
// ---------------------------------------------------------------------------

/// Tracks partial onboarding progress. Persisted to `onboarding.toml`.
///
/// API keys are NOT stored here -- they go directly to `credentials.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingProgress {
    #[serde(default)]
    pub current_step: OnboardingStep,

    #[serde(default)]
    pub llm_provider: Option<LlmProvider>,

    #[serde(default)]
    pub llm_model: Option<String>,

    #[serde(default)]
    pub strategy_configured: bool,
}

impl Default for OnboardingProgress {
    fn default() -> Self {
        OnboardingProgress {
            current_step: OnboardingStep::LlmSetup,
            llm_provider: None,
            llm_model: None,
            strategy_configured: false,
        }
    }
}

// ---------------------------------------------------------------------------
// File path helper
// ---------------------------------------------------------------------------

/// Returns the path to `onboarding.toml` inside the given config directory.
///
/// The config directory is `<app_data_dir>/config/` (the same directory that
/// houses `league.toml`, `strategy.toml`, and `credentials.toml`).
fn onboarding_toml_path(config_dir: &Path) -> PathBuf {
    config_dir.join("onboarding.toml")
}

/// Returns the config directory path derived from the OS app data directory.
fn default_config_dir() -> PathBuf {
    crate::app_dirs::app_data_dir().join("config")
}

// ---------------------------------------------------------------------------
// Load / Save
// ---------------------------------------------------------------------------

/// Load onboarding progress from `onboarding.toml` in the app data config
/// directory.
///
/// Returns `OnboardingProgress::default()` (step = `LlmSetup`) when the file
/// does not exist or cannot be parsed.
pub fn load_onboarding_progress() -> OnboardingProgress {
    load_onboarding_progress_from(&default_config_dir())
}

/// Load onboarding progress from a specific config directory. Useful for
/// testing.
pub(crate) fn load_onboarding_progress_from(config_dir: &Path) -> OnboardingProgress {
    let path = onboarding_toml_path(config_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_default(),
        Err(_) => OnboardingProgress::default(),
    }
}

/// Save onboarding progress to `onboarding.toml` in the app data config
/// directory.
///
/// Creates the config directory if it does not exist.
pub fn save_onboarding_progress(progress: &OnboardingProgress) -> std::io::Result<()> {
    save_onboarding_progress_to(progress, &default_config_dir())
}

/// Save onboarding progress to a specific config directory. Useful for
/// testing.
pub(crate) fn save_onboarding_progress_to(
    progress: &OnboardingProgress,
    config_dir: &Path,
) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let path = onboarding_toml_path(config_dir);
    let text = toml::to_string_pretty(progress)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&path, text)
}

// ---------------------------------------------------------------------------
// is_configured
// ---------------------------------------------------------------------------

/// Returns `true` when onboarding is complete and the app is ready to run:
///
/// 1. `current_step == Complete`
/// 2. The selected LLM provider has a non-empty API key in credentials
/// 3. Strategy has been configured (i.e. `strategy_configured == true`)
///
/// This is the main gate for deciding whether to show the onboarding wizard
/// or proceed directly to the draft view.
pub fn is_configured(progress: &OnboardingProgress, credentials: &CredentialsConfig) -> bool {
    if progress.current_step != OnboardingStep::Complete {
        return false;
    }

    if !progress.strategy_configured {
        return false;
    }

    // Check that the selected provider has a non-empty API key.
    let Some(ref provider) = progress.llm_provider else {
        return false;
    };

    has_api_key_for_provider(provider, credentials)
}

/// Check whether the given provider has a non-empty API key in credentials.
fn has_api_key_for_provider(provider: &LlmProvider, credentials: &CredentialsConfig) -> bool {
    let key = match provider {
        LlmProvider::Anthropic => credentials.anthropic_api_key.as_deref(),
        LlmProvider::Google => credentials.google_api_key.as_deref(),
        LlmProvider::OpenAI => credentials.openai_api_key.as_deref(),
    };

    key.is_some_and(|k| !k.is_empty())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- is_configured tests ------------------------------------------------

    #[test]
    fn is_configured_false_when_step_not_complete() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::LlmSetup,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some("claude-sonnet-4-6".to_string()),
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: Some("sk-ant-test-key".to_string()),
            google_api_key: None,
            openai_api_key: None,
        };
        assert!(!is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_false_when_step_strategy_setup() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::StrategySetup,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some("claude-sonnet-4-6".to_string()),
            strategy_configured: false,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: Some("sk-ant-test-key".to_string()),
            google_api_key: None,
            openai_api_key: None,
        };
        assert!(!is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_false_when_complete_but_no_api_key() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some("claude-sonnet-4-6".to_string()),
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: None,
            google_api_key: None,
            openai_api_key: None,
        };
        assert!(!is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_false_when_complete_but_empty_api_key() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Google),
            llm_model: Some("gemini-2.0-flash".to_string()),
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: None,
            google_api_key: Some(String::new()),
            openai_api_key: None,
        };
        assert!(!is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_false_when_complete_but_no_provider() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: None,
            llm_model: None,
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: Some("sk-ant-test-key".to_string()),
            google_api_key: None,
            openai_api_key: None,
        };
        assert!(!is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_false_when_complete_but_strategy_not_configured() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some("claude-sonnet-4-6".to_string()),
            strategy_configured: false,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: Some("sk-ant-test-key".to_string()),
            google_api_key: None,
            openai_api_key: None,
        };
        assert!(!is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_true_when_complete_with_anthropic_key() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some("claude-sonnet-4-6".to_string()),
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: Some("sk-ant-test-key".to_string()),
            google_api_key: None,
            openai_api_key: None,
        };
        assert!(is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_true_when_complete_with_google_key() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Google),
            llm_model: Some("gemini-2.5-pro".to_string()),
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: None,
            google_api_key: Some("google-key-123".to_string()),
            openai_api_key: None,
        };
        assert!(is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_true_when_complete_with_openai_key() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::OpenAI),
            llm_model: Some("gpt-4.1".to_string()),
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: None,
            google_api_key: None,
            openai_api_key: Some("sk-openai-test-key".to_string()),
        };
        assert!(is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_false_when_wrong_provider_key() {
        // Provider is OpenAI but only Anthropic key is set
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::OpenAI),
            llm_model: Some("gpt-4.1".to_string()),
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: Some("sk-ant-test-key".to_string()),
            google_api_key: None,
            openai_api_key: None,
        };
        assert!(!is_configured(&progress, &creds));
    }

    // -- save / load round-trip tests ---------------------------------------

    #[test]
    fn save_load_roundtrip_full_progress() {
        let tmp = std::env::temp_dir().join("onboarding_test_roundtrip");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let progress = OnboardingProgress {
            current_step: OnboardingStep::StrategySetup,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some("claude-sonnet-4-6".to_string()),
            strategy_configured: false,
        };

        save_onboarding_progress_to(&progress, &tmp).unwrap();
        let loaded = load_onboarding_progress_from(&tmp);

        assert_eq!(loaded.current_step, OnboardingStep::StrategySetup);
        assert_eq!(loaded.llm_provider, Some(LlmProvider::Anthropic));
        assert_eq!(
            loaded.llm_model.as_deref(),
            Some("claude-sonnet-4-6")
        );
        assert!(!loaded.strategy_configured);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn save_load_roundtrip_complete() {
        let tmp = std::env::temp_dir().join("onboarding_test_roundtrip_complete");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Google),
            llm_model: Some("gemini-2.5-pro".to_string()),
            strategy_configured: true,
        };

        save_onboarding_progress_to(&progress, &tmp).unwrap();
        let loaded = load_onboarding_progress_from(&tmp);

        assert_eq!(loaded.current_step, OnboardingStep::Complete);
        assert_eq!(loaded.llm_provider, Some(LlmProvider::Google));
        assert_eq!(loaded.llm_model.as_deref(), Some("gemini-2.5-pro"));
        assert!(loaded.strategy_configured);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn save_load_roundtrip_minimal_progress() {
        let tmp = std::env::temp_dir().join("onboarding_test_roundtrip_minimal");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let progress = OnboardingProgress {
            current_step: OnboardingStep::LlmSetup,
            llm_provider: None,
            llm_model: None,
            strategy_configured: false,
        };

        save_onboarding_progress_to(&progress, &tmp).unwrap();
        let loaded = load_onboarding_progress_from(&tmp);

        assert_eq!(loaded.current_step, OnboardingStep::LlmSetup);
        assert!(loaded.llm_provider.is_none());
        assert!(loaded.llm_model.is_none());
        assert!(!loaded.strategy_configured);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        let tmp = std::env::temp_dir().join("onboarding_test_missing_file");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // No onboarding.toml in the directory
        let loaded = load_onboarding_progress_from(&tmp);

        assert_eq!(loaded.current_step, OnboardingStep::LlmSetup);
        assert!(loaded.llm_provider.is_none());
        assert!(loaded.llm_model.is_none());
        assert!(!loaded.strategy_configured);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_returns_default_when_directory_missing() {
        let tmp = std::env::temp_dir().join("onboarding_test_no_dir_at_all");
        let _ = std::fs::remove_dir_all(&tmp);

        // Directory does not exist
        let loaded = load_onboarding_progress_from(&tmp);

        assert_eq!(loaded.current_step, OnboardingStep::LlmSetup);
        assert!(loaded.llm_provider.is_none());
    }

    #[test]
    fn load_returns_default_when_file_is_invalid_toml() {
        let tmp = std::env::temp_dir().join("onboarding_test_invalid_toml");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        std::fs::write(tmp.join("onboarding.toml"), "not valid [[[ toml").unwrap();

        let loaded = load_onboarding_progress_from(&tmp);
        assert_eq!(loaded.current_step, OnboardingStep::LlmSetup);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_handles_partial_toml() {
        // A file with only some fields should still load, filling in defaults
        let tmp = std::env::temp_dir().join("onboarding_test_partial_toml");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        std::fs::write(
            tmp.join("onboarding.toml"),
            "current_step = \"strategy_setup\"\n",
        )
        .unwrap();

        let loaded = load_onboarding_progress_from(&tmp);
        assert_eq!(loaded.current_step, OnboardingStep::StrategySetup);
        assert!(loaded.llm_provider.is_none());
        assert!(loaded.llm_model.is_none());
        assert!(!loaded.strategy_configured);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn save_creates_directory_if_missing() {
        let tmp = std::env::temp_dir().join("onboarding_test_create_dir");
        let nested = tmp.join("nested").join("config");
        let _ = std::fs::remove_dir_all(&tmp);

        let progress = OnboardingProgress::default();
        save_onboarding_progress_to(&progress, &nested).unwrap();

        assert!(nested.join("onboarding.toml").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -- OnboardingStep default test ----------------------------------------

    #[test]
    fn onboarding_step_default_is_llm_setup() {
        assert_eq!(OnboardingStep::default(), OnboardingStep::LlmSetup);
    }

    // -- OnboardingProgress default test ------------------------------------

    #[test]
    fn onboarding_progress_default() {
        let p = OnboardingProgress::default();
        assert_eq!(p.current_step, OnboardingStep::LlmSetup);
        assert!(p.llm_provider.is_none());
        assert!(p.llm_model.is_none());
        assert!(!p.strategy_configured);
    }
}
