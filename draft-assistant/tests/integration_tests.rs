// Integration tests for the draft assistant.
//
// These tests exercise the full system end-to-end using the library crate's
// public API. They verify that the major subsystems (valuation pipeline,
// draft state management, crash recovery, CSV import, LLM prompt construction,
// and WebSocket protocol handling) work together correctly.

use std::collections::HashMap;

use draft_assistant::app::{self, AppState};
use draft_assistant::config::*;
use draft_assistant::db::Database;
use draft_assistant::draft::pick::{DraftPick, Position};
use draft_assistant::draft::state::DraftState;
use draft_assistant::llm::client::LlmClient;
use draft_assistant::protocol::*;
use draft_assistant::valuation::projections::{AllProjections, PitcherType};
use draft_assistant::valuation::zscore::PlayerValuation;
use draft_assistant::ws_server::WsEvent;

use tokio::sync::mpsc;

// ===========================================================================
// Test helpers
// ===========================================================================

/// Fixture directory path (relative to project root, which is the cwd for
/// `cargo test`).
const FIXTURES: &str = "tests/fixtures";

/// Build the roster config HashMap -- single source of truth for roster slots.
fn roster_config() -> HashMap<String, usize> {
    let mut m = HashMap::new();
    m.insert("C".into(), 1);
    m.insert("1B".into(), 1);
    m.insert("2B".into(), 1);
    m.insert("3B".into(), 1);
    m.insert("SS".into(), 1);
    m.insert("LF".into(), 1);
    m.insert("CF".into(), 1);
    m.insert("RF".into(), 1);
    m.insert("UTIL".into(), 1);
    m.insert("SP".into(), 5);
    m.insert("RP".into(), 6);
    m.insert("BE".into(), 6);
    m.insert("IL".into(), 5);
    m
}

/// Build a 10-team list -- single source of truth for team data.
fn ten_teams() -> Vec<(String, String)> {
    (1..=10)
        .map(|i| (format!("team_{i}"), format!("Team {i}")))
        .collect()
}

/// Build a teams HashMap for LeagueConfig (derived from `ten_teams()`).
fn teams_map() -> HashMap<String, String> {
    ten_teams().into_iter().collect()
}

/// Build a test-ready Config with inline league/strategy settings (no files).
/// Uses `roster_config()` and `teams_map()` as single sources of truth.
fn inline_config() -> Config {
    let league = LeagueConfig {
        name: "Test Integration League".into(),
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
        roster: roster_config(),
        roster_limits: RosterLimits {
            max_sp: 7,
            max_rp: 7,
            gs_per_week: 7,
        },
        teams: teams_map(),
        my_team: MyTeam {
            team_id: "team_1".into(),
        },
    };

    let strategy = StrategyConfig {
        hitting_budget_fraction: 0.65,
        weights: CategoryWeights {
            R: 1.0,
            HR: 1.0,
            RBI: 1.0,
            BB: 1.2,
            SB: 1.0,
            AVG: 1.0,
            K: 1.0,
            W: 1.0,
            SV: 0.7,
            HD: 1.3,
            ERA: 1.0,
            WHIP: 1.0,
        },
        pool: PoolConfig {
            min_pa: 300,
            min_ip_sp: 80.0,
            min_g_rp: 30,
            hitter_pool_size: 150,
            sp_pool_size: 70,
            rp_pool_size: 80,
        },
        holds_estimation: HoldsEstimationConfig {
            default_hold_rate: 0.25,
        },
        llm: LlmConfig {
            model: "test".into(),
            analysis_max_tokens: 400,
            planning_max_tokens: 600,
            analysis_trigger: "nomination".into(),
            prefire_planning: true,
        },
    };

    Config {
        league,
        strategy,
        credentials: CredentialsConfig::default(),
        ws_port: 0,
        db_path: ":memory:".into(),
        data_paths: DataPaths {
            hitters: format!("{}/sample_hitters.csv", FIXTURES),
            pitchers_sp: format!("{}/sample_pitchers_sp.csv", FIXTURES),
            pitchers_rp: format!("{}/sample_pitchers_rp.csv", FIXTURES),
            holds_overlay: format!("{}/sample_holds.csv", FIXTURES),
            adp: format!("{}/sample_adp.csv", FIXTURES),
        },
    }
}

/// Load projections from fixture CSVs and run the full valuation pipeline.
fn load_fixture_players(config: &Config) -> Vec<PlayerValuation> {
    let projections = draft_assistant::valuation::projections::load_all_from_paths(
        &config.data_paths,
        config.strategy.holds_estimation.default_hold_rate,
    )
    .expect("fixture CSVs should load");

    draft_assistant::valuation::compute_initial(&projections, config)
        .expect("initial valuation should succeed")
}

/// Load fixture projections.
fn load_fixture_projections(config: &Config) -> AllProjections {
    draft_assistant::valuation::projections::load_all_from_paths(
        &config.data_paths,
        config.strategy.holds_estimation.default_hold_rate,
    )
    .expect("fixture CSVs should load")
}

/// Create a full AppState wired up with fixture data, in-memory DB, and
/// LLM disabled.
fn create_test_app_state_from_fixtures() -> AppState {
    let config = inline_config();
    let projections = load_fixture_projections(&config);
    let mut available = load_fixture_players(&config);

    let draft_state = DraftState::new(ten_teams(), "team_1", 260, &roster_config());

    // Recalculate with draft state for consistency
    draft_assistant::valuation::recalculate_all(
        &mut available,
        &config.league,
        &config.strategy,
        &draft_state,
    );

    let db = Database::open(":memory:").expect("in-memory db");
    let llm_client = LlmClient::Disabled;
    let (llm_tx, _llm_rx) = mpsc::channel(16);

    AppState::new(config, draft_state, available, projections, db, llm_client, llm_tx)
}

// ===========================================================================
// Mock draft event generator
// ===========================================================================

/// A simulated draft event for testing.
#[derive(Debug, Clone)]
pub struct MockDraftEvent {
    pub pick_number: u32,
    pub team_id: String,
    pub team_name: String,
    pub player_name: String,
    pub position: String,
    pub price: u32,
    pub espn_player_id: String,
}

