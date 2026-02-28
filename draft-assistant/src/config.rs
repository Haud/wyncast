// Configuration loading and parsing (league.toml, strategy.toml, credentials.toml).

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

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

    #[error("failed to initialize config from defaults: {message}")]
    DefaultsCopyError { message: String },
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
    pub db_path: String,
    pub data_paths: DataPaths,
}

// ---------------------------------------------------------------------------
// league.toml structs
// ---------------------------------------------------------------------------

/// Wrapper for the top-level `[league]` table in league.toml.
#[derive(Debug, Clone, Deserialize)]
struct LeagueFile {
    league: LeagueConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LeagueConfig {
    pub name: String,
    pub platform: String,
    pub num_teams: usize,
    pub scoring_type: String,
    pub salary_cap: u32,
    pub batting_categories: CategoriesSection,
    pub pitching_categories: CategoriesSection,
    pub roster: HashMap<String, usize>,
    pub roster_limits: RosterLimits,
    /// Static team definitions (optional). Teams are now populated dynamically
    /// from ESPN's live draft data via the extension.
    #[serde(default)]
    pub teams: HashMap<String, String>,
    /// The user's team identifier (optional). When omitted, the user's team
    /// is identified dynamically from the ESPN extension's `myTeamId` field.
    #[serde(default)]
    pub my_team: Option<MyTeam>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CategoriesSection {
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RosterLimits {
    pub max_sp: usize,
    pub max_rp: usize,
    pub gs_per_week: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MyTeam {
    pub team_id: String,
}

// ---------------------------------------------------------------------------
// strategy.toml structs
// ---------------------------------------------------------------------------

/// Raw deserialization target for the entire strategy.toml file.
#[derive(Debug, Clone, Deserialize)]
struct StrategyFile {
    budget: BudgetSection,
    category_weights: CategoryWeights,
    pool: PoolConfig,
    llm: LlmConfig,
    websocket: WebsocketSection,
    database: DatabaseSection,
    data_paths: DataPaths,
}

#[derive(Debug, Clone, Deserialize)]
struct BudgetSection {
    hitting_budget_fraction: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct WebsocketSection {
    port: u16,
}

#[derive(Debug, Clone, Deserialize)]
struct DatabaseSection {
    path: String,
}

/// The public strategy config assembled from the strategy.toml sections.
#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub hitting_budget_fraction: f64,
    pub weights: CategoryWeights,
    pub pool: PoolConfig,
    pub llm: LlmConfig,
}

/// Category weight multipliers. The field names use UPPERCASE to match the
/// TOML keys (R, HR, ...). Serde aliases with `#[serde(rename)]` map them.
#[derive(Debug, Clone, Deserialize)]
#[allow(non_snake_case)]
pub struct CategoryWeights {
    pub R: f64,
    pub HR: f64,
    pub RBI: f64,
    pub BB: f64,
    pub SB: f64,
    pub AVG: f64,
    pub K: f64,
    pub W: f64,
    pub SV: f64,
    pub HD: f64,
    pub ERA: f64,
    pub WHIP: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PoolConfig {
    pub min_pa: usize,
    pub min_ip_sp: f64,
    pub min_g_rp: usize,
    pub hitter_pool_size: usize,
    pub sp_pool_size: usize,
    pub rp_pool_size: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub model: String,
    pub analysis_max_tokens: u32,
    pub planning_max_tokens: u32,
    pub analysis_trigger: String,
    pub prefire_planning: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DataPaths {
    pub hitters: String,
    pub pitchers: String,
}

// ---------------------------------------------------------------------------
// credentials.toml structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CredentialsConfig {
    pub anthropic_api_key: Option<String>,
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
    };

    let ws_port = strategy_file.websocket.port;
    let db_path = strategy_file.database.path;
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
        db_path,
        data_paths,
    };

    validate(&config)?;

    Ok(config)
}

/// Ensure all config files exist by copying missing ones from `defaults/`.
/// Returns the list of files that were copied. Skips `.example` files.
pub fn ensure_config_files(base_dir: &Path) -> Result<Vec<PathBuf>, ConfigError> {
    let defaults_dir = base_dir.join("defaults");
    let config_dir = base_dir.join("config");

    if !defaults_dir.exists() {
        // If config/ also doesn't exist, the app will fail to load config.
        // Return an error with a clear message about the missing defaults directory.
        if !config_dir.exists() {
            return Err(ConfigError::DefaultsCopyError {
                message: format!(
                    "neither defaults/ nor config/ directory found in {}; \
                     run from the project root or ensure defaults/ is present",
                    base_dir.display()
                ),
            });
        }
        return Ok(vec![]);
    }

    std::fs::create_dir_all(&config_dir).map_err(|e| ConfigError::DefaultsCopyError {
        message: format!("failed to create config directory: {e}"),
    })?;

    let mut copied = Vec::new();

    let entries = std::fs::read_dir(&defaults_dir).map_err(|e| ConfigError::DefaultsCopyError {
        message: format!("failed to read defaults directory: {e}"),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| ConfigError::DefaultsCopyError {
            message: format!("failed to read defaults entry: {e}"),
        })?;
        let path = entry.path();

        // Skip non-files and entries without a file name
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };

        // Skip .example template files
        if file_name.to_str().is_some_and(|n| n.ends_with(".example")) {
            continue;
        }
        let target = config_dir.join(file_name);

        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(mut dest) => {
                let content = std::fs::read(&path).map_err(|e| ConfigError::DefaultsCopyError {
                    message: format!("failed to read {}: {e}", path.display()),
                })?;
                std::io::Write::write_all(&mut dest, &content).map_err(|e| {
                    ConfigError::DefaultsCopyError {
                        message: format!("failed to write {}: {e}", target.display()),
                    }
                })?;
                copied.push(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // File already exists in config/, skip it
            }
            Err(e) => {
                return Err(ConfigError::DefaultsCopyError {
                    message: format!("failed to create {}: {e}", target.display()),
                });
            }
        }
    }

    Ok(copied)
}

/// Convenience wrapper: loads config relative to the current working directory.
/// Ensures default config files are copied before loading.
pub fn load_config() -> Result<Config, ConfigError> {
    let cwd = std::env::current_dir().map_err(|_| ConfigError::FileNotFound {
        path: PathBuf::from("."),
    })?;
    ensure_config_files(&cwd)?;
    load_config_from(&cwd)
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
    let w = &config.strategy.weights;
    let weight_fields: &[(&str, f64)] = &[
        ("weights.R", w.R),
        ("weights.HR", w.HR),
        ("weights.RBI", w.RBI),
        ("weights.BB", w.BB),
        ("weights.SB", w.SB),
        ("weights.AVG", w.AVG),
        ("weights.K", w.K),
        ("weights.W", w.W),
        ("weights.SV", w.SV),
        ("weights.HD", w.HD),
        ("weights.ERA", w.ERA),
        ("weights.WHIP", w.WHIP),
    ];
    for (name, val) in weight_fields {
        if *val <= 0.0 {
            return Err(ConfigError::ValidationError {
                field: name.to_string(),
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
    use std::path::PathBuf;

    /// Helper: returns the path to the draft-assistant project root
    /// (works whether `cargo test` runs from the crate root or repo root).
    fn project_root() -> PathBuf {
        let cwd = std::env::current_dir().unwrap();
        if cwd.join("defaults").exists() {
            cwd
        } else if cwd.join("draft-assistant/defaults").exists() {
            cwd.join("draft-assistant")
        } else {
            panic!("Cannot locate defaults/ directory from CWD {:?}", cwd);
        }
    }

    #[test]
    fn load_valid_config_from_project_files() {
        let root = project_root();
        ensure_config_files(&root).expect("should copy default configs");
        let config = load_config_from(&root).expect("should load valid config");

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
        assert_eq!(config.league.roster.get("SP"), Some(&5));
        assert_eq!(config.league.roster.get("RP"), Some(&6));
        assert_eq!(config.league.roster_limits.max_rp, 7);
        assert_eq!(config.league.roster_limits.gs_per_week, 7);
        // Teams and my_team are now optional (populated from ESPN live data)
        assert!(config.league.teams.is_empty());
        assert!(config.league.my_team.is_none());

        // Strategy assertions
        assert!((config.strategy.hitting_budget_fraction - 0.65).abs() < f64::EPSILON);
        assert!((config.strategy.weights.SV - 0.7).abs() < f64::EPSILON);
        assert!((config.strategy.weights.R - 1.0).abs() < f64::EPSILON);
        assert_eq!(config.strategy.pool.hitter_pool_size, 150);
        assert_eq!(config.strategy.pool.sp_pool_size, 70);
        assert_eq!(config.strategy.pool.rp_pool_size, 80);
        assert_eq!(config.strategy.llm.model, "claude-sonnet-4-5-20250929");
        assert_eq!(config.strategy.llm.analysis_max_tokens, 400);
        assert_eq!(config.strategy.llm.planning_max_tokens, 600);
        assert_eq!(config.strategy.llm.analysis_trigger, "nomination");
        assert!(config.strategy.llm.prefire_planning);

        // Infrastructure assertions
        assert_eq!(config.ws_port, 9001);
        assert_eq!(config.db_path, "draft-assistant.db");
        assert_eq!(config.data_paths.hitters, "data/projections/hitters.csv");
    }

    #[test]
    fn missing_credentials_toml_is_ok() {
        // Create a temporary directory with league.toml and strategy.toml but no credentials.toml
        let tmp = std::env::temp_dir().join("config_test_no_creds");
        let config_dir = tmp.join("config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&config_dir).unwrap();

        let root = project_root();
        fs::copy(root.join("defaults/league.toml"), config_dir.join("league.toml")).unwrap();
        fs::copy(
            root.join("defaults/strategy.toml"),
            config_dir.join("strategy.toml"),
        )
        .unwrap();

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

        let root = project_root();
        fs::copy(root.join("defaults/league.toml"), config_dir.join("league.toml")).unwrap();
        fs::copy(
            root.join("defaults/strategy.toml"),
            config_dir.join("strategy.toml"),
        )
        .unwrap();
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

        let root = project_root();
        fs::copy(
            root.join("defaults/strategy.toml"),
            config_dir.join("strategy.toml"),
        )
        .unwrap();

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

        let root = project_root();
        fs::copy(
            root.join("defaults/strategy.toml"),
            config_dir.join("strategy.toml"),
        )
        .unwrap();

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

        let root = project_root();
        fs::copy(root.join("defaults/league.toml"), config_dir.join("league.toml")).unwrap();

        // Write strategy.toml with hitting_budget_fraction = 1.5
        let strategy_text = fs::read_to_string(root.join("defaults/strategy.toml")).unwrap();
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

        let root = project_root();
        fs::copy(root.join("defaults/league.toml"), config_dir.join("league.toml")).unwrap();

        let strategy_text = fs::read_to_string(root.join("defaults/strategy.toml")).unwrap();
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

        let root = project_root();
        fs::copy(root.join("defaults/league.toml"), config_dir.join("league.toml")).unwrap();

        let strategy_text = fs::read_to_string(root.join("defaults/strategy.toml")).unwrap();
        // Set SV weight to 0.0 (should fail validation: weights must be > 0)
        let modified = strategy_text.replace("SV   = 0.7", "SV   = 0.0");
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

        let root = project_root();
        fs::copy(root.join("defaults/league.toml"), config_dir.join("league.toml")).unwrap();

        let strategy_text = fs::read_to_string(root.join("defaults/strategy.toml")).unwrap();
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

        // No league.toml written
        let root = project_root();
        fs::copy(
            root.join("defaults/strategy.toml"),
            config_dir.join("strategy.toml"),
        )
        .unwrap();

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

        let root = project_root();
        fs::copy(root.join("defaults/league.toml"), config_dir.join("league.toml")).unwrap();
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

        let root = project_root();
        fs::copy(
            root.join("defaults/strategy.toml"),
            config_dir.join("strategy.toml"),
        )
        .unwrap();

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
    fn ensure_config_files_copies_missing_files() {
        let tmp = std::env::temp_dir().join("config_test_ensure_copies");
        let _ = fs::remove_dir_all(&tmp);

        // Create defaults/ with league.toml and strategy.toml
        let defaults_dir = tmp.join("defaults");
        fs::create_dir_all(&defaults_dir).unwrap();

        let root = project_root();
        fs::copy(root.join("defaults/league.toml"), defaults_dir.join("league.toml")).unwrap();
        fs::copy(root.join("defaults/strategy.toml"), defaults_dir.join("strategy.toml")).unwrap();
        // Add an example file that should NOT be copied
        fs::write(
            defaults_dir.join("credentials.toml.example"),
            "anthropic_api_key = \"sk-ant-...\"\n",
        )
        .unwrap();

        // No config/ dir exists yet
        assert!(!tmp.join("config").exists());

        let copied = ensure_config_files(&tmp).expect("should succeed");
        assert_eq!(copied.len(), 2);

        // config/ should now exist with both files
        assert!(tmp.join("config/league.toml").exists());
        assert!(tmp.join("config/strategy.toml").exists());
        // example file should NOT have been copied
        assert!(!tmp.join("config/credentials.toml.example").exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_config_files_skips_existing() {
        let tmp = std::env::temp_dir().join("config_test_ensure_skips");
        let _ = fs::remove_dir_all(&tmp);

        let defaults_dir = tmp.join("defaults");
        let config_dir = tmp.join("config");
        fs::create_dir_all(&defaults_dir).unwrap();
        fs::create_dir_all(&config_dir).unwrap();

        let root = project_root();
        fs::copy(root.join("defaults/league.toml"), defaults_dir.join("league.toml")).unwrap();
        fs::copy(root.join("defaults/strategy.toml"), defaults_dir.join("strategy.toml")).unwrap();

        // Pre-create league.toml in config/ with custom content
        fs::write(config_dir.join("league.toml"), "# custom\n").unwrap();

        let copied = ensure_config_files(&tmp).expect("should succeed");
        // Only strategy.toml should be copied (league.toml already exists)
        assert_eq!(copied.len(), 1);
        assert!(copied[0].ends_with("strategy.toml"));

        // Original custom content should be preserved
        let content = fs::read_to_string(config_dir.join("league.toml")).unwrap();
        assert_eq!(content, "# custom\n");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_config_files_no_defaults_dir_is_ok() {
        let tmp = std::env::temp_dir().join("config_test_no_defaults");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Create config/ so it's not an error (just no defaults to copy)
        fs::create_dir_all(tmp.join("config")).unwrap();

        // No defaults/ directory, but config/ exists - should succeed
        let copied = ensure_config_files(&tmp).expect("should succeed");
        assert!(copied.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_config_files_errors_when_both_dirs_missing() {
        let tmp = std::env::temp_dir().join("config_test_both_missing");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Neither defaults/ nor config/ exist
        let err = ensure_config_files(&tmp).unwrap_err();
        match &err {
            ConfigError::DefaultsCopyError { message } => {
                assert!(message.contains("neither defaults/ nor config/"));
            }
            other => panic!("expected DefaultsCopyError, got: {other}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }
}
