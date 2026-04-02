// Shared test helpers and fixtures used across unit test modules.
//
// Provides common configuration builders, assertion helpers, and convenience
// constructors so that individual test modules don't duplicate boilerplate.

use std::collections::HashMap;

use crate::config::*;
use crate::draft::state::{DraftState, TeamBudgetPayload};
use crate::stats::{CategoryValues, StatRegistry};
use crate::valuation::zscore::PlayerValuation;

// ---------------------------------------------------------------------------
// Configuration fixtures
// ---------------------------------------------------------------------------

/// Standard 10-team league config with the default 12 categories.
pub fn test_league_config() -> LeagueConfig {
    LeagueConfig {
        name: "Test League".into(),
        platform: "espn".into(),
        num_teams: 10,
        scoring_type: "h2h_most_categories".into(),
        salary_cap: 260,
        batting_categories: CategoriesSection {
            categories: vec![
                "R".into(),
                "HR".into(),
                "RBI".into(),
                "BB".into(),
                "SB".into(),
                "AVG".into(),
            ],
        },
        pitching_categories: CategoriesSection {
            categories: vec![
                "K".into(),
                "W".into(),
                "SV".into(),
                "HD".into(),
                "ERA".into(),
                "WHIP".into(),
            ],
        },
        roster_limits: RosterLimits {
            max_sp: 7,
            max_rp: 7,
            gs_per_week: 7,
        },
        teams: HashMap::new(),
    }
}

/// Standard roster slot configuration (26 roster slots).
pub fn test_roster_config() -> HashMap<String, usize> {
    let mut config = HashMap::new();
    config.insert("C".into(), 1);
    config.insert("1B".into(), 1);
    config.insert("2B".into(), 1);
    config.insert("3B".into(), 1);
    config.insert("SS".into(), 1);
    config.insert("LF".into(), 1);
    config.insert("CF".into(), 1);
    config.insert("RF".into(), 1);
    config.insert("UTIL".into(), 1);
    config.insert("SP".into(), 5);
    config.insert("RP".into(), 6);
    config.insert("BE".into(), 6);
    config.insert("IL".into(), 5);
    config
}

/// Standard strategy config with league-appropriate weights.
pub fn test_strategy_config() -> StrategyConfig {
    StrategyConfig {
        hitting_budget_fraction: 0.65,
        weights: CategoryWeights::from_pairs([
            ("R", 1.0),
            ("HR", 1.0),
            ("RBI", 1.0),
            ("BB", 1.2),
            ("SB", 1.0),
            ("AVG", 1.0),
            ("K", 1.0),
            ("W", 1.0),
            ("SV", 0.7),
            ("HD", 1.3),
            ("ERA", 1.0),
            ("WHIP", 1.0),
        ]),
        strategy_overview: None,
        pool: PoolConfig {
            min_pa: 300,
            min_ip_sp: 80.0,
            min_g_rp: 30,
            hitter_pool_size: 150,
            sp_pool_size: 70,
            rp_pool_size: 80,
        },
        llm: LlmConfig {
            provider: crate::llm::provider::LlmProvider::Anthropic,
            model: "test".into(),
            analysis_max_tokens: 2048,
            planning_max_tokens: 2048,
            analysis_trigger: "nomination".into(),
            prefire_planning: true,
        },
    }
}

/// Standard full Config combining league + strategy defaults.
pub fn test_config() -> Config {
    Config {
        league: test_league_config(),
        strategy: test_strategy_config(),
        credentials: CredentialsConfig::default(),
        ws_port: 9001,
        data_paths: DataPaths::default(),
    }
}

// ---------------------------------------------------------------------------
// Registry and draft state fixtures
// ---------------------------------------------------------------------------

/// Build a StatRegistry from the standard 12-category league config.
pub fn test_registry() -> StatRegistry {
    StatRegistry::from_league_config(&test_league_config()).expect("test registry")
}

/// Generate ESPN budget payloads for `num_teams` teams, each with $260.
pub fn test_espn_budgets(num_teams: usize) -> Vec<TeamBudgetPayload> {
    (1..=num_teams)
        .map(|i| TeamBudgetPayload {
            team_id: format!("{}", i),
            team_name: format!("Team {}", i),
            budget: 260,
        })
        .collect()
}

/// Create a DraftState with `num_teams` teams registered and team "1" as mine.
pub fn create_test_draft_state(num_teams: usize) -> DraftState {
    let mut state = DraftState::new(260, &test_roster_config());
    state.reconcile_budgets(&test_espn_budgets(num_teams));
    state.set_my_team_by_id("1");
    state
}

// ---------------------------------------------------------------------------
// CategoryValues helpers
// ---------------------------------------------------------------------------

/// Create a `CategoryValues` from abbreviation-value pairs.
///
/// More readable than `CategoryValues::from_vec(vec![...])` with positional
/// indices. Unspecified categories default to 0.0.
///
/// ```ignore
/// let needs = test_category_values(&registry, &[("R", 0.8), ("HR", 0.5), ("BB", 1.0)]);
/// ```
pub fn test_category_values(registry: &StatRegistry, pairs: &[(&str, f64)]) -> CategoryValues {
    let mut cv = CategoryValues::zeros(registry.len());
    for &(abbrev, value) in pairs {
        if let Some(idx) = registry.index_of(abbrev) {
            cv.set(idx, value);
        }
    }
    cv
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

/// Check approximate floating-point equality within `epsilon`.
pub fn approx_eq(a: f64, b: f64, epsilon: f64) -> bool {
    (a - b).abs() < epsilon
}

/// Assert that `actual` is within 1e-10 of `expected`, with a descriptive label.
pub fn assert_close(actual: f64, expected: f64, label: &str) {
    assert!(
        (actual - expected).abs() < 1e-10,
        "{}: expected {:.15}, got {:.15}, diff={:.2e}",
        label,
        expected,
        actual,
        (actual - expected).abs(),
    );
}

/// Find a player by name in a slice, panicking if not found.
pub fn find_player<'a>(players: &'a [PlayerValuation], name: &str) -> &'a PlayerValuation {
    players.iter().find(|p| p.name == name).unwrap()
}