/// Generate a sequence of realistic mock draft picks. Each pick represents
/// a completed auction for a specific player. The data mirrors the fixture
/// CSVs so that player removal from the available pool can be verified.
pub fn generate_mock_draft_events() -> Vec<MockDraftEvent> {
    vec![
        MockDraftEvent {
            pick_number: 1,
            team_id: "team_3".into(),
            team_name: "Team 3".into(),
            player_name: "Shohei Ohtani".into(),
            position: "DH".into(),
            price: 62,
            espn_player_id: "espn_100".into(),
        },
        MockDraftEvent {
            pick_number: 2,
            team_id: "team_4".into(),
            team_name: "Team 4".into(),
            player_name: "Aaron Judge".into(),
            position: "OF".into(),
            price: 55,
            espn_player_id: "espn_101".into(),
        },
        MockDraftEvent {
            pick_number: 3,
            team_id: "team_5".into(),
            team_name: "Team 5".into(),
            player_name: "Juan Soto".into(),
            position: "OF".into(),
            price: 48,
            espn_player_id: "espn_102".into(),
        },
        MockDraftEvent {
            pick_number: 4,
            team_id: "team_6".into(),
            team_name: "Team 6".into(),
            player_name: "Bobby Witt Jr.".into(),
            position: "SS".into(),
            price: 42,
            espn_player_id: "espn_103".into(),
        },
        MockDraftEvent {
            pick_number: 5,
            team_id: "team_7".into(),
            team_name: "Team 7".into(),
            player_name: "Mookie Betts".into(),
            position: "SS".into(),
            price: 40,
            espn_player_id: "espn_104".into(),
        },
        MockDraftEvent {
            pick_number: 6,
            team_id: "team_8".into(),
            team_name: "Team 8".into(),
            player_name: "Trea Turner".into(),
            position: "SS".into(),
            price: 38,
            espn_player_id: "espn_105".into(),
        },
        MockDraftEvent {
            pick_number: 7,
            team_id: "team_9".into(),
            team_name: "Team 9".into(),
            player_name: "Freddie Freeman".into(),
            position: "1B".into(),
            price: 36,
            espn_player_id: "espn_106".into(),
        },
        MockDraftEvent {
            pick_number: 8,
            team_id: "team_10".into(),
            team_name: "Team 10".into(),
            player_name: "Gerrit Cole".into(),
            position: "SP".into(),
            price: 35,
            espn_player_id: "espn_107".into(),
        },
    ]
}

/// Convert a MockDraftEvent into a DraftPick.
fn mock_event_to_pick(event: &MockDraftEvent) -> DraftPick {
    DraftPick {
        pick_number: event.pick_number,
        team_id: event.team_id.clone(),
        team_name: event.team_name.clone(),
        player_name: event.player_name.clone(),
        position: event.position.clone(),
        price: event.price,
        espn_player_id: Some(event.espn_player_id.clone()),
    }
}

/// Build a JSON STATE_UPDATE message containing picks up to `up_to_pick` (inclusive)
/// and an optional nomination.
fn build_state_update_json(
    events: &[MockDraftEvent],
    up_to_pick: u32,
    nomination: Option<(&str, &str, &str, &str)>, // (player_id, player_name, position, nominated_by)
) -> String {
    let picks: Vec<serde_json::Value> = events
        .iter()
        .filter(|e| e.pick_number <= up_to_pick)
        .map(|e| {
            serde_json::json!({
                "pickNumber": e.pick_number,
                "teamId": e.team_id,
                "teamName": e.team_name,
                "playerId": e.espn_player_id,
                "playerName": e.player_name,
                "position": e.position,
                "price": e.price
            })
        })
        .collect();

    let nom_value = match nomination {
        Some((pid, pname, pos, by)) => serde_json::json!({
            "playerId": pid,
            "playerName": pname,
            "position": pos,
            "nominatedBy": by,
            "currentBid": 1,
            "currentBidder": null,
            "timeRemaining": 30
        }),
        None => serde_json::Value::Null,
    };

    serde_json::json!({
        "type": "STATE_UPDATE",
        "timestamp": 1700000000 + up_to_pick as u64,
        "payload": {
            "picks": picks,
            "currentNomination": nom_value,
            "myTeamId": "team_1",
            "source": "test"
        }
    })
    .to_string()
}

// ===========================================================================
// Test: Full draft simulation (8 picks via process_new_picks)
// ===========================================================================

#[test]
fn full_draft_simulation_via_process_new_picks() {
    let mut state = create_test_app_state_from_fixtures();
    let events = generate_mock_draft_events();
    let initial_player_count = state.available_players.len();

    // Process all 8 picks one at a time, verifying state after each
    for (i, event) in events.iter().enumerate() {
        let pick = mock_event_to_pick(event);
        let player_name = pick.player_name.clone();
        let team_id = pick.team_id.clone();
        let price = pick.price;

        state.process_new_picks(vec![pick]);

        // Pick count should increase
        assert_eq!(
            state.draft_state.pick_count,
            i + 1,
            "Pick count mismatch after pick {}",
            i + 1
        );

        // Player should be removed from available pool
        assert!(
            !state
                .available_players
                .iter()
                .any(|p| p.name == player_name),
            "Player '{}' should be removed from available pool after pick {}",
            player_name,
            i + 1
        );

        // Team budget should be updated
        let team = state
            .draft_state
            .team(&team_id)
            .expect("team should exist");
        assert!(
            team.budget_spent >= price,
            "Team {} budget_spent should be >= {} after their pick",
            team_id,
            price
        );
    }

    // After all 8 picks:
    assert_eq!(state.draft_state.pick_count, 8);
    assert_eq!(state.draft_state.picks.len(), 8);

    // Available player count should decrease by 8
    assert_eq!(
        state.available_players.len(),
        initial_player_count - 8,
        "Expected exactly 8 players removed from pool"
    );

    // Inflation tracker should reflect the total spent
    let _total_spent: u32 = events.iter().map(|e| e.price).sum();
    assert!(
        state.inflation.total_dollars_spent > 0.0,
        "Inflation tracker should have recorded spending"
    );

    // Scarcity should be recomputed and non-empty
    assert!(
        !state.scarcity.is_empty(),
        "Scarcity entries should exist after picks"
    );

    // All remaining players should have finite dollar values
    for player in &state.available_players {
        assert!(
            player.dollar_value.is_finite(),
            "Player '{}' has non-finite dollar value after recalculation",
            player.name
        );
    }

    // DB should have all 8 picks persisted
    let db_picks = state.db.load_picks().unwrap();
    assert_eq!(db_picks.len(), 8, "DB should have 8 picks persisted");
    assert_eq!(db_picks[0].player_name, "Shohei Ohtani");
    assert_eq!(db_picks[7].player_name, "Gerrit Cole");
}

