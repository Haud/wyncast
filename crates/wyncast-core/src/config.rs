// Configuration loading and parsing (league.toml, strategy.toml, credentials.toml).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::llm::provider::LlmProvider;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("failed to parse config file {path}: {source}")]
    ParseError {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("validation error for field `{field}`: {message}")]
    ValidationError { field: String, message: String },

    #[error("failed to initialize default config files: {message}")]
    DefaultsWriteError { message: String },
}

// ---------------------------------------------------------------------------
// Top-level assembled Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Config {
    pub league: LeagueConfig,
    pub strategy: StrategyConfig,
    pub credentials: CredentialsConfig,
    pub ws_port: u16,
    pub data_paths: DataPaths,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            league: LeagueConfig::default(),
            strategy: StrategyConfig::default(),
            credentials: CredentialsConfig::default(),
            ws_port: 9001,
            data_paths: DataPaths::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// league.toml structs
// ---------------------------------------------------------------------------

/// Wrapper for the top-level `[league]` table in league.toml.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct LeagueFile {
    league: LeagueConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LeagueConfig {
    pub name: String,
    pub platform: String,
    pub num_teams: usize,
    pub scoring_type: String,
    pub salary_cap: u32,
    pub batting_categories: CategoriesSection,
    pub pitching_categories: CategoriesSection,
    pub roster_limits: RosterLimits,
    /// Static team definitions (optional). Teams are now populated dynamically
    /// from ESPN's live draft data via the extension.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub teams: HashMap<String, String>,
}

