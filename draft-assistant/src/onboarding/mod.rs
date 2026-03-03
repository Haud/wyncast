// Onboarding state management and first-run detection.
//
// Persists partial onboarding progress to `onboarding.toml` in the app data
// config directory so users can resume if interrupted mid-flow.
//
// All filesystem access goes through the [`FileSystem`] trait so tests can
// inject a fake implementation and avoid writing to disk.

pub mod fs;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

pub use fs::{FileSystem, RealFileSystem};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
// OnboardingManager
// ---------------------------------------------------------------------------

/// Manages loading and saving of [`OnboardingProgress`].
///
/// All filesystem access is delegated to the generic `F: FileSystem`
/// parameter, allowing tests to inject an in-memory fake.
pub struct OnboardingManager<F: FileSystem> {
    config_dir: PathBuf,
    fs: F,
}

impl<F: FileSystem> OnboardingManager<F> {
    /// Create a new manager for the given config directory and filesystem.
    pub fn new(config_dir: PathBuf, fs: F) -> Self {
        Self { config_dir, fs }
    }

    /// Returns the path to `onboarding.toml` inside the config directory.
    fn onboarding_toml_path(&self) -> PathBuf {
        self.config_dir.join("onboarding.toml")
    }

    /// Load onboarding progress from `onboarding.toml`.
    ///
    /// Returns `OnboardingProgress::default()` (step = `LlmSetup`) when the
    /// file does not exist or cannot be parsed.
    pub fn load_progress(&self) -> OnboardingProgress {
        let path = self.onboarding_toml_path();
        match self.fs.read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(progress) => progress,
                Err(e) => {
                    warn!("Failed to parse onboarding.toml, resetting to defaults: {e}");
                    OnboardingProgress::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => OnboardingProgress::default(),
            Err(e) => {
                warn!("Failed to read onboarding.toml, resetting to defaults: {e}");
                OnboardingProgress::default()
            }
        }
    }

    /// Save onboarding progress to `onboarding.toml`.
    ///
    /// Creates the config directory if it does not exist. Uses atomic
    /// write-to-temp-then-rename.
    pub fn save_progress(&self, progress: &OnboardingProgress) -> std::io::Result<()> {
        self.fs.create_dir_all(&self.config_dir)?;
        let path = self.onboarding_toml_path();
        let text = toml::to_string_pretty(progress)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let tmp_path = path.with_extension("toml.tmp");
        self.fs.write(&tmp_path, &text)?;
        self.fs.rename(&tmp_path, &path)
    }
}

// ---------------------------------------------------------------------------
// Convenience functions (backward-compatible public API)
// ---------------------------------------------------------------------------

/// Returns the config directory path derived from the OS app data directory.
fn default_config_dir() -> PathBuf {
    crate::app_dirs::app_data_dir().join("config")
}

/// Load onboarding progress from `onboarding.toml` in the app data config
/// directory.
///
/// Returns `OnboardingProgress::default()` (step = `LlmSetup`) when the file
/// does not exist or cannot be parsed.
pub fn load_onboarding_progress() -> OnboardingProgress {
    let manager = OnboardingManager::new(default_config_dir(), RealFileSystem);
    manager.load_progress()
}