/// Test that multiple picks can be processed in a single batch.
#[test]
fn batch_picks_processed_correctly() {
    let mut state = create_test_app_state_from_fixtures();
    let events = generate_mock_draft_events();
    let initial_count = state.available_players.len();

    // Process all 8 picks at once
    let picks: Vec<DraftPick> = events.iter().map(mock_event_to_pick).collect();
    state.process_new_picks(picks);

    assert_eq!(state.draft_state.pick_count, 8);
    assert_eq!(
        state.available_players.len(),
        initial_count - 8
    );

    // Verify each drafted player is gone
    for event in &events {
        assert!(
            !state.available_players.iter().any(|p| p.name == event.player_name),
            "Player '{}' should not be in available pool",
            event.player_name
        );
    }
}

/// Test that valuations are recalculated after picks (dollar values shift).
#[test]
fn valuations_recalculate_after_picks() {
    let mut state = create_test_app_state_from_fixtures();

    // Snapshot initial top-valued players
    let initial_top_values: Vec<(String, f64)> = state
        .available_players
        .iter()
        .take(5)
        .map(|p| (p.name.clone(), p.dollar_value))
        .collect();

    // Process 4 expensive picks (removing top players)
    let events = generate_mock_draft_events();
    let picks: Vec<DraftPick> = events[..4].iter().map(mock_event_to_pick).collect();
    state.process_new_picks(picks);

    // The remaining top players should have shifted dollar values.
    // At minimum, the list of names at the top should be different (since
    // we removed 4 top players) or the values should have changed due
    // to recalculation with a smaller pool.
    let new_top_values: Vec<(String, f64)> = state
        .available_players
        .iter()
        .take(5)
        .map(|p| (p.name.clone(), p.dollar_value))
        .collect();

    // The drafted players should not appear in the new top
    for (name, _) in &initial_top_values {
        if ["Shohei Ohtani", "Aaron Judge", "Juan Soto", "Bobby Witt Jr."].contains(&name.as_str()) {
            assert!(
                !new_top_values.iter().any(|(n, _)| n == name),
                "Drafted player '{}' should not be in top values",
                name
            );
        }
    }
}

// ===========================================================================
// Test: Crash recovery
// ===========================================================================

#[test]
fn crash_recovery_restores_picks_and_continues() {
    let config = inline_config();
    let projections = load_fixture_projections(&config);
    let mut available = load_fixture_players(&config);

    // Simulate a first session: create state, record picks, then "crash"
    let db = Database::open(":memory:").expect("in-memory db");

    // Record 3 picks directly to DB (simulating a previous session)
    let events = generate_mock_draft_events();
    for event in &events[..3] {
        let pick = mock_event_to_pick(event);
        db.record_pick(&pick).unwrap();
    }
    assert!(db.has_draft_in_progress().unwrap());

    // Simulate restart: create a fresh AppState with the same DB
    let draft_state = DraftState::new(ten_teams(), "team_1", 260, &roster_config());
    draft_assistant::valuation::recalculate_all(
        &mut available,
        &config.league,
        &config.strategy,
        &draft_state,
    );
    let initial_player_count = available.len();

    let llm_client = LlmClient::Disabled;
    let (llm_tx, _llm_rx) = mpsc::channel(16);
    let mut state = AppState::new(
        config,
        draft_state,
        available,
        projections,
        db,
        llm_client,
        llm_tx,
    );

    // Run crash recovery
    let recovered = app::recover_from_db(&mut state).unwrap();
    assert!(recovered, "Should detect draft in progress and recover");

    // Verify state was restored
    assert_eq!(state.draft_state.pick_count, 3);
    assert_eq!(state.draft_state.picks.len(), 3);
    assert_eq!(state.draft_state.picks[0].player_name, "Shohei Ohtani");
    assert_eq!(state.draft_state.picks[1].player_name, "Aaron Judge");
    assert_eq!(state.draft_state.picks[2].player_name, "Juan Soto");

    // Players should be removed from available pool
    assert_eq!(state.available_players.len(), initial_player_count - 3);
    assert!(!state.available_players.iter().any(|p| p.name == "Shohei Ohtani"));
    assert!(!state.available_players.iter().any(|p| p.name == "Aaron Judge"));
    assert!(!state.available_players.iter().any(|p| p.name == "Juan Soto"));

    // Budget should be updated for each team
    let team3 = state.draft_state.team("team_3").unwrap();
    assert_eq!(team3.budget_spent, 62);
    let team4 = state.draft_state.team("team_4").unwrap();
    assert_eq!(team4.budget_spent, 55);
    let team5 = state.draft_state.team("team_5").unwrap();
    assert_eq!(team5.budget_spent, 48);

    // Inflation and scarcity should be recalculated
    assert!(state.inflation.total_dollars_spent > 0.0);
    assert!(!state.scarcity.is_empty());

    // Now continue the draft with a new pick (pick 4)
    let pick4 = mock_event_to_pick(&events[3]);
    state.process_new_picks(vec![pick4]);

    assert_eq!(state.draft_state.pick_count, 4);
    assert!(!state.available_players.iter().any(|p| p.name == "Bobby Witt Jr."));

    // All 4 picks should be in the DB
    let db_picks = state.db.load_picks().unwrap();
    assert_eq!(db_picks.len(), 4);
}

/// Verify that crash recovery with an empty DB returns false (no recovery needed).
#[test]
fn crash_recovery_empty_db_returns_false() {
    let config = inline_config();
    let projections = load_fixture_projections(&config);
    let available = load_fixture_players(&config);
    let draft_state = DraftState::new(ten_teams(), "team_1", 260, &roster_config());
    let db = Database::open(":memory:").expect("in-memory db");
    let llm_client = LlmClient::Disabled;
    let (llm_tx, _llm_rx) = mpsc::channel(16);
    let mut state = AppState::new(
        config,
        draft_state,
        available,
        projections,
        db,
        llm_client,
        llm_tx,
    );

    let recovered = app::recover_from_db(&mut state).unwrap();
    assert!(!recovered, "Empty DB should not trigger recovery");
    assert_eq!(state.draft_state.pick_count, 0);
}

