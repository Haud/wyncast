// Integration tests for the draft assistant scaffold.

use std::path::Path;

/// CARGO_MANIFEST_DIR is `crates/wyncast-tui`; workspace root is two dirs up.
fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn crate_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Verify that the project scaffold compiles successfully.
#[test]
fn project_compiles() {
    assert!(true);
}

/// Verify that the default credentials config has no keys set.
#[test]
fn default_credentials_config_is_empty() {
    let config = wyncast_tui::config::CredentialsConfig::default();
    assert!(config.anthropic_api_key.is_none());
    assert!(config.google_api_key.is_none());
    assert!(config.openai_api_key.is_none());
}

/// Verify that browser-specific manifest.json files are valid JSON.
#[test]
fn extension_manifests_are_valid_json() {
    let root = workspace_root();
    let manifests = [
        root.join("extension/manifest.json"),
        root.join("extension/chrome/manifest.json"),
    ];
    for path in &manifests {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|_| panic!("{} should exist", path.display()));
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
        assert!(
            parsed.is_ok(),
            "{} is not valid JSON: {:?}",
            path.display(),
            parsed.err()
        );
    }
}

/// Verify that all expected directories exist.
#[test]
fn directory_structure_exists() {
    let crate_dir = crate_root();
    let workspace_dir = workspace_root();

    let crate_dirs = [
        "src",
        "src/llm",
        "src/tui",
        "src/tui/widgets",
        "tests",
        "tests/fixtures",
    ];
    for dir in &crate_dirs {
        let full = crate_dir.join(dir);
        assert!(full.is_dir(), "Expected crate directory '{}' to exist", dir);
    }

    let workspace_dirs = [
        "projections",
        "extension",
        "extension/content_scripts",
        "extension/chrome",
        "extension/icons",
    ];
    for dir in &workspace_dirs {
        let full = workspace_dir.join(dir);
        assert!(
            full.is_dir(),
            "Expected workspace directory '{}' to exist",
            dir
        );
    }
}

/// Verify that all expected source files exist.
#[test]
fn source_files_exist() {
    let crate_dir = crate_root();
    // Files remaining in wyncast-tui
    let tui_files = [
        "src/main.rs",
        "src/lib.rs",
        "src/llm/mod.rs",
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
    for file in &tui_files {
        let full = crate_dir.join(file);
        assert!(
            full.is_file(),
            "Expected source file '{}' to exist",
            file
        );
    }

    // Files moved to wyncast-core, wyncast-llm, and wyncast-baseball
    let workspace_dir = workspace_root();
    let core_files = [
        "crates/wyncast-core/src/config.rs",
        "crates/wyncast-core/src/ws_server.rs",
        "crates/wyncast-core/src/db.rs",
        "crates/wyncast-core/src/stats.rs",
        "crates/wyncast-core/src/app_dirs.rs",
        "crates/wyncast-core/src/migrations.rs",
        "crates/wyncast-core/src/picks.rs",
        "crates/wyncast-llm/src/client.rs",
        "crates/wyncast-baseball/src/draft/mod.rs",
        "crates/wyncast-baseball/src/draft/pick.rs",
        "crates/wyncast-baseball/src/draft/roster.rs",
        "crates/wyncast-baseball/src/draft/state.rs",
        "crates/wyncast-baseball/src/valuation/mod.rs",
        "crates/wyncast-baseball/src/valuation/projections.rs",
        "crates/wyncast-baseball/src/valuation/zscore.rs",
        "crates/wyncast-baseball/src/valuation/vor.rs",
        "crates/wyncast-baseball/src/valuation/auction.rs",
        "crates/wyncast-baseball/src/valuation/scarcity.rs",
        "crates/wyncast-baseball/src/llm/mod.rs",
        "crates/wyncast-baseball/src/llm/prompt.rs",
        "crates/wyncast-baseball/src/matchup/mod.rs",
        "crates/wyncast-app/src/app/mod.rs",
        "crates/wyncast-app/src/protocol.rs",
        "crates/wyncast-app/src/onboarding/mod.rs",
    ];
    for file in &core_files {
        let full = workspace_dir.join(file);
        assert!(
            full.is_file(),
            "Expected core source file '{}' to exist",
            file
        );
    }
}

/// Verify that the in-code default league config has correct settings
/// (replaces the old test that read defaults/league.toml directly).
#[test]
fn league_config_has_correct_settings() {
    let config = wyncast_tui::config::LeagueConfig::default();

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
    let config = wyncast_tui::config::StrategyConfig::default();

    assert!((config.hitting_budget_fraction - 0.65).abs() < f64::EPSILON);
    assert!((config.weights.get("SV").unwrap() - 0.7).abs() < f64::EPSILON);
    assert_eq!(config.llm.model, "claude-sonnet-4-6");
    assert_eq!(
        config.llm.provider,
        wyncast_tui::llm::provider::LlmProvider::Anthropic
    );
    assert_eq!(config.llm.analysis_max_tokens, 2048);
    assert_eq!(config.llm.planning_max_tokens, 2048);
}
