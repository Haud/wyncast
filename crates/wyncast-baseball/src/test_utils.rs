// Shared test helpers and fixtures used across unit test modules.
//
// Provides common configuration builders, assertion helpers, and convenience
// constructors so that individual test modules don't duplicate boilerplate.

use std::collections::HashMap;

use wyncast_core::config::*;
use wyncast_core::stats::{CategoryValues, StatRegistry};

use crate::draft::pick::Position;
use crate::draft::state::{DraftState, TeamBudgetPayload};
use crate::valuation::projections::PitcherType;
use crate::valuation::zscore::{CategoryZScores, PlayerValuation, ProjectionData};

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
            provider: wyncast_core::llm::provider::LlmProvider::Anthropic,
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

// ---------------------------------------------------------------------------
// PlayerValuation builders
// ---------------------------------------------------------------------------

/// Builder for `PlayerValuation` test fixtures.
pub struct TestPlayer {
    name: String,
    pitcher_type: Option<PitcherType>,
    positions: Vec<Position>,
    vor: f64,
    total_zscore: Option<f64>,
    dollar_value: f64,
    zscore_pairs: Vec<(String, f64)>,
}

impl TestPlayer {
    /// Start building a hitter fixture. Default position: `FirstBase`.
    pub fn hitter(name: &str) -> Self {
        TestPlayer {
            name: name.into(),
            pitcher_type: None,
            positions: vec![Position::FirstBase],
            vor: 0.0,
            total_zscore: None,
            dollar_value: 0.0,
            zscore_pairs: vec![],
        }
    }

    /// Start building a pitcher fixture. Default position derived from `pt`.
    pub fn pitcher(name: &str, pt: PitcherType) -> Self {
        let pos = match pt {
            PitcherType::SP => Position::StartingPitcher,
            PitcherType::RP => Position::ReliefPitcher,
        };
        TestPlayer {
            name: name.into(),
            pitcher_type: Some(pt),
            positions: vec![pos],
            vor: 0.0,
            total_zscore: None,
            dollar_value: 0.0,
            zscore_pairs: vec![],
        }
    }

    /// Set the VOR value.
    pub fn vor(mut self, v: f64) -> Self {
        self.vor = v;
        self
    }

    /// Override `total_zscore` directly.
    pub fn total_zscore(mut self, z: f64) -> Self {
        self.total_zscore = Some(z);
        self
    }

    /// Override the eligible positions list.
    pub fn positions(mut self, ps: Vec<Position>) -> Self {
        self.positions = ps;
        self
    }

    /// Set the dollar value.
    pub fn dollar(mut self, d: f64) -> Self {
        self.dollar_value = d;
        self
    }

    /// Set per-category z-score values by abbreviation.
    pub fn zscores(mut self, pairs: &[(&str, f64)]) -> Self {
        self.zscore_pairs = pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect();
        self
    }

    /// Build the `PlayerValuation`.
    pub fn build(self) -> PlayerValuation {
        let is_pitcher = self.pitcher_type.is_some();
        let registry = test_registry();
        let total = self.total_zscore.unwrap_or(if is_pitcher {
            self.vor + 1.0
        } else {
            self.vor + 2.0
        });

        let mut zv = CategoryValues::zeros(registry.len());
        for (abbrev, value) in &self.zscore_pairs {
            if let Some(idx) = registry.index_of(abbrev) {
                zv.set(idx, *value);
            }
        }

        let (projection, category_zscores) = if is_pitcher {
            let proj = ProjectionData {
                values: HashMap::from([
                    ("ip".into(), 180.0),
                    ("k".into(), 200.0),
                    ("w".into(), 14.0),
                    ("sv".into(), 0.0),
                    ("hd".into(), 0.0),
                    ("era".into(), 3.20),
                    ("whip".into(), 1.10),
                    ("g".into(), 30.0),
                    ("gs".into(), 30.0),
                ]),
            };
            (proj, CategoryZScores::pitcher(zv, total))
        } else {
            let proj = ProjectionData {
                values: HashMap::from([
                    ("pa".into(), 600.0),
                    ("ab".into(), 550.0),
                    ("h".into(), 150.0),
                    ("hr".into(), 25.0),
                    ("r".into(), 80.0),
                    ("rbi".into(), 85.0),
                    ("bb".into(), 50.0),
                    ("sb".into(), 10.0),
                    ("avg".into(), 0.273),
                ]),
            };
            (proj, CategoryZScores::hitter(zv, total))
        };

        PlayerValuation {
            name: self.name,
            team: "TST".into(),
            positions: self.positions.clone(),
            is_pitcher,
            is_two_way: false,
            pitcher_type: self.pitcher_type,
            projection,
            total_zscore: total,
            category_zscores,
            vor: self.vor,
            initial_vor: self.vor,
            best_position: self.positions.first().copied(),
            dollar_value: self.dollar_value,
        }
    }
}

// ---------------------------------------------------------------------------
// Stat-based PlayerValuation constructors
// ---------------------------------------------------------------------------

/// Build a hitter `PlayerValuation` from raw projection stats.
pub fn make_hitter(
    name: &str,
    r: u32,
    hr: u32,
    rbi: u32,
    bb: u32,
    sb: u32,
    ab: u32,
    avg: f64,
    positions: Vec<Position>,
) -> PlayerValuation {
    PlayerValuation {
        name: name.into(),
        team: "TST".into(),
        positions,
        is_pitcher: false,
        is_two_way: false,
        pitcher_type: None,
        projection: ProjectionData {
            values: HashMap::from([
                ("pa".into(), (ab + bb) as f64),
                ("ab".into(), ab as f64),
                ("h".into(), (ab as f64 * avg).round()),
                ("hr".into(), hr as f64),
                ("r".into(), r as f64),
                ("rbi".into(), rbi as f64),
                ("bb".into(), bb as f64),
                ("sb".into(), sb as f64),
                ("avg".into(), avg),
            ]),
        },
        total_zscore: 0.0,
        category_zscores: CategoryZScores::zeros_hitter(test_registry().len()),
        vor: 0.0,
        initial_vor: 0.0,
        best_position: None,
        dollar_value: 0.0,
    }
}

/// Build a pitcher `PlayerValuation` from raw projection stats.
pub fn make_pitcher(
    name: &str,
    k: u32,
    w: u32,
    sv: u32,
    hd: u32,
    ip: f64,
    era: f64,
    whip: f64,
    pitcher_type: PitcherType,
) -> PlayerValuation {
    let pos = match pitcher_type {
        PitcherType::SP => Position::StartingPitcher,
        PitcherType::RP => Position::ReliefPitcher,
    };
    PlayerValuation {
        name: name.into(),
        team: "TST".into(),
        positions: vec![pos],
        is_pitcher: true,
        is_two_way: false,
        pitcher_type: Some(pitcher_type),
        projection: ProjectionData {
            values: HashMap::from([
                ("ip".into(), ip),
                ("k".into(), k as f64),
                ("w".into(), w as f64),
                ("sv".into(), sv as f64),
                ("hd".into(), hd as f64),
                ("era".into(), era),
                ("whip".into(), whip),
                ("g".into(), 30.0),
                (
                    "gs".into(),
                    if pitcher_type == PitcherType::SP {
                        30.0
                    } else {
                        0.0
                    },
                ),
            ]),
        },
        total_zscore: 0.0,
        category_zscores: CategoryZScores::zeros_pitcher(test_registry().len()),
        vor: 0.0,
        initial_vor: 0.0,
        best_position: None,
        dollar_value: 0.0,
    }
}