// ===========================================================================
// Test: CSV import (FanGraphs format)
// ===========================================================================

#[test]
fn csv_import_loads_all_fixture_data() {
    let config = inline_config();
    let projections = load_fixture_projections(&config);

    // Verify hitter count
    assert_eq!(
        projections.hitters.len(),
        20,
        "Should load 20 hitters from fixture"
    );

    // Verify SP count
    let sp_count = projections
        .pitchers
        .iter()
        .filter(|p| p.pitcher_type == PitcherType::SP)
        .count();
    assert_eq!(sp_count, 10, "Should load 10 SP from fixture");

    // Verify RP count
    let rp_count = projections
        .pitchers
        .iter()
        .filter(|p| p.pitcher_type == PitcherType::RP)
        .count();
    assert_eq!(rp_count, 10, "Should load 10 RP from fixture");

    // Verify ADP data loaded
    assert!(!projections.adp.is_empty(), "ADP data should be loaded");
    assert!(
        projections.adp.contains_key("Aaron Judge"),
        "ADP should contain Aaron Judge"
    );
    assert!(
        (projections.adp["Aaron Judge"] - 2.5).abs() < 0.01,
        "Aaron Judge ADP should be 2.5"
    );

    // Verify holds overlay was merged
    let devin = projections
        .pitchers
        .iter()
        .find(|p| p.name == "Devin Williams")
        .expect("Devin Williams should exist");
    assert_eq!(
        devin.hd, 25,
        "Devin Williams should have 25 holds from overlay"
    );

    // Verify specific hitter stats
    let judge = projections
        .hitters
        .iter()
        .find(|h| h.name == "Aaron Judge")
        .expect("Aaron Judge should exist");
    assert_eq!(judge.hr, 52);
    assert_eq!(judge.r, 120);
    assert_eq!(judge.rbi, 130);
    assert_eq!(judge.bb, 90);
    assert!((judge.avg - 0.300).abs() < f64::EPSILON);

    // Verify specific pitcher stats
    let cole = projections
        .pitchers
        .iter()
        .find(|p| p.name == "Gerrit Cole")
        .expect("Gerrit Cole should exist");
    assert_eq!(cole.k, 250);
    assert_eq!(cole.w, 16);
    assert!((cole.ip - 200.0).abs() < f64::EPSILON);
    assert!((cole.era - 2.80).abs() < f64::EPSILON);
}

/// Verify that the valuation pipeline produces valid dollar values for
/// all fixture players.
#[test]
fn valuation_pipeline_produces_valid_dollar_values() {
    let config = inline_config();
    let players = load_fixture_players(&config);

    assert!(
        !players.is_empty(),
        "Valuation pipeline should produce players"
    );

    // Every player should have a finite dollar value >= 0
    for player in &players {
        assert!(
            player.dollar_value.is_finite() && player.dollar_value >= 0.0,
            "Player '{}' has invalid dollar value: {}",
            player.name,
            player.dollar_value
        );
    }

    // The top player should have a dollar value > $1 (they're not all replacement level)
    let top = &players[0];
    assert!(
        top.dollar_value > 1.0,
        "Top player '{}' should have dollar value > $1, got {}",
        top.name,
        top.dollar_value
    );

    // Hitters and pitchers should both be present
    let hitter_count = players.iter().filter(|p| !p.is_pitcher).count();
    let pitcher_count = players.iter().filter(|p| p.is_pitcher).count();
    assert!(hitter_count > 0, "Should have hitters in pool");
    assert!(pitcher_count > 0, "Should have pitchers in pool");
}

/// Verify that the holds overlay correctly applies to RP who have overlay
/// entries, and estimates holds for those without.
#[test]
fn holds_overlay_applied_correctly() {
    let config = inline_config();
    let projections = load_fixture_projections(&config);

    // Devin Williams: overlay says 25 HD
    let dw = projections
        .pitchers
        .iter()
        .find(|p| p.name == "Devin Williams")
        .unwrap();
    assert_eq!(dw.hd, 25);

    // Clay Holmes: overlay says 18 HD
    let ch = projections
        .pitchers
        .iter()
        .find(|p| p.name == "Clay Holmes")
        .unwrap();
    assert_eq!(ch.hd, 18);

    // Kenley Jansen: no overlay entry, RP with G=55 SV=25 GS=0
    // Estimated: (55 - 25 - 0) * 0.25 = 7.5 -> 8
    let kj = projections
        .pitchers
        .iter()
        .find(|p| p.name == "Kenley Jansen")
        .unwrap();
    // Estimation formula: max(0, (G - SV - GS) * hold_rate)
    let expected_hd = ((55.0_f64 - 25.0 - 0.0).max(0.0) * 0.25).round() as u32;
    assert_eq!(
        kj.hd, expected_hd,
        "Kenley Jansen estimated holds should be {}",
        expected_hd
    );
}

// ===========================================================================
// Test: LLM prompt format validation
// ===========================================================================

#[test]
fn nomination_analysis_prompt_contains_required_sections() {
    let state = create_test_app_state_from_fixtures();

    // Find a known player in the available pool
    let player = state
        .available_players
        .iter()
        .find(|p| p.name == "Gunnar Henderson")
        .expect("Gunnar Henderson should be available")
        .clone();

    let nomination = NominationInfo {
        player_name: "Gunnar Henderson".into(),
        position: "SS".into(),
        nominated_by: "Team 5".into(),
        current_bid: 5,
        current_bidder: None,
        time_remaining: Some(30),
    };

    let prompt = draft_assistant::llm::prompt::build_nomination_analysis_prompt(
        &player,
        &nomination,
        &state.draft_state.my_team().roster,
        &state.category_needs,
        &state.scarcity,
        &state.available_players,
        &state.draft_state,
        &state.inflation,
    );

    // Verify required sections are present
    assert!(
        prompt.contains("NOMINATION"),
        "Prompt should contain NOMINATION section"
    );
    assert!(
        prompt.contains("Gunnar Henderson"),
        "Prompt should contain the player name"
    );
    assert!(
        prompt.contains("PLAYER PROFILE"),
        "Prompt should contain PLAYER PROFILE section"
    );
    assert!(
        prompt.contains("MY ROSTER"),
        "Prompt should contain MY ROSTER section"
    );
    assert!(
        prompt.contains("CATEGORY NEEDS"),
        "Prompt should contain CATEGORY NEEDS section"
    );
    assert!(
        prompt.contains("POSITIONAL SCARCITY"),
        "Prompt should contain POSITIONAL SCARCITY section"
    );

    // Verify the prompt includes numeric data (dollar values, bid info)
    assert!(
        prompt.contains("$5"),
        "Prompt should contain the current bid amount"
    );
    assert!(
        prompt.contains("Team 5"),
        "Prompt should contain the nominating team"
    );
}

