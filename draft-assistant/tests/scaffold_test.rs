// Integration tests for the draft assistant scaffold.

use std::path::Path;

/// Verify that the project scaffold compiles successfully.
#[test]
fn project_compiles() {
    assert!(true);
}

/// Verify that the default credentials config has no keys set.
#[test]
fn default_credentials_config_is_empty() {
    let config = draft_assistant::config::CredentialsConfig::default();
    assert!(config.anthropic_api_key.is_none());
    assert!(config.google_api_key.is_none());
    assert!(config.openai_api_key.is_none());
}

/// Verify that browser-specific manifest.json files are valid JSON.
#[test]
fn extension_manifests_are_valid_json() {
    let manifests = ["extension/manifest.json", "extension/chrome/manifest.json"];
    for path in manifests {
        let content =
            std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{} should exist", path));
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
        assert!(
            parsed.is_ok(),
            "{} is not valid JSON: {:?}",
            path,
            parsed.err()
        );
    }
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
        "projections",
        "extension",
        "extension/content_scripts",
        "extension/chrome",
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
        "src/app/mod.rs",
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
        "src/tui/app.rs",
        "src/tui/widgets/mod.rs",
        "src/tui/widgets/nomination_banner.rs",
        "src/tui/draft/main_panel/mod.rs",
        "src/tui/draft/main_panel/analysis.rs",
        "src/tui/draft/sidebar/plan.rs",
        "src/tui/draft/main_panel/available.rs",
        "src/tui/draft/mod.rs",
        "src/tui/draft/draft_log.rs",
        "src/tui/draft/teams.rs",
        "src/tui/draft/sidebar/mod.rs",
        "src/tui/draft/sidebar/roster.rs",
        "src/tui/draft/sidebar/scarcity.rs",
        "src/tui/widgets/budget.rs",
        "src/tui/widgets/status_bar.rs",
    ];
    for file in expected_files {
        assert!(Path::new(file).is_file(), "Expected source file '{}' to exist", file);
    }
}

/// Verify that the in-code default league config has correct settings
/// (replaces the old test that read defaults/league.toml directly).
#[test]
fn league_config_has_correct_settings() {
    let config = draft_assistant::config::LeagueConfig::default();

    assert_eq!(config.num_teams, 10);
    assert_eq!(config.salary_cap, 260);
    assert_eq!(config.scoring_type, "h2h_most_categories");

    assert_eq!(
        config.batting_categories.categories,
        vec!["R", "HR", "RBI", "BB", "SB", "AVG"]
    );
    assert_eq!(
        config.pitching_categories.categories,
        vec!["K", "W", "SV", "HD", "ERA", "WHIP"]
    );
}

/// Verify that the in-code default strategy config has correct settings
/// (replaces the old test that read defaults/strategy.toml directly).
#[test]
fn strategy_config_has_correct_settings() {
    let config = draft_assistant::config::StrategyConfig::default();

    assert!((config.hitting_budget_fraction - 0.65).abs() < f64::EPSILON);
    assert!((config.weights.get("SV").unwrap() - 0.7).abs() < f64::EPSILON);
    assert_eq!(config.llm.model, "claude-sonnet-4-6");
    assert_eq!(
        config.llm.provider,
        draft_assistant::llm::provider::LlmProvider::Anthropic
    );
    assert_eq!(config.llm.analysis_max_tokens, 2048);
    assert_eq!(config.llm.planning_max_tokens, 2048);
}