impl Default for LeagueConfig {
    fn default() -> Self {
        Self {
            name: "Wyndham Lewis Vorticist Baseball".to_string(),
            platform: "espn".to_string(),
            num_teams: 10,
            scoring_type: "h2h_most_categories".to_string(),
            salary_cap: 260,
            batting_categories: CategoriesSection {
                categories: vec![
                    "R".to_string(),
                    "HR".to_string(),
                    "RBI".to_string(),
                    "BB".to_string(),
                    "SB".to_string(),
                    "AVG".to_string(),
                ],
            },
            pitching_categories: CategoriesSection {
                categories: vec![
                    "K".to_string(),
                    "W".to_string(),
                    "SV".to_string(),
                    "HD".to_string(),
                    "ERA".to_string(),
                    "WHIP".to_string(),
                ],
            },
            roster_limits: RosterLimits::default(),
            teams: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CategoriesSection {
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RosterLimits {
    pub max_sp: usize,
    pub max_rp: usize,
    pub gs_per_week: usize,
}

impl Default for RosterLimits {
    fn default() -> Self {
        Self {
            max_sp: 7,
            max_rp: 7,
            gs_per_week: 7,
        }
    }
}

// ---------------------------------------------------------------------------
// strategy.toml structs
// ---------------------------------------------------------------------------

/// Raw deserialization target for the entire strategy.toml file.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct StrategyFile {
    budget: BudgetSection,
    category_weights: CategoryWeights,
    pool: PoolConfig,
    llm: LlmConfig,
    websocket: WebsocketSection,
    #[serde(default, skip_serializing_if = "DataPaths::is_empty")]
    data_paths: DataPaths,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    strategy_overview: Option<String>,
}

impl Default for StrategyFile {
    fn default() -> Self {
        let strategy = StrategyConfig::default();
        Self {
            budget: BudgetSection {
                hitting_budget_fraction: strategy.hitting_budget_fraction,
            },
            category_weights: strategy.weights,
            pool: strategy.pool,
            llm: strategy.llm,
            websocket: WebsocketSection { port: 9001 },
            data_paths: DataPaths::default(),
            strategy_overview: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BudgetSection {
    hitting_budget_fraction: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WebsocketSection {
    port: u16,
}

/// The public strategy config assembled from the strategy.toml sections.
#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub hitting_budget_fraction: f64,
    pub weights: CategoryWeights,
    pub pool: PoolConfig,
    pub llm: LlmConfig,
    /// Prose overview of the user's draft strategy, generated by the LLM
    /// during onboarding. Included in draft-time LLM prompts for context.
    pub strategy_overview: Option<String>,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            hitting_budget_fraction: 0.65,
            weights: CategoryWeights::default(),
            pool: PoolConfig::default(),
            llm: LlmConfig::default(),
            strategy_overview: None,
        }
    }
}

/// Category weight multipliers, keyed by stat abbreviation (e.g. "R", "HR", "ERA").
///
/// Wraps a `HashMap<String, f64>` so leagues with non-standard categories
/// (OPS, QS, K/9, etc.) work without code changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryWeights(pub HashMap<String, f64>);

impl CategoryWeights {
    /// Look up the weight for a category. Returns `Some(value)` if present.
    pub fn get(&self, abbrev: &str) -> Option<f64> {
        self.0.get(abbrev).copied()
    }

    /// Returns the weight for a category, or 0.0 if missing.
    pub fn weight(&self, abbrev: &str) -> f64 {
        self.0.get(abbrev).copied().unwrap_or(0.0)
    }

    /// Create from an iterator of (name, value) pairs.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (impl Into<String>, f64)>) -> Self {
        Self(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// Iterate over all (category, weight) entries.
    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> + '_ {
        self.0.iter().map(|(k, v)| (k.as_str(), *v))
    }
}

impl Default for CategoryWeights {
    fn default() -> Self {
        Self::from_pairs([
            ("R", 1.0), ("HR", 1.0), ("RBI", 1.0), ("BB", 1.0),
            ("SB", 1.0), ("AVG", 1.0), ("K", 1.0), ("W", 1.0),
            ("SV", 0.7), ("HD", 1.0), ("ERA", 1.0), ("WHIP", 1.0),
        ])
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PoolConfig {
    pub min_pa: usize,
    pub min_ip_sp: f64,
    pub min_g_rp: usize,
    pub hitter_pool_size: usize,
    pub sp_pool_size: usize,
    pub rp_pool_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_pa: 200,
            min_ip_sp: 50.0,
            min_g_rp: 20,
            hitter_pool_size: 150,
            sp_pool_size: 70,
            rp_pool_size: 80,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    /// Which LLM backend to use.  Defaults to `anthropic` for backwards
    /// compatibility with existing strategy.toml files that predate this field.
    #[serde(default = "default_llm_provider")]
    pub provider: LlmProvider,
    pub model: String,
    pub analysis_max_tokens: u32,
    pub planning_max_tokens: u32,
    pub analysis_trigger: String,
    pub prefire_planning: bool,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProvider::Anthropic,
            model: "claude-sonnet-4-6".to_string(),
            analysis_max_tokens: 2048,
            planning_max_tokens: 2048,
            analysis_trigger: "nomination".to_string(),
            prefire_planning: true,
        }
    }
}

fn default_llm_provider() -> LlmProvider {
    LlmProvider::Anthropic
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[derive(Default)]
pub struct DataPaths {
    pub hitters: Option<String>,
    pub pitchers: Option<String>,
}


impl DataPaths {
    /// Returns true if both paths are None (no CSV overrides configured).
    pub fn is_empty(&self) -> bool {
        self.hitters.is_none() && self.pitchers.is_none()
    }
}

// ---------------------------------------------------------------------------
// credentials.toml structs
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize, serde::Serialize, Default)]
pub struct CredentialsConfig {
    pub anthropic_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub openai_api_key: Option<String>,
}

impl std::fmt::Debug for CredentialsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialsConfig")
            .field("anthropic_api_key", &self.anthropic_api_key.as_ref().map(|_| "[REDACTED]"))
            .field("google_api_key", &self.google_api_key.as_ref().map(|_| "[REDACTED]"))
            .field("openai_api_key", &self.openai_api_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Loading logic
// ---------------------------------------------------------------------------

/// Load and validate configuration from `config/league.toml`,
/// `config/strategy.toml`, and (optionally) `config/credentials.toml`,
/// all relative to the given `base_dir`.
///
/// This is the lower-level loading primitive that does not auto-copy defaults.
/// Prefer `load_config()` which handles default initialization automatically.
pub(crate) fn load_config_from(base_dir: &Path) -> Result<Config, ConfigError> {
    let config_dir = base_dir.join("config");

    // --- league.toml (required) ---
    let league_path = config_dir.join("league.toml");
    let league_text = read_file(&league_path)?;
    let league_file: LeagueFile =
        toml::from_str(&league_text).map_err(|e| ConfigError::ParseError {
            path: league_path.clone(),
            source: e,
        })?;
    let league = league_file.league;

    // --- strategy.toml (required) ---
    let strategy_path = config_dir.join("strategy.toml");
    let strategy_text = read_file(&strategy_path)?;
    let strategy_file: StrategyFile =
        toml::from_str(&strategy_text).map_err(|e| ConfigError::ParseError {
            path: strategy_path.clone(),
            source: e,
        })?;

    let strategy = StrategyConfig {
        hitting_budget_fraction: strategy_file.budget.hitting_budget_fraction,
        weights: strategy_file.category_weights,
        pool: strategy_file.pool,
        llm: strategy_file.llm,
        strategy_overview: strategy_file.strategy_overview,
    };

    let ws_port = strategy_file.websocket.port;
    let data_paths = strategy_file.data_paths;

    // --- credentials.toml (optional) ---
    let credentials_path = config_dir.join("credentials.toml");
    let credentials = if credentials_path.exists() {
        let cred_text = read_file(&credentials_path)?;
        toml::from_str(&cred_text).map_err(|e| ConfigError::ParseError {
            path: credentials_path.clone(),
            source: e,
        })?
    } else {
        CredentialsConfig::default()
    };

    let config = Config {
        league,
        strategy,
        credentials,
        ws_port,
        data_paths,
    };

    validate(&config)?;

    Ok(config)
}

/// Ensure that `league.toml` and `strategy.toml` exist in `<base_dir>/config/`.
///
/// For each missing file, the in-code `Default` impls are serialized to TOML
/// and written out. Files that already exist are left untouched.
///
/// Returns the list of files that were newly created.
pub fn ensure_default_config_files(base_dir: &Path) -> Result<Vec<PathBuf>, ConfigError> {
    let config_dir = base_dir.join("config");

    std::fs::create_dir_all(&config_dir).map_err(|e| ConfigError::DefaultsWriteError {
        message: format!("failed to create config directory: {e}"),
    })?;

    let mut created = Vec::new();

    // --- league.toml ---
    let league_path = config_dir.join("league.toml");
    {
        let league_file = LeagueFile {
            league: LeagueConfig::default(),
        };
        let text = toml::to_string_pretty(&league_file).map_err(|e| {
            ConfigError::DefaultsWriteError {
                message: format!("failed to serialize default league config: {e}"),
            }
        })?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&league_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(text.as_bytes()).map_err(|e| {
                    ConfigError::DefaultsWriteError {
                        message: format!("failed to write {}: {e}", league_path.display()),
                    }
                })?;
                created.push(league_path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // File already exists — skip silently.
            }
            Err(e) => {
                return Err(ConfigError::DefaultsWriteError {
                    message: format!("failed to create {}: {e}", league_path.display()),
                });
            }
        }
    }

    // --- strategy.toml ---
    let strategy_path = config_dir.join("strategy.toml");
    {
        let strategy_file = StrategyFile::default();
        let text = toml::to_string_pretty(&strategy_file).map_err(|e| {
            ConfigError::DefaultsWriteError {
                message: format!("failed to serialize default strategy config: {e}"),
            }
        })?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&strategy_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(text.as_bytes()).map_err(|e| {
                    ConfigError::DefaultsWriteError {
                        message: format!("failed to write {}: {e}", strategy_path.display()),
                    }
                })?;
                created.push(strategy_path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // File already exists — skip silently.
            }
            Err(e) => {
                return Err(ConfigError::DefaultsWriteError {
                    message: format!("failed to create {}: {e}", strategy_path.display()),
                });
            }
        }
    }

    Ok(created)
}

/// Convenience wrapper: loads config from the OS-standard app data directory.
///
/// Config files live in `<app_data_dir>/config/` (e.g. `~/.local/share/wyncast/config/`).
/// If `league.toml` or `strategy.toml` do not yet exist, they are written from
/// in-code default values.
pub fn load_config() -> Result<Config, ConfigError> {
    let data_dir = crate::app_dirs::app_data_dir();

    ensure_default_config_files(&data_dir)?;
    load_config_from(&data_dir)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_file(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|_| ConfigError::FileNotFound {
        path: path.to_path_buf(),
    })
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate(config: &Config) -> Result<(), ConfigError> {
    // League validations
    if config.league.num_teams == 0 {
        return Err(ConfigError::ValidationError {
            field: "league.num_teams".into(),
            message: "must be greater than 0".into(),
        });
    }

    if config.league.salary_cap == 0 {
        return Err(ConfigError::ValidationError {
            field: "league.salary_cap".into(),
            message: "must be greater than 0".into(),
        });
    }

    // Strategy validations
    let frac = config.strategy.hitting_budget_fraction;
    if !(0.0..=1.0).contains(&frac) {
        return Err(ConfigError::ValidationError {
            field: "strategy.hitting_budget_fraction".into(),
            message: format!("must be between 0.0 and 1.0 inclusive, got {frac}"),
        });
    }

    // Category weights must all be positive
    for (name, val) in config.strategy.weights.iter() {
        if val <= 0.0 {
            return Err(ConfigError::ValidationError {
                field: format!("weights.{name}"),
                message: format!("must be > 0, got {val}"),
            });
        }
    }

    // Pool sizes must be positive
    let pool = &config.strategy.pool;
    let pool_fields: &[(&str, usize)] = &[
        ("pool.min_pa", pool.min_pa),
        ("pool.min_g_rp", pool.min_g_rp),
        ("pool.hitter_pool_size", pool.hitter_pool_size),
        ("pool.sp_pool_size", pool.sp_pool_size),
        ("pool.rp_pool_size", pool.rp_pool_size),
    ];
    for (name, val) in pool_fields {
        if *val == 0 {
            return Err(ConfigError::ValidationError {
                field: name.to_string(),
                message: "must be > 0".into(),
            });
        }
    }

    if pool.min_ip_sp <= 0.0 {
        return Err(ConfigError::ValidationError {
            field: "pool.min_ip_sp".into(),
            message: format!("must be > 0, got {}", pool.min_ip_sp),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: serialize and write default league.toml into the config dir.
    fn write_default_league_toml(config_dir: &Path) {
        let league_file = LeagueFile {
            league: LeagueConfig::default(),
        };
        let text = toml::to_string_pretty(&league_file).unwrap();
        fs::write(config_dir.join("league.toml"), text).unwrap();
    }

    /// Helper: serialize and write default strategy.toml into the config dir.
    fn write_default_strategy_toml(config_dir: &Path) {
        let strategy_file = StrategyFile::default();
        let text = toml::to_string_pretty(&strategy_file).unwrap();
        fs::write(config_dir.join("strategy.toml"), text).unwrap();
    }

    #[test]
    fn load_valid_config_from_defaults() {
        let tmp = std::env::temp_dir().join("config_test_load_defaults");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        write_default_league_toml(&config_dir);
        write_default_strategy_toml(&config_dir);

        let config = load_config_from(&tmp).expect("should load valid config");

        // League assertions
        assert_eq!(config.league.name, "Wyndham Lewis Vorticist Baseball");
        assert_eq!(config.league.platform, "espn");
        assert_eq!(config.league.num_teams, 10);
        assert_eq!(config.league.scoring_type, "h2h_most_categories");
        assert_eq!(config.league.salary_cap, 260);
        assert_eq!(
            config.league.batting_categories.categories,
            vec!["R", "HR", "RBI", "BB", "SB", "AVG"]
        );
        assert_eq!(
            config.league.pitching_categories.categories,
            vec!["K", "W", "SV", "HD", "ERA", "WHIP"]
        );
        assert_eq!(config.league.roster_limits.max_rp, 7);
        assert_eq!(config.league.roster_limits.gs_per_week, 7);
        // Teams are now optional (populated from ESPN live data)
        assert!(config.league.teams.is_empty());

        // Strategy assertions
        assert!((config.strategy.hitting_budget_fraction - 0.65).abs() < f64::EPSILON);
        assert!((config.strategy.weights.get("SV").unwrap() - 0.7).abs() < f64::EPSILON);
        assert!((config.strategy.weights.get("R").unwrap() - 1.0).abs() < f64::EPSILON);
        assert_eq!(config.strategy.pool.hitter_pool_size, 150);
        assert_eq!(config.strategy.pool.sp_pool_size, 70);
        assert_eq!(config.strategy.pool.rp_pool_size, 80);
        assert_eq!(config.strategy.llm.model, "claude-sonnet-4-6");
        assert_eq!(config.strategy.llm.provider, LlmProvider::Anthropic);
        assert_eq!(config.strategy.llm.analysis_max_tokens, 2048);
        assert_eq!(config.strategy.llm.planning_max_tokens, 2048);
        assert_eq!(config.strategy.llm.analysis_trigger, "nomination");
        assert!(config.strategy.llm.prefire_planning);

        // Infrastructure assertions
        assert_eq!(config.ws_port, 9001);
        assert!(config.data_paths.hitters.is_none());
        assert!(config.data_paths.pitchers.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn missing_credentials_toml_is_ok() {
        // Create a temporary directory with league.toml and strategy.toml but no credentials.toml
        let tmp = std::env::temp_dir().join("config_test_no_creds");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        write_default_league_toml(&config_dir);
        write_default_strategy_toml(&config_dir);

        let config = load_config_from(&tmp).expect("should load without credentials.toml");
        assert!(config.credentials.anthropic_api_key.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn credentials_toml_with_api_key() {
        let tmp = std::env::temp_dir().join("config_test_with_creds");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        write_default_league_toml(&config_dir);
        write_default_strategy_toml(&config_dir);
        fs::write(
            config_dir.join("credentials.toml"),
            "anthropic_api_key = \"sk-ant-test-key\"\n",
        )
        .unwrap();

        let config = load_config_from(&tmp).expect("should load with credentials.toml");
        assert_eq!(
            config.credentials.anthropic_api_key.as_deref(),
            Some("sk-ant-test-key")
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_num_teams_zero() {
        let tmp = std::env::temp_dir().join("config_test_num_teams_zero");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        // Write a league.toml with num_teams = 0
        let league_toml = r#"
[league]
name = "Test"
platform = "espn"
num_teams = 0
scoring_type = "h2h"
salary_cap = 260

[league.batting_categories]
categories = ["R"]

[league.pitching_categories]
categories = ["K"]

[league.roster]
SP = 5

[league.roster_limits]
max_sp = 7
max_rp = 7
gs_per_week = 7
"#;
        fs::write(config_dir.join("league.toml"), league_toml).unwrap();
        write_default_strategy_toml(&config_dir);

        let err = load_config_from(&tmp).unwrap_err();
        match &err {
            ConfigError::ValidationError { field, .. } => {
                assert_eq!(field, "league.num_teams");
            }
            other => panic!("expected ValidationError, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_salary_cap_zero() {
        let tmp = std::env::temp_dir().join("config_test_salary_cap_zero");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        let league_toml = r#"
[league]
name = "Test"
platform = "espn"
num_teams = 10
scoring_type = "h2h"
salary_cap = 0

[league.batting_categories]
categories = ["R"]

[league.pitching_categories]
categories = ["K"]

[league.roster]
SP = 5

[league.roster_limits]
max_sp = 7
max_rp = 7
gs_per_week = 7
"#;
        fs::write(config_dir.join("league.toml"), league_toml).unwrap();
        write_default_strategy_toml(&config_dir);

        let err = load_config_from(&tmp).unwrap_err();
        match &err {
            ConfigError::ValidationError { field, .. } => {
                assert_eq!(field, "league.salary_cap");
            }
            other => panic!("expected ValidationError, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_hitting_budget_fraction_too_high() {
        let tmp = std::env::temp_dir().join("config_test_budget_high");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        write_default_league_toml(&config_dir);

        // Write strategy.toml with hitting_budget_fraction = 1.5
        let strategy_text = toml::to_string_pretty(&StrategyFile::default()).unwrap();
        let modified = strategy_text.replace(
            "hitting_budget_fraction = 0.65",
            "hitting_budget_fraction = 1.5",
        );
        fs::write(config_dir.join("strategy.toml"), modified).unwrap();

        let err = load_config_from(&tmp).unwrap_err();
        match &err {
            ConfigError::ValidationError { field, .. } => {
                assert_eq!(field, "strategy.hitting_budget_fraction");
            }
            other => panic!("expected ValidationError, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_hitting_budget_fraction_negative() {
        let tmp = std::env::temp_dir().join("config_test_budget_neg");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        write_default_league_toml(&config_dir);

        let strategy_text = toml::to_string_pretty(&StrategyFile::default()).unwrap();
        let modified = strategy_text.replace(
            "hitting_budget_fraction = 0.65",
            "hitting_budget_fraction = -0.1",
        );
        fs::write(config_dir.join("strategy.toml"), modified).unwrap();

        let err = load_config_from(&tmp).unwrap_err();
        match &err {
            ConfigError::ValidationError { field, .. } => {
                assert_eq!(field, "strategy.hitting_budget_fraction");
            }
            other => panic!("expected ValidationError, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_zero_weight() {
        let tmp = std::env::temp_dir().join("config_test_zero_weight");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        write_default_league_toml(&config_dir);

        let strategy_text = toml::to_string_pretty(&StrategyFile::default()).unwrap();
        // Set SV weight to 0.0 (should fail validation: weights must be > 0)
        let modified = strategy_text.replace("SV = 0.7", "SV = 0.0");
        fs::write(config_dir.join("strategy.toml"), modified).unwrap();

        let err = load_config_from(&tmp).unwrap_err();
        match &err {
            ConfigError::ValidationError { field, .. } => {
                assert_eq!(field, "weights.SV");
            }
            other => panic!("expected ValidationError, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_zero_pool_size() {
        let tmp = std::env::temp_dir().join("config_test_zero_pool");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        write_default_league_toml(&config_dir);

        let strategy_text = toml::to_string_pretty(&StrategyFile::default()).unwrap();
        let modified = strategy_text.replace("hitter_pool_size = 150", "hitter_pool_size = 0");
        fs::write(config_dir.join("strategy.toml"), modified).unwrap();

        let err = load_config_from(&tmp).unwrap_err();
        match &err {
            ConfigError::ValidationError { field, .. } => {
                assert_eq!(field, "pool.hitter_pool_size");
            }
            other => panic!("expected ValidationError, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn file_not_found_for_missing_league_toml() {
        let tmp = std::env::temp_dir().join("config_test_missing_league");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        // No league.toml written — only strategy.toml
        write_default_strategy_toml(&config_dir);

        let err = load_config_from(&tmp).unwrap_err();
        match &err {
            ConfigError::FileNotFound { path } => {
                assert!(path.ends_with("league.toml"));
            }
            other => panic!("expected FileNotFound, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn file_not_found_for_missing_strategy_toml() {
        let tmp = std::env::temp_dir().join("config_test_missing_strategy");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        write_default_league_toml(&config_dir);
        // No strategy.toml written

        let err = load_config_from(&tmp).unwrap_err();
        match &err {
            ConfigError::FileNotFound { path } => {
                assert!(path.ends_with("strategy.toml"));
            }
            other => panic!("expected FileNotFound, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_error_for_invalid_toml() {
        let tmp = std::env::temp_dir().join("config_test_invalid_toml");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        fs::write(config_dir.join("league.toml"), "this is not valid [[[ toml").unwrap();
        write_default_strategy_toml(&config_dir);

        let err = load_config_from(&tmp).unwrap_err();
        match &err {
            ConfigError::ParseError { path, .. } => {
                assert!(path.ends_with("league.toml"));
            }
            other => panic!("expected ParseError, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_default_config_files_creates_missing_files() {
        let tmp = std::env::temp_dir().join("config_test_ensure_creates");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // No config/ dir exists yet
        assert!(!tmp.join("config").exists());

        let created = ensure_default_config_files(&tmp).expect("should succeed");
        assert_eq!(created.len(), 2);

        // config/ should now exist with both files
        assert!(tmp.join("config/league.toml").exists());
        assert!(tmp.join("config/strategy.toml").exists());
        // credentials.toml should NOT be created (it's optional)
        assert!(!tmp.join("config/credentials.toml").exists());

        // The generated files should be loadable
        let config = load_config_from(&tmp).expect("should load generated config");
        assert_eq!(config.league.num_teams, 10);
        assert_eq!(config.ws_port, 9001);

        // The generated strategy.toml should NOT contain [data_paths] section
        // since both paths default to None and the section is skipped when empty
        let strategy_content = fs::read_to_string(tmp.join("config/strategy.toml")).unwrap();
        assert!(
            !strategy_content.contains("[data_paths]"),
            "default strategy.toml should not contain [data_paths] section"
        );
        assert!(config.data_paths.hitters.is_none());
        assert!(config.data_paths.pitchers.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn strategy_toml_with_data_paths_overrides() {
        let tmp = std::env::temp_dir().join("config_test_data_paths_override");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        write_default_league_toml(&config_dir);

        // Write a strategy.toml with data_paths set
        let mut strategy_text = toml::to_string_pretty(&StrategyFile::default()).unwrap();
        strategy_text.push_str("\n[data_paths]\nhitters = \"custom/hitters.csv\"\npitchers = \"custom/pitchers.csv\"\n");
        fs::write(config_dir.join("strategy.toml"), strategy_text).unwrap();

        let config = load_config_from(&tmp).expect("should load config with data_paths");
        assert_eq!(config.data_paths.hitters.as_deref(), Some("custom/hitters.csv"));
        assert_eq!(config.data_paths.pitchers.as_deref(), Some("custom/pitchers.csv"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_default_config_files_skips_existing() {
        let tmp = std::env::temp_dir().join("config_test_ensure_skips");
        let _ = fs::remove_dir_all(&tmp);

        let config_dir = tmp.join("config");
        fs::create_dir_all(&config_dir).unwrap();

        // Pre-create league.toml in config/ with custom content
        fs::write(config_dir.join("league.toml"), "# custom\n").unwrap();

        let created = ensure_default_config_files(&tmp).expect("should succeed");
        // Only strategy.toml should be created (league.toml already exists)
        assert_eq!(created.len(), 1);
        assert!(created[0].ends_with("strategy.toml"));

        // Original custom content should be preserved
        let content = fs::read_to_string(config_dir.join("league.toml")).unwrap();
        assert_eq!(content, "# custom\n");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn default_config_matches_expected_values() {
        let config = Config::default();

        // Verify the in-code defaults match the values that were in the old TOML files
        assert_eq!(config.league.name, "Wyndham Lewis Vorticist Baseball");
        assert_eq!(config.league.platform, "espn");
        assert_eq!(config.league.num_teams, 10);
        assert_eq!(config.league.scoring_type, "h2h_most_categories");
        assert_eq!(config.league.salary_cap, 260);
        assert_eq!(
            config.league.batting_categories.categories,
            vec!["R", "HR", "RBI", "BB", "SB", "AVG"]
        );
        assert_eq!(
            config.league.pitching_categories.categories,
            vec!["K", "W", "SV", "HD", "ERA", "WHIP"]
        );
        assert_eq!(config.league.roster_limits.max_sp, 7);
        assert_eq!(config.league.roster_limits.max_rp, 7);
        assert_eq!(config.league.roster_limits.gs_per_week, 7);
        assert!(config.league.teams.is_empty());

        assert!((config.strategy.hitting_budget_fraction - 0.65).abs() < f64::EPSILON);
        assert!((config.strategy.weights.get("R").unwrap() - 1.0).abs() < f64::EPSILON);
        assert!((config.strategy.weights.get("SV").unwrap() - 0.7).abs() < f64::EPSILON);
        assert_eq!(config.strategy.pool.min_pa, 200);
        assert!((config.strategy.pool.min_ip_sp - 50.0).abs() < f64::EPSILON);
        assert_eq!(config.strategy.pool.min_g_rp, 20);
        assert_eq!(config.strategy.pool.hitter_pool_size, 150);
        assert_eq!(config.strategy.pool.sp_pool_size, 70);
        assert_eq!(config.strategy.pool.rp_pool_size, 80);
        assert_eq!(config.strategy.llm.provider, LlmProvider::Anthropic);
        assert_eq!(config.strategy.llm.model, "claude-sonnet-4-6");
        assert_eq!(config.strategy.llm.analysis_max_tokens, 2048);
        assert_eq!(config.strategy.llm.planning_max_tokens, 2048);
        assert_eq!(config.strategy.llm.analysis_trigger, "nomination");
        assert!(config.strategy.llm.prefire_planning);

        assert_eq!(config.ws_port, 9001);
        assert!(config.data_paths.hitters.is_none());
        assert!(config.data_paths.pitchers.is_none());

        assert!(config.credentials.anthropic_api_key.is_none());
        assert!(config.credentials.google_api_key.is_none());
        assert!(config.credentials.openai_api_key.is_none());
    }
}