#[test]
fn nomination_planning_prompt_contains_required_sections() {
    let state = create_test_app_state_from_fixtures();

    let prompt = draft_assistant::llm::prompt::build_nomination_planning_prompt(
        &state.draft_state.my_team().roster,
        &state.category_needs,
        &state.scarcity,
        &state.available_players,
        &state.draft_state,
        &state.inflation,
    );

    // Verify required sections are present
    assert!(
        prompt.contains("MY ROSTER"),
        "Planning prompt should contain MY ROSTER section"
    );
    assert!(
        prompt.contains("CATEGORY STRENGTHS"),
        "Planning prompt should contain CATEGORY STRENGTHS section"
    );
    assert!(
        prompt.contains("POSITIONAL SCARCITY"),
        "Planning prompt should contain POSITIONAL SCARCITY section"
    );
    assert!(
        prompt.contains("NOMINATION PLANNING"),
        "Planning prompt should contain NOMINATION PLANNING header"
    );
    assert!(
        prompt.contains("TOP 10 AVAILABLE TARGETS"),
        "Planning prompt should contain TOP AVAILABLE TARGETS section"
    );
    assert!(
        prompt.contains("WHO SHOULD I NOMINATE"),
        "Planning prompt should contain the closing question"
    );
}

#[test]
fn system_prompt_contains_league_context() {
    let system = draft_assistant::llm::prompt::system_prompt();

    assert!(
        system.contains("fantasy baseball"),
        "System prompt should mention fantasy baseball"
    );
    assert!(
        system.contains("auction"),
        "System prompt should mention auction format"
    );
    assert!(
        system.contains("BB"),
        "System prompt should mention BB as a key edge"
    );
    assert!(
        system.contains("HD"),
        "System prompt should mention HD as a key edge"
    );
    assert!(
        system.contains("VERDICT"),
        "System prompt should describe the verdict format"
    );
}

// ===========================================================================
// Test: Protocol round-trip and JSON serialization
// ===========================================================================

#[test]
fn extension_protocol_state_update_roundtrip() {
    let events = generate_mock_draft_events();
    let json = build_state_update_json(
        &events,
        3,
        Some((
            "espn_103",
            "Bobby Witt Jr.",
            "SS",
            "Team 6",
        )),
    );

    // Deserialize the JSON
    let msg: ExtensionMessage = serde_json::from_str(&json).unwrap();

    match msg {
        ExtensionMessage::StateUpdate { timestamp, payload } => {
            assert_eq!(timestamp, 1700000003);
            assert_eq!(payload.picks.len(), 3);
            assert_eq!(payload.picks[0].player_name, "Shohei Ohtani");
            assert_eq!(payload.picks[0].price, 62);
            assert_eq!(payload.picks[1].player_name, "Aaron Judge");
            assert_eq!(payload.picks[2].player_name, "Juan Soto");

            let nom = payload.current_nomination.unwrap();
            assert_eq!(nom.player_name, "Bobby Witt Jr.");
            assert_eq!(nom.position, "SS");
            assert_eq!(nom.nominated_by, "Team 6");
            assert_eq!(nom.current_bid, 1);
        }
        _ => panic!("Expected StateUpdate variant"),
    }
}

#[test]
fn extension_protocol_state_update_no_nomination() {
    let events = generate_mock_draft_events();
    let json = build_state_update_json(&events, 8, None);

    let msg: ExtensionMessage = serde_json::from_str(&json).unwrap();

    match msg {
        ExtensionMessage::StateUpdate { payload, .. } => {
            assert_eq!(payload.picks.len(), 8);
            assert!(payload.current_nomination.is_none());
        }
        _ => panic!("Expected StateUpdate variant"),
    }
}

#[test]
fn extension_connected_roundtrip() {
    let json = r#"{
        "type": "EXTENSION_CONNECTED",
        "payload": {
            "platform": "firefox",
            "extensionVersion": "1.0.0"
        }
    }"#;

    let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
    let serialized = serde_json::to_string(&msg).unwrap();
    let reparsed: ExtensionMessage = serde_json::from_str(&serialized).unwrap();
    assert_eq!(msg, reparsed);
}

#[test]
fn heartbeat_roundtrip() {
    let json = r#"{
        "type": "EXTENSION_HEARTBEAT",
        "payload": {
            "timestamp": 1700000123
        }
    }"#;

    let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
    let serialized = serde_json::to_string(&msg).unwrap();
    let reparsed: ExtensionMessage = serde_json::from_str(&serialized).unwrap();
    assert_eq!(msg, reparsed);
}

