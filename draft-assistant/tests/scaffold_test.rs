// Integration tests for the draft assistant scaffold.

use std::path::Path;

/// Verify that the project scaffold compiles successfully.
#[test]
fn project_compiles() {
    assert!(true);
}

/// Verify that config/league.toml is valid TOML.
#[test]
fn league_toml_is_valid() {
    let content = std::fs::read_to_string("config/league.toml").expect("config/league.toml should exist");
    let parsed: Result<toml::Value, _> = toml::from_str(&content);
    assert!(parsed.is_ok(), "config/league.toml is not valid TOML: {:?}", parsed.err());
}

/// Verify that config/strategy.toml is valid TOML.
#[test]
fn strategy_toml_is_valid() {
    let content =
        std::fs::read_to_string("config/strategy.toml").expect("config/strategy.toml should exist");
    let parsed: Result<toml::Value, _> = toml::from_str(&content);
    assert!(parsed.is_ok(), "config/strategy.toml is not valid TOML: {:?}", parsed.err());
}

/// Verify that config/credentials.toml.example is valid TOML.
#[test]
fn credentials_example_is_valid_toml() {
    let content = std::fs::read_to_string("config/credentials.toml.example")
        .expect("config/credentials.toml.example should exist");
    let parsed: Result<toml::Value, _> = toml::from_str(&content);
    assert!(
        parsed.is_ok(),
        "config/credentials.toml.example is not valid TOML: {:?}",
        parsed.err()
    );
}

/// Verify that extension/manifest.json is valid JSON.
#[test]
fn extension_manifest_is_valid_json() {
    let content = std::fs::read_to_string("extension/manifest.json")
        .expect("extension/manifest.json should exist");
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(
        parsed.is_ok(),
        "extension/manifest.json is not valid JSON: {:?}",
        parsed.err()
    );
}

/// Verify that all expected directories exist.
#[test]
fn directory_structure_exists() {
    let expected_dirs = [
        "src",
        "src/valuation",
        "src/draft",
        "src/llm",
        "src/tui",
        "src/tui/widgets",
        "config",
        "data",
        "data/projections",
        "extension",
        "extension/content_scripts",
        "extension/icons",
        "tests",
        "tests/fixtures",
    ];
    for dir in expected_dirs {
        assert!(Path::new(dir).is_dir(), "Expected directory '{}' to exist", dir);
    }
}

/// Verify that all expected source files exist.
#[test]
fn source_files_exist() {
    let expected_files = [
        "src/main.rs",
        "src/lib.rs",
        "src/app.rs",
        "src/config.rs",
        "src/ws_server.rs",
        "src/protocol.rs",
        "src/db.rs",
        "src/valuation/mod.rs",
        "src/valuation/projections.rs",
        "src/valuation/zscore.rs",
        "src/valuation/vor.rs",
        "src/valuation/auction.rs",
        "src/valuation/scarcity.rs",
        "src/draft/mod.rs",
        "src/draft/state.rs",
        "src/draft/roster.rs",
        "src/draft/pick.rs",
        "src/llm/mod.rs",
        "src/llm/client.rs",
        "src/llm/prompt.rs",
        "src/tui/mod.rs",
        "src/tui/layout.rs",
        "src/tui/input.rs",
        "src/tui/widgets/mod.rs",
        "src/tui/widgets/nomination_banner.rs",
        "src/tui/widgets/llm_analysis.rs",
        "src/tui/widgets/nomination_plan.rs",
        "src/tui/widgets/available.rs",
        "src/tui/widgets/draft_log.rs",
        "src/tui/widgets/teams.rs",
        "src/tui/widgets/roster.rs",
        "src/tui/widgets/scarcity.rs",
        "src/tui/widgets/budget.rs",
        "src/tui/widgets/status_bar.rs",
    ];
    for file in expected_files {
        assert!(Path::new(file).is_file(), "Expected source file '{}' to exist", file);
    }
}

/// Verify that data CSV files have correct headers.
#[test]
fn csv_files_have_headers() {
    let holds_content =
        std::fs::read_to_string("data/holds_projections.csv").expect("holds_projections.csv should exist");
    assert!(
        holds_content.starts_with("Name,Team,HD"),
        "holds_projections.csv should have correct headers"
    );

    let adp_content = std::fs::read_to_string("data/adp.csv").expect("adp.csv should exist");
    assert!(adp_content.starts_with("Name,ADP"), "adp.csv should have correct headers");
}

/// Verify league.toml contains expected league settings.
#[test]
fn league_toml_has_correct_settings() {
    let content = std::fs::read_to_string("config/league.toml").unwrap();
    let config: toml::Value = toml::from_str(&content).unwrap();

    let league = config.get("league").expect("league section should exist");
    assert_eq!(league.get("num_teams").unwrap().as_integer().unwrap(), 10);
    assert_eq!(league.get("salary_cap").unwrap().as_integer().unwrap(), 260);
    assert_eq!(
        league.get("scoring_type").unwrap().as_str().unwrap(),
        "h2h_most_categories"
    );

    let batting = league
        .get("batting_categories")
        .expect("batting_categories should exist");
    let batting_cats: Vec<&str> = batting
        .get("categories")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(batting_cats, vec!["R", "HR", "RBI", "BB", "SB", "AVG"]);

    let pitching = league
        .get("pitching_categories")
        .expect("pitching_categories should exist");
    let pitching_cats: Vec<&str> = pitching
        .get("categories")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(pitching_cats, vec!["K", "W", "SV", "HD", "ERA", "WHIP"]);
}

/// Verify strategy.toml contains expected strategy settings.
#[test]
fn strategy_toml_has_correct_settings() {
    let content = std::fs::read_to_string("config/strategy.toml").unwrap();
    let config: toml::Value = toml::from_str(&content).unwrap();

    let budget = config.get("budget").expect("budget section should exist");
    let hitting_frac = budget
        .get("hitting_budget_fraction")
        .unwrap()
        .as_float()
        .unwrap();
    assert!((hitting_frac - 0.65).abs() < f64::EPSILON);

    let weights = config
        .get("category_weights")
        .expect("category_weights should exist");
    let sv_weight = weights.get("SV").unwrap().as_float().unwrap();
    assert!((sv_weight - 0.7).abs() < f64::EPSILON);

    let llm = config.get("llm").expect("llm section should exist");
    assert_eq!(
        llm.get("model").unwrap().as_str().unwrap(),
        "claude-sonnet-4-5-20250929"
    );
    assert_eq!(llm.get("analysis_max_tokens").unwrap().as_integer().unwrap(), 400);
    assert_eq!(llm.get("planning_max_tokens").unwrap().as_integer().unwrap(), 600);
}