/// Save onboarding progress to `onboarding.toml` in the app data config
/// directory.
///
/// Creates the config directory if it does not exist.
pub fn save_onboarding_progress(progress: &OnboardingProgress) -> std::io::Result<()> {
    let manager = OnboardingManager::new(default_config_dir(), RealFileSystem);
    manager.save_progress(progress)
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
///
/// This is a pure function with no I/O -- it only inspects the provided data.
pub fn is_configured(progress: &OnboardingProgress, credentials: &CredentialsConfig) -> bool {
    if progress.current_step != OnboardingStep::Complete {
        return false;
    }

    if !progress.strategy_configured {
        return false;
    }

    // Check that a non-empty model name has been selected.
    let model_valid = progress
        .llm_model
        .as_deref()
        .is_some_and(|m| !m.trim().is_empty());
    if !model_valid {
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

    key.is_some_and(|k| !k.trim().is_empty())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Mutex;

    // -----------------------------------------------------------------------
    // FakeFileSystem
    // -----------------------------------------------------------------------

    /// In-memory filesystem for tests. No disk I/O.
    struct FakeFileSystem {
        files: Mutex<HashMap<PathBuf, String>>,
    }

    impl FakeFileSystem {
        fn new() -> Self {
            Self {
                files: Mutex::new(HashMap::new()),
            }
        }

        /// Pre-populate a file for read tests.
        fn with_file(self, path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
            self.files
                .lock()
                .unwrap()
                .insert(path.into(), contents.into());
            self
        }

        /// Read back what was written (for assertions).
        fn get(&self, path: impl AsRef<Path>) -> Option<String> {
            self.files
                .lock()
                .unwrap()
                .get(path.as_ref())
                .cloned()
        }
    }

    impl FileSystem for FakeFileSystem {
        fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
            self.files
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))
        }

        fn write(&self, path: &Path, contents: &str) -> std::io::Result<()> {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), contents.to_string());
            Ok(())
        }

        fn create_dir_all(&self, _path: &Path) -> std::io::Result<()> {
            // No-op in the fake: directories are implicit.
            Ok(())
        }

        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            let mut files = self.files.lock().unwrap();
            let contents = files.remove(from).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "source not found")
            })?;
            files.insert(to.to_path_buf(), contents);
            Ok(())
        }
    }

    /// Helper: create a manager backed by a FakeFileSystem.
    fn fake_manager(fs: FakeFileSystem) -> OnboardingManager<FakeFileSystem> {
        OnboardingManager::new(PathBuf::from("/fake/config"), fs)
    }

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

    #[test]
    fn is_configured_false_when_whitespace_only_api_key() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some("claude-sonnet-4-6".to_string()),
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: Some("   \t\n".to_string()),
            google_api_key: None,
            openai_api_key: None,
        };
        assert!(!is_configured(&progress, &creds));
    }

    #[test]
    fn is_configured_false_when_llm_model_is_none() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Anthropic),
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
    fn is_configured_false_when_llm_model_is_empty() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some(String::new()),
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
    fn is_configured_false_when_llm_model_is_whitespace() {
        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some("   ".to_string()),
            strategy_configured: true,
        };
        let creds = CredentialsConfig {
            anthropic_api_key: Some("sk-ant-test-key".to_string()),
            google_api_key: None,
            openai_api_key: None,
        };
        assert!(!is_configured(&progress, &creds));
    }

    // -- save / load round-trip tests (all use FakeFileSystem) ---------------

    #[test]
    fn save_load_roundtrip_full_progress() {
        let manager = fake_manager(FakeFileSystem::new());

        let progress = OnboardingProgress {
            current_step: OnboardingStep::StrategySetup,
            llm_provider: Some(LlmProvider::Anthropic),
            llm_model: Some("claude-sonnet-4-6".to_string()),
            strategy_configured: false,
        };

        manager.save_progress(&progress).unwrap();
        let loaded = manager.load_progress();

        assert_eq!(loaded.current_step, OnboardingStep::StrategySetup);
        assert_eq!(loaded.llm_provider, Some(LlmProvider::Anthropic));
        assert_eq!(loaded.llm_model.as_deref(), Some("claude-sonnet-4-6"));
        assert!(!loaded.strategy_configured);
    }

    #[test]
    fn save_load_roundtrip_complete() {
        let manager = fake_manager(FakeFileSystem::new());

        let progress = OnboardingProgress {
            current_step: OnboardingStep::Complete,
            llm_provider: Some(LlmProvider::Google),
            llm_model: Some("gemini-2.5-pro".to_string()),
            strategy_configured: true,
        };

        manager.save_progress(&progress).unwrap();
        let loaded = manager.load_progress();

        assert_eq!(loaded.current_step, OnboardingStep::Complete);
        assert_eq!(loaded.llm_provider, Some(LlmProvider::Google));
        assert_eq!(loaded.llm_model.as_deref(), Some("gemini-2.5-pro"));
        assert!(loaded.strategy_configured);
    }

    #[test]
    fn save_load_roundtrip_minimal_progress() {
        let manager = fake_manager(FakeFileSystem::new());

        let progress = OnboardingProgress {
            current_step: OnboardingStep::LlmSetup,
            llm_provider: None,
            llm_model: None,
            strategy_configured: false,
        };

        manager.save_progress(&progress).unwrap();
        let loaded = manager.load_progress();

        assert_eq!(loaded.current_step, OnboardingStep::LlmSetup);
        assert!(loaded.llm_provider.is_none());
        assert!(loaded.llm_model.is_none());
        assert!(!loaded.strategy_configured);
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        // Empty fake FS -- no onboarding.toml
        let manager = fake_manager(FakeFileSystem::new());
        let loaded = manager.load_progress();

        assert_eq!(loaded.current_step, OnboardingStep::LlmSetup);
        assert!(loaded.llm_provider.is_none());
        assert!(loaded.llm_model.is_none());
        assert!(!loaded.strategy_configured);
    }

    #[test]
    fn load_returns_default_when_file_is_invalid_toml() {
        let fs = FakeFileSystem::new()
            .with_file("/fake/config/onboarding.toml", "not valid [[[ toml");
        let manager = fake_manager(fs);

        let loaded = manager.load_progress();
        assert_eq!(loaded.current_step, OnboardingStep::LlmSetup);
    }

    #[test]
    fn load_handles_partial_toml() {
        let fs = FakeFileSystem::new().with_file(
            "/fake/config/onboarding.toml",
            "current_step = \"strategy_setup\"\n",
        );
        let manager = fake_manager(fs);

        let loaded = manager.load_progress();
        assert_eq!(loaded.current_step, OnboardingStep::StrategySetup);
        assert!(loaded.llm_provider.is_none());
        assert!(loaded.llm_model.is_none());
        assert!(!loaded.strategy_configured);
    }

    #[test]
    fn save_writes_to_onboarding_toml() {
        let fs = FakeFileSystem::new();
        let manager = fake_manager(fs);

        let progress = OnboardingProgress::default();
        manager.save_progress(&progress).unwrap();

        // Verify the file was written at the expected path
        let contents = manager.fs.get("/fake/config/onboarding.toml");
        assert!(contents.is_some(), "onboarding.toml should exist after save");

        // The temp file should have been renamed away
        let tmp_contents = manager.fs.get("/fake/config/onboarding.toml.tmp");
        assert!(tmp_contents.is_none(), "temp file should not remain after rename");
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