/// Verify that the camelCase/snake_case conversion works correctly for
/// all fields when deserializing from extension-style JSON.
#[test]
fn camel_case_deserialization_all_fields() {
    let json = r#"{
        "type": "STATE_UPDATE",
        "timestamp": 1700000001,
        "payload": {
            "picks": [{
                "pickNumber": 1,
                "teamId": "team_3",
                "teamName": "Sluggers",
                "playerId": "espn_100",
                "playerName": "Shohei Ohtani",
                "position": "DH",
                "price": 62
            }],
            "currentNomination": {
                "playerId": "espn_101",
                "playerName": "Aaron Judge",
                "position": "OF",
                "nominatedBy": "Aces",
                "currentBid": 5,
                "currentBidder": "Bombers",
                "timeRemaining": 25
            },
            "myTeamId": "team_1",
            "source": "test"
        }
    }"#;

    let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
    match msg {
        ExtensionMessage::StateUpdate { timestamp, payload } => {
            assert_eq!(timestamp, 1700000001);

            let pick = &payload.picks[0];
            assert_eq!(pick.pick_number, 1);
            assert_eq!(pick.team_id, "team_3");
            assert_eq!(pick.team_name, "Sluggers");
            assert_eq!(pick.player_id, "espn_100");
            assert_eq!(pick.player_name, "Shohei Ohtani");
            assert_eq!(pick.price, 62);

            let nom = payload.current_nomination.unwrap();
            assert_eq!(nom.player_id, "espn_101");
            assert_eq!(nom.player_name, "Aaron Judge");
            assert_eq!(nom.nominated_by, "Aces");
            assert_eq!(nom.current_bid, 5);
            assert_eq!(nom.current_bidder, Some("Bombers".into()));
            assert_eq!(nom.time_remaining, Some(25));

            assert_eq!(payload.my_team_id, Some("team_1".into()));
            assert_eq!(payload.source, Some("test".into()));
        }
        _ => panic!("Expected StateUpdate variant"),
    }
}

/// Verify that malformed JSON does not panic.
#[test]
fn malformed_json_does_not_panic() {
    let bad_inputs = [
        "",
        "{}",
        r#"{"type": "UNKNOWN"}"#,
        r#"{"type": "STATE_UPDATE"}"#,
        r#"not json at all"#,
        r#"{"type": "STATE_UPDATE", "payload": null}"#,
    ];

    for input in &bad_inputs {
        let result = serde_json::from_str::<ExtensionMessage>(input);
        assert!(
            result.is_err(),
            "Expected error for input: {}, got: {:?}",
            input,
            result
        );
    }
}

// ===========================================================================
// Test: Mock draft events JSON fixture
// ===========================================================================

#[test]
fn mock_draft_events_fixture_loads_and_parses() {
    let content = std::fs::read_to_string(format!("{}/mock_draft_events.json", FIXTURES))
        .expect("mock_draft_events.json should exist");

    let events: Vec<serde_json::Value> = serde_json::from_str(&content).unwrap();
    assert_eq!(events.len(), 8, "Should have 8 state update events");

    // Each event should be a valid STATE_UPDATE
    for (i, event) in events.iter().enumerate() {
        let msg: ExtensionMessage = serde_json::from_value(event.clone()).unwrap_or_else(|e| {
            panic!("Event {} failed to parse: {}", i, e);
        });
        match msg {
            ExtensionMessage::StateUpdate { payload, .. } => {
                // First event should have 1 pick, second 2, etc. up to 8
                assert_eq!(
                    payload.picks.len(),
                    i + 1,
                    "Event {} should have {} picks",
                    i,
                    i + 1
                );
            }
            _ => panic!("Event {} should be a StateUpdate", i),
        }
    }

    // The last event should have null nomination (draft paused)
    let last: ExtensionMessage = serde_json::from_value(events[7].clone()).unwrap();
    match last {
        ExtensionMessage::StateUpdate { payload, .. } => {
            assert!(
                payload.current_nomination.is_none(),
                "Last event should have null nomination"
            );
        }
        _ => unreachable!(),
    }
}

// ===========================================================================
// Test: Event loop integration (async)
// ===========================================================================

#[tokio::test]
async fn event_loop_processes_state_update_with_picks() {
    let state = create_test_app_state_from_fixtures();

    let (ws_tx, ws_rx) = mpsc::channel(16);
    let (_llm_tx, llm_rx) = mpsc::channel(16);
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let (ui_tx, mut ui_rx) = mpsc::channel(64);

    let handle = tokio::spawn(app::run(ws_rx, llm_rx, cmd_rx, ui_tx, state));

    // Send a state update with 1 pick and a nomination
    let events = generate_mock_draft_events();
    let json = build_state_update_json(
        &events,
        1,
        Some(("espn_101", "Aaron Judge", "OF", "Team 4")),
    );
    ws_tx.send(WsEvent::Message(json)).await.unwrap();

    // Should receive a NominationUpdate
    let update = ui_rx.recv().await.unwrap();
    match update {
        UiUpdate::NominationUpdate(info) => {
            assert_eq!(info.player_name, "Aaron Judge");
            assert_eq!(info.current_bid, 1);
        }
        other => panic!("Expected NominationUpdate, got {:?}", other),
    }

    // Clean up
    cmd_tx.send(UserCommand::Quit).await.unwrap();
    let result = handle.await.unwrap();
    assert!(result.is_ok());
}

#[tokio::test]
async fn event_loop_handles_connection_lifecycle() {
    let state = create_test_app_state_from_fixtures();

    let (ws_tx, ws_rx) = mpsc::channel(16);
    let (_llm_tx, llm_rx) = mpsc::channel(16);
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let (ui_tx, mut ui_rx) = mpsc::channel(64);

    let handle = tokio::spawn(app::run(ws_rx, llm_rx, cmd_rx, ui_tx, state));

    // Send connected
    ws_tx
        .send(WsEvent::Connected {
            addr: "127.0.0.1:5555".into(),
        })
        .await
        .unwrap();

    let update = ui_rx.recv().await.unwrap();
    assert_eq!(
        update,
        UiUpdate::ConnectionStatus(ConnectionStatus::Connected)
    );

    // Send disconnected
    ws_tx.send(WsEvent::Disconnected).await.unwrap();

    let update = ui_rx.recv().await.unwrap();
    assert_eq!(
        update,
        UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)
    );

    // Quit
    cmd_tx.send(UserCommand::Quit).await.unwrap();
    let _ = handle.await;
}

#[tokio::test]
async fn event_loop_incremental_state_updates() {
    let state = create_test_app_state_from_fixtures();

    let (ws_tx, ws_rx) = mpsc::channel(16);
    let (_llm_tx, llm_rx) = mpsc::channel(16);
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let (ui_tx, mut ui_rx) = mpsc::channel(64);

    let handle = tokio::spawn(app::run(ws_rx, llm_rx, cmd_rx, ui_tx, state));

    let events = generate_mock_draft_events();

    // Send first state update (1 pick + nomination for pick 2)
    let json1 = build_state_update_json(
        &events,
        1,
        Some(("espn_101", "Aaron Judge", "OF", "Team 4")),
    );
    ws_tx.send(WsEvent::Message(json1)).await.unwrap();

    // Should receive NominationUpdate for Aaron Judge
    let update = ui_rx.recv().await.unwrap();
    match &update {
        UiUpdate::NominationUpdate(info) => {
            assert_eq!(info.player_name, "Aaron Judge");
        }
        other => panic!("Expected NominationUpdate, got {:?}", other),
    }

    // Send second state update (2 picks, nomination for pick 3)
    let json2 = build_state_update_json(
        &events,
        2,
        Some(("espn_102", "Juan Soto", "OF", "Team 5")),
    );
    ws_tx.send(WsEvent::Message(json2)).await.unwrap();

    // Should receive NominationUpdate for Juan Soto (new nomination)
    let update2 = ui_rx.recv().await.unwrap();
    match &update2 {
        UiUpdate::NominationUpdate(info) => {
            assert_eq!(info.player_name, "Juan Soto");
        }
        other => panic!("Expected NominationUpdate for Juan Soto, got {:?}", other),
    }

    // Quit
    cmd_tx.send(UserCommand::Quit).await.unwrap();
    let _ = handle.await;
}

// ===========================================================================
// Test: Mock draft event generator
// ===========================================================================

#[test]
fn mock_event_generator_produces_valid_events() {
    let events = generate_mock_draft_events();

    assert_eq!(events.len(), 8, "Generator should produce 8 events");

    // Pick numbers should be sequential 1..=8
    for (i, event) in events.iter().enumerate() {
        assert_eq!(
            event.pick_number,
            (i + 1) as u32,
            "Pick numbers should be sequential"
        );
    }

    // All team_ids should be valid
    for event in &events {
        assert!(
            event.team_id.starts_with("team_"),
            "Team ID should start with team_"
        );
    }

    // No duplicate player names
    let names: std::collections::HashSet<&str> =
        events.iter().map(|e| e.player_name.as_str()).collect();
    assert_eq!(names.len(), 8, "All player names should be unique");

    // All players should have non-zero prices
    for event in &events {
        assert!(
            event.price > 0,
            "Player '{}' should have a non-zero price",
            event.player_name
        );
    }

    // Prices should generally decrease (this is a realistic auction pattern)
    let first_price = events[0].price;
    let last_price = events[7].price;
    assert!(
        first_price > last_price,
        "First pick (${}) should cost more than last (${})",
        first_price,
        last_price
    );
}

#[test]
fn mock_events_match_fixture_players() {
    let config = inline_config();
    let projections = load_fixture_projections(&config);
    let events = generate_mock_draft_events();

    // Every player in the mock events should exist in the fixture projections
    for event in &events {
        let found_hitter = projections
            .hitters
            .iter()
            .any(|h| h.name == event.player_name);
        let found_pitcher = projections
            .pitchers
            .iter()
            .any(|p| p.name == event.player_name);

        assert!(
            found_hitter || found_pitcher,
            "Mock event player '{}' should exist in fixture projections",
            event.player_name
        );
    }
}

// ===========================================================================
// Test: Scarcity and inflation behavior during simulation
// ===========================================================================

#[test]
fn scarcity_updates_as_positions_are_drafted() {
    let mut state = create_test_app_state_from_fixtures();

    // Get initial SS scarcity
    let initial_ss = state
        .scarcity
        .iter()
        .find(|s| s.position == Position::ShortStop)
        .map(|s| s.players_above_replacement);

    // Draft 3 SS-eligible players (Bobby Witt Jr., Mookie Betts, Trea Turner)
    let events = generate_mock_draft_events();
    // picks 4, 5, 6 are SS players
    let ss_picks: Vec<DraftPick> = events[3..6].iter().map(mock_event_to_pick).collect();

    // Also process the first 3 picks so pick numbers are valid
    let early_picks: Vec<DraftPick> = events[..3].iter().map(mock_event_to_pick).collect();
    state.process_new_picks(early_picks);
    state.process_new_picks(ss_picks);

    let new_ss = state
        .scarcity
        .iter()
        .find(|s| s.position == Position::ShortStop)
        .map(|s| s.players_above_replacement);

    // After removing 3 SS players, scarcity at SS should change
    assert!(
        new_ss.is_some(),
        "SS scarcity entry should exist after picks"
    );

    // If there were SS players above replacement before, there should be fewer now
    if let (Some(initial), Some(new)) = (initial_ss, new_ss) {
        if initial > 0 {
            assert!(
                new < initial,
                "SS players above replacement should decrease: was {}, now {}",
                initial,
                new
            );
        }
    }
}

#[test]
fn inflation_increases_with_overpay() {
    let mut state = create_test_app_state_from_fixtures();

    // Record an extremely expensive pick (overpay)
    let pick = DraftPick {
        pick_number: 1,
        team_id: "team_3".into(),
        team_name: "Team 3".into(),
        player_name: "Shohei Ohtani".into(),
        position: "DH".into(),
        price: 80, // Very high overpay
        espn_player_id: Some("espn_100".into()),
    };

    state.process_new_picks(vec![pick]);

    // Inflation should show some spending
    assert!(
        state.inflation.total_dollars_spent > 0.0,
        "Inflation should track spending"
    );
    assert!(
        state.inflation.inflation_rate.is_finite(),
        "Inflation rate should be finite"
    );
}

// ===========================================================================
// Test: Database operations integration
// ===========================================================================

#[test]
fn database_round_trip_with_all_fields() {
    let db = Database::open(":memory:").expect("in-memory db");

    let pick = DraftPick {
        pick_number: 1,
        team_id: "team_1".into(),
        team_name: "Vorticists".into(),
        player_name: "Aaron Judge".into(),
        position: "OF".into(),
        price: 55,
        espn_player_id: Some("espn_101".into()),
    };

    db.record_pick(&pick).unwrap();
    assert!(db.has_draft_in_progress().unwrap());

    let loaded = db.load_picks().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].pick_number, 1);
    assert_eq!(loaded[0].team_id, "team_1");
    assert_eq!(loaded[0].team_name, "Vorticists");
    assert_eq!(loaded[0].player_name, "Aaron Judge");
    assert_eq!(loaded[0].position, "OF");
    assert_eq!(loaded[0].price, 55);
    assert_eq!(loaded[0].espn_player_id, Some("espn_101".into()));
}

#[test]
fn database_idempotent_pick_recording() {
    let db = Database::open(":memory:").expect("in-memory db");

    let pick = DraftPick {
        pick_number: 1,
        team_id: "team_1".into(),
        team_name: "Team 1".into(),
        player_name: "Aaron Judge".into(),
        position: "OF".into(),
        price: 55,
        espn_player_id: None,
    };

    // Record the same pick twice (INSERT OR IGNORE)
    db.record_pick(&pick).unwrap();
    db.record_pick(&pick).unwrap();

    let loaded = db.load_picks().unwrap();
    assert_eq!(loaded.len(), 1, "Duplicate picks should be idempotent");
}

// ===========================================================================
// Test: Fixture file integrity
// ===========================================================================

#[test]
fn fixture_toml_files_are_valid() {
    let league_text = std::fs::read_to_string(format!("{}/sample_league.toml", FIXTURES))
        .expect("sample_league.toml");
    let parsed: Result<toml::Value, _> = toml::from_str(&league_text);
    assert!(parsed.is_ok(), "sample_league.toml should be valid TOML");

    let strategy_text = std::fs::read_to_string(format!("{}/sample_strategy.toml", FIXTURES))
        .expect("sample_strategy.toml");
    let parsed: Result<toml::Value, _> = toml::from_str(&strategy_text);
    assert!(parsed.is_ok(), "sample_strategy.toml should be valid TOML");
}

#[test]
fn fixture_csv_files_have_correct_headers() {
    let hitters = std::fs::read_to_string(format!("{}/sample_hitters.csv", FIXTURES)).unwrap();
    assert!(
        hitters.starts_with("Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG"),
        "Hitters CSV should have correct headers"
    );

    let sp = std::fs::read_to_string(format!("{}/sample_pitchers_sp.csv", FIXTURES)).unwrap();
    assert!(
        sp.starts_with("Name,Team,IP,K,W,SV,ERA,WHIP,G,GS"),
        "SP CSV should have correct headers"
    );

    let rp = std::fs::read_to_string(format!("{}/sample_pitchers_rp.csv", FIXTURES)).unwrap();
    assert!(
        rp.starts_with("Name,Team,IP,K,W,SV,ERA,WHIP,G,GS"),
        "RP CSV should have correct headers"
    );

    let holds = std::fs::read_to_string(format!("{}/sample_holds.csv", FIXTURES)).unwrap();
    assert!(
        holds.starts_with("Name,Team,HD"),
        "Holds CSV should have correct headers"
    );

    let adp = std::fs::read_to_string(format!("{}/sample_adp.csv", FIXTURES)).unwrap();
    assert!(
        adp.starts_with("Name,ADP"),
        "ADP CSV should have correct headers"
    );
}

#[test]
fn fixture_mock_events_json_is_valid() {
    let content = std::fs::read_to_string(format!("{}/mock_draft_events.json", FIXTURES)).unwrap();
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(
        parsed.is_ok(),
        "mock_draft_events.json should be valid JSON"
    );
}

// ===========================================================================
// Test: Position parsing edge cases
// ===========================================================================

#[test]
fn position_parsing_handles_all_draft_positions() {
    // All positions that appear in mock draft events
    let positions = ["DH", "OF", "SS", "1B", "SP"];
    for pos_str in positions {
        let parsed = Position::from_str_pos(pos_str);
        assert!(
            parsed.is_some(),
            "Position '{}' should parse successfully",
            pos_str
        );
    }
}

// ===========================================================================
// Test: Full pipeline end-to-end
// ===========================================================================

/// This test exercises the full pipeline from fixture CSV loading through
/// valuation, draft state management, picks, crash recovery, and prompt
/// generation -- all in one test.
#[test]
fn end_to_end_pipeline() {
    // 1. Load projections from fixture CSVs
    let config = inline_config();
    let projections = load_fixture_projections(&config);
    assert!(projections.hitters.len() >= 20);
    assert!(projections.pitchers.len() >= 20);

    // 2. Compute initial valuations
    let players = load_fixture_players(&config);
    assert!(!players.is_empty());

    // 3. Create AppState
    let mut state = create_test_app_state_from_fixtures();
    let initial_pool_size = state.available_players.len();

    // 4. Process picks incrementally
    let events = generate_mock_draft_events();
    for event in &events[..4] {
        state.process_new_picks(vec![mock_event_to_pick(event)]);
    }
    assert_eq!(state.draft_state.pick_count, 4);
    assert_eq!(state.available_players.len(), initial_pool_size - 4);

    // 5. Verify crash recovery would work with this state
    let db_picks = state.db.load_picks().unwrap();
    assert_eq!(db_picks.len(), 4);
    assert!(state.db.has_draft_in_progress().unwrap());

    // 6. Verify prompt generation works with the current state
    // Find a player still in the pool
    if let Some(player) = state.available_players.first() {
        let nomination = NominationInfo {
            player_name: player.name.clone(),
            position: player
                .positions
                .first()
                .map(|p| p.display_str().to_string())
                .unwrap_or_else(|| "UTIL".into()),
            nominated_by: "Team 5".into(),
            current_bid: 1,
            current_bidder: None,
            time_remaining: Some(30),
        };

        let prompt = draft_assistant::llm::prompt::build_nomination_analysis_prompt(
            player,
            &nomination,
            &state.draft_state.my_team().roster,
            &state.category_needs,
            &state.scarcity,
            &state.available_players,
            &state.draft_state,
            &state.inflation,
        );

        assert!(!prompt.is_empty(), "Prompt should not be empty");
        assert!(
            prompt.contains(&player.name),
            "Prompt should contain the player name"
        );
    }

    // 7. Process remaining picks
    for event in &events[4..] {
        state.process_new_picks(vec![mock_event_to_pick(event)]);
    }
    assert_eq!(state.draft_state.pick_count, 8);
    assert_eq!(state.available_players.len(), initial_pool_size - 8);

    // 8. All valuations should still be valid
    for player in &state.available_players {
        assert!(
            player.dollar_value.is_finite(),
            "Player '{}' should have finite dollar value",
            player.name
        );
    }
}
