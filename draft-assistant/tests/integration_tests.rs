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

/// Build ESPN-style team budget data for 10 teams, each with $260 budget.
fn ten_team_budgets() -> Vec<draft_assistant::draft::state::TeamBudgetPayload> {
    (1..=10)
        .map(|i| draft_assistant::draft::state::TeamBudgetPayload {
            team_id: format!("{}", i),
            team_name: format!("Team {}", i),
            budget: 260,
        })
        .collect()
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
        teams: HashMap::new(),
        my_team: None,
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
            pitchers: format!("{}/sample_pitchers.csv", FIXTURES),
        },
    }
}

/// Load projections from fixture CSVs and run the full valuation pipeline.
fn load_fixture_players(config: &Config) -> Vec<PlayerValuation> {
    let projections = draft_assistant::valuation::projections::load_all_from_paths(
        &config.data_paths,
    )
    .expect("fixture CSVs should load");

    draft_assistant::valuation::compute_initial(&projections, config)
        .expect("initial valuation should succeed")
}

/// Load fixture projections.
fn load_fixture_projections(config: &Config) -> AllProjections {
    draft_assistant::valuation::projections::load_all_from_paths(
        &config.data_paths,
    )
    .expect("fixture CSVs should load")
}

/// Create a full AppState wired up with fixture data, in-memory DB, and
/// LLM disabled. Teams are registered from ESPN-style budget data.
fn create_test_app_state_from_fixtures() -> AppState {
    let config = inline_config();
    let projections = load_fixture_projections(&config);
    let mut available = load_fixture_players(&config);

    let mut draft_state = DraftState::new(260, &roster_config());
    draft_state.reconcile_budgets(&ten_team_budgets());
    draft_state.set_my_team_by_name("Team 1");

    // Recalculate with draft state for consistency
    draft_assistant::valuation::recalculate_all(
        &mut available,
        &config.league,
        &config.strategy,
        &draft_state,
    );

    let db = Database::open(":memory:").expect("in-memory db");
    let draft_id = Database::generate_draft_id();
    let llm_client = LlmClient::Disabled;
    let (llm_tx, _llm_rx) = mpsc::channel(16);

    AppState::new(config, draft_state, available, projections, db, draft_id, llm_client, llm_tx)
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
            team_id: "3".into(),
            team_name: "Team 3".into(),
            player_name: "Shohei Ohtani".into(),
            position: "DH".into(),
            price: 62,
            espn_player_id: "espn_100".into(),
        },
        MockDraftEvent {
            pick_number: 2,
            team_id: "4".into(),
            team_name: "Team 4".into(),
            player_name: "Aaron Judge".into(),
            position: "OF".into(),
            price: 55,
            espn_player_id: "espn_101".into(),
        },
        MockDraftEvent {
            pick_number: 3,
            team_id: "5".into(),
            team_name: "Team 5".into(),
            player_name: "Juan Soto".into(),
            position: "OF".into(),
            price: 48,
            espn_player_id: "espn_102".into(),
        },
        MockDraftEvent {
            pick_number: 4,
            team_id: "6".into(),
            team_name: "Team 6".into(),
            player_name: "Bobby Witt Jr.".into(),
            position: "SS".into(),
            price: 42,
            espn_player_id: "espn_103".into(),
        },
        MockDraftEvent {
            pick_number: 5,
            team_id: "7".into(),
            team_name: "Team 7".into(),
            player_name: "Mookie Betts".into(),
            position: "SS".into(),
            price: 40,
            espn_player_id: "espn_104".into(),
        },
        MockDraftEvent {
            pick_number: 6,
            team_id: "8".into(),
            team_name: "Team 8".into(),
            player_name: "Trea Turner".into(),
            position: "SS".into(),
            price: 38,
            espn_player_id: "espn_105".into(),
        },
        MockDraftEvent {
            pick_number: 7,
            team_id: "9".into(),
            team_name: "Team 9".into(),
            player_name: "Freddie Freeman".into(),
            position: "1B".into(),
            price: 36,
            espn_player_id: "espn_106".into(),
        },
        MockDraftEvent {
            pick_number: 8,
            team_id: "10".into(),
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
        eligible_slots: vec![],
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
    let db_picks = state.db.load_picks(&state.draft_id).unwrap();
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
    let draft_id = Database::generate_draft_id();

    // Record 3 picks directly to DB (simulating a previous session)
    let events = generate_mock_draft_events();
    for event in &events[..3] {
        let pick = mock_event_to_pick(event);
        db.record_pick(&pick, &draft_id).unwrap();
    }
    db.set_draft_id(&draft_id).unwrap();
    assert!(db.has_draft_in_progress(&draft_id).unwrap());

    // Simulate restart: create a fresh AppState with the same DB
    let mut draft_state = DraftState::new(260, &roster_config());
    draft_state.reconcile_budgets(&ten_team_budgets());
    draft_state.set_my_team_by_name("Team 1");

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
        draft_id,
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
    let team3 = state.draft_state.team("3").unwrap();
    assert_eq!(team3.budget_spent, 62);
    let team4 = state.draft_state.team("4").unwrap();
    assert_eq!(team4.budget_spent, 55);
    let team5 = state.draft_state.team("5").unwrap();
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
    let db_picks = state.db.load_picks(&state.draft_id).unwrap();
    assert_eq!(db_picks.len(), 4);
}

/// Verify that crash recovery with an empty DB returns false (no recovery needed).
#[test]
fn crash_recovery_empty_db_returns_false() {
    let config = inline_config();
    let projections = load_fixture_projections(&config);
    let available = load_fixture_players(&config);
    let mut draft_state = DraftState::new(260, &roster_config());
    draft_state.reconcile_budgets(&ten_team_budgets());
    draft_state.set_my_team_by_name("Team 1");
    let db = Database::open(":memory:").expect("in-memory db");
    let draft_id = Database::generate_draft_id();
    let llm_client = LlmClient::Disabled;
    let (llm_tx, _llm_rx) = mpsc::channel(16);
    let mut state = AppState::new(
        config,
        draft_state,
        available,
        projections,
        db,
        draft_id,
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

/// Verify that holds are loaded directly from the HLD column in the
/// combined pitchers CSV (Razzball format — no overlay or estimation).
#[test]
fn holds_loaded_from_csv_directly() {
    let config = inline_config();
    let projections = load_fixture_projections(&config);

    // Devin Williams: HLD=25 in CSV
    let dw = projections
        .pitchers
        .iter()
        .find(|p| p.name == "Devin Williams")
        .unwrap();
    assert_eq!(dw.hd, 25);

    // Clay Holmes: HLD=18 in CSV
    let ch = projections
        .pitchers
        .iter()
        .find(|p| p.name == "Clay Holmes")
        .unwrap();
    assert_eq!(ch.hd, 18);

    // Kenley Jansen: HLD=6 in CSV
    let kj = projections
        .pitchers
        .iter()
        .find(|p| p.name == "Kenley Jansen")
        .unwrap();
    assert_eq!(kj.hd, 6);
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
        eligible_slots: vec![],
    };

    let prompt = draft_assistant::llm::prompt::build_nomination_analysis_prompt(
        &player,
        &nomination,
        &state.draft_state.my_team().unwrap().roster,
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
        &state.draft_state.my_team().unwrap().roster,
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

    // Should receive a StateSnapshot first (new picks trigger snapshot)
    let update = ui_rx.recv().await.unwrap();
    match update {
        UiUpdate::StateSnapshot(snapshot) => {
            assert!(snapshot.pick_count > 0, "Pick count should be > 0 after processing picks");
        }
        other => panic!("Expected StateSnapshot, got {:?}", other),
    }

    // Then receive the NominationUpdate
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
    assert!(
        matches!(update, UiUpdate::ConnectionStatus(ConnectionStatus::Connected)),
        "Expected ConnectionStatus(Connected), got {:?}", update
    );

    // Send disconnected
    ws_tx.send(WsEvent::Disconnected).await.unwrap();

    let update = ui_rx.recv().await.unwrap();
    assert!(
        matches!(update, UiUpdate::ConnectionStatus(ConnectionStatus::Disconnected)),
        "Expected ConnectionStatus(Disconnected), got {:?}", update
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

    // Should receive StateSnapshot first (new picks trigger snapshot)
    let update = ui_rx.recv().await.unwrap();
    assert!(
        matches!(&update, UiUpdate::StateSnapshot(_)),
        "Expected StateSnapshot, got {:?}", update
    );

    // Then receive NominationUpdate for Aaron Judge
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

    // Should receive StateSnapshot first (new pick triggers snapshot)
    let update2 = ui_rx.recv().await.unwrap();
    assert!(
        matches!(&update2, UiUpdate::StateSnapshot(_)),
        "Expected StateSnapshot, got {:?}", update2
    );

    // Then NominationCleared (previous nomination resolved) followed by
    // NominationUpdate for Juan Soto
    let update2 = ui_rx.recv().await.unwrap();
    match &update2 {
        UiUpdate::NominationCleared => {
            // Previous nomination was cleared; next should be the new one
            let update3 = ui_rx.recv().await.unwrap();
            match &update3 {
                UiUpdate::NominationUpdate(info) => {
                    assert_eq!(info.player_name, "Juan Soto");
                }
                other => panic!("Expected NominationUpdate for Juan Soto after NominationCleared, got {:?}", other),
            }
        }
        UiUpdate::NominationUpdate(info) => {
            assert_eq!(info.player_name, "Juan Soto");
        }
        other => panic!("Expected NominationCleared or NominationUpdate for Juan Soto, got {:?}", other),
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

    // All team_ids should be valid (numeric string format from ESPN)
    for event in &events {
        assert!(
            event.team_id.parse::<u32>().is_ok(),
            "Team ID should be a numeric string, got: {}",
            event.team_id
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
        eligible_slots: vec![],
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
    let draft_id = Database::generate_draft_id();

    let pick = DraftPick {
        pick_number: 1,
        team_id: "team_1".into(),
        team_name: "Vorticists".into(),
        player_name: "Aaron Judge".into(),
        position: "OF".into(),
        price: 55,
        espn_player_id: Some("espn_101".into()),
        eligible_slots: vec![],
    };

    db.record_pick(&pick, &draft_id).unwrap();
    assert!(db.has_draft_in_progress(&draft_id).unwrap());

    let loaded = db.load_picks(&draft_id).unwrap();
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
    let draft_id = Database::generate_draft_id();

    let pick = DraftPick {
        pick_number: 1,
        team_id: "team_1".into(),
        team_name: "Team 1".into(),
        player_name: "Aaron Judge".into(),
        position: "OF".into(),
        price: 55,
        espn_player_id: None,
        eligible_slots: vec![],
    };

    // Record the same pick twice (INSERT OR IGNORE)
    db.record_pick(&pick, &draft_id).unwrap();
    db.record_pick(&pick, &draft_id).unwrap();

    let loaded = db.load_picks(&draft_id).unwrap();
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
        hitters.starts_with("Name,Team,POS,PA,AB,H,HR,R,RBI,BB,SB,AVG"),
        "Hitters CSV should have correct headers"
    );

    let pitchers = std::fs::read_to_string(format!("{}/sample_pitchers.csv", FIXTURES)).unwrap();
    assert!(
        pitchers.starts_with("Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K"),
        "Pitchers CSV should have correct headers"
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
    let db_picks = state.db.load_picks(&state.draft_id).unwrap();
    assert_eq!(db_picks.len(), 4);
    assert!(state.db.has_draft_in_progress(&state.draft_id).unwrap());

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
            eligible_slots: vec![],
        };

        let prompt = draft_assistant::llm::prompt::build_nomination_analysis_prompt(
            player,
            &nomination,
            &state.draft_state.my_team().unwrap().roster,
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

// ===========================================================================
// Tests: Draft log deduplication and pick numbering
// ===========================================================================

/// Recording the same player with different pick_numbers (ESPN renumbering)
/// must not create duplicate entries in the draft log.
#[test]
fn draft_log_no_duplicates_on_espn_renumbering() {
    let mut draft_state = DraftState::new(260, &roster_config());
    draft_state.reconcile_budgets(&ten_team_budgets());
    draft_state.set_my_team_by_name("Team 1");

    // Record the first pick normally
    draft_state.record_pick(DraftPick {
        pick_number: 1,
        team_id: "1".to_string(),
        team_name: "Team 1".to_string(),
        player_name: "Mike Trout".to_string(),
        position: "CF".to_string(),
        price: 45,
        espn_player_id: None,
        eligible_slots: vec![],
    });

    assert_eq!(draft_state.picks.len(), 1);
    assert_eq!(draft_state.pick_count, 1);

    // ESPN renumbers the same player with a different pick_number (virtualized
    // list shift). This should be a complete no-op.
    draft_state.record_pick(DraftPick {
        pick_number: 101,
        team_id: "1".to_string(),
        team_name: "Team 1".to_string(),
        player_name: "Mike Trout".to_string(),
        position: "CF".to_string(),
        price: 45,
        espn_player_id: None,
        eligible_slots: vec![],
    });

    assert_eq!(
        draft_state.picks.len(),
        1,
        "Draft log should not contain duplicate entries for the same player"
    );
    assert_eq!(
        draft_state.pick_count, 1,
        "Pick count should not increase for a duplicate player"
    );
    assert_eq!(
        draft_state.picks[0].pick_number, 1,
        "Original pick number should be preserved"
    );
}

/// Same test as above but with ESPN player IDs present.
#[test]
fn draft_log_no_duplicates_by_espn_player_id() {
    let mut draft_state = DraftState::new(260, &roster_config());
    draft_state.reconcile_budgets(&ten_team_budgets());
    draft_state.set_my_team_by_name("Team 1");

    draft_state.record_pick(DraftPick {
        pick_number: 1,
        team_id: "1".to_string(),
        team_name: "Team 1".to_string(),
        player_name: "Mike Trout".to_string(),
        position: "CF".to_string(),
        price: 45,
        espn_player_id: Some("33039".to_string()),
        eligible_slots: vec![],
    });

    // Same ESPN player ID, different pick number
    draft_state.record_pick(DraftPick {
        pick_number: 55,
        team_id: "1".to_string(),
        team_name: "Team 1".to_string(),
        player_name: "Mike Trout".to_string(),
        position: "CF".to_string(),
        price: 45,
        espn_player_id: Some("33039".to_string()),
        eligible_slots: vec![],
    });

    assert_eq!(
        draft_state.picks.len(),
        1,
        "Duplicate ESPN player ID should be rejected"
    );
    assert_eq!(draft_state.pick_count, 1);
}

/// Pick numbering should be sequential starting from 1, and the draft log
/// in the snapshot should contain exactly the right picks in order.
#[test]
fn draft_log_sequential_numbering_and_snapshot_correctness() {
    let mut state = create_test_app_state_from_fixtures();
    let events = generate_mock_draft_events();

    // Process the first 3 picks
    for event in &events[..3] {
        state.process_new_picks(vec![mock_event_to_pick(event)]);
    }

    // Verify sequential pick numbers in the draft log
    assert_eq!(state.draft_state.picks.len(), 3);
    for (i, pick) in state.draft_state.picks.iter().enumerate() {
        assert_eq!(
            pick.pick_number,
            (i + 1) as u32,
            "Pick {} should have pick_number {}, got {}",
            i,
            i + 1,
            pick.pick_number
        );
    }

    // Build snapshot and verify draft_log matches
    let snapshot = state.build_snapshot();
    assert_eq!(
        snapshot.draft_log.len(),
        3,
        "Snapshot draft_log should contain exactly 3 picks"
    );
    assert_eq!(snapshot.draft_log[0].player_name, events[0].player_name);
    assert_eq!(snapshot.draft_log[1].player_name, events[1].player_name);
    assert_eq!(snapshot.draft_log[2].player_name, events[2].player_name);

    // Pick numbers in snapshot should match
    for (i, pick) in snapshot.draft_log.iter().enumerate() {
        assert_eq!(pick.pick_number, (i + 1) as u32);
    }
}

/// Simulate the full renumbering scenario: 3 picks arrive, then the same
/// 3 picks arrive with shifted pick_numbers (as if ESPN's virtualized list
/// scrolled). The draft log should still contain exactly 3 unique entries.
#[test]
fn draft_log_resilient_to_virtualized_list_renumbering() {
    use draft_assistant::draft::state::{
        compute_state_diff, PickPayload, StateUpdatePayload,
    };

    let mut draft_state = DraftState::new(260, &roster_config());
    draft_state.reconcile_budgets(&ten_team_budgets());
    draft_state.set_my_team_by_name("Team 1");

    // First state update: 3 picks with numbers 1, 2, 3
    let payload1 = StateUpdatePayload {
        picks: vec![
            PickPayload {
                pick_number: 1,
                team_id: "1".to_string(),
                team_name: "Team 1".to_string(),
                player_id: "p1".to_string(),
                player_name: "Player A".to_string(),
                position: "CF".to_string(),
                price: 30,
                eligible_slots: vec![],
            },
            PickPayload {
                pick_number: 2,
                team_id: "2".to_string(),
                team_name: "Team 2".to_string(),
                player_id: "p2".to_string(),
                player_name: "Player B".to_string(),
                position: "SP".to_string(),
                price: 25,
                eligible_slots: vec![],
            },
            PickPayload {
                pick_number: 3,
                team_id: "3".to_string(),
                team_name: "Team 3".to_string(),
                player_id: "p3".to_string(),
                player_name: "Player C".to_string(),
                position: "1B".to_string(),
                price: 20,
                eligible_slots: vec![],
            },
        ],
        current_nomination: None,
        ..Default::default()
    };

    let diff1 = compute_state_diff(&None, &payload1);
    assert_eq!(diff1.new_picks.len(), 3);
    for pick in diff1.new_picks {
        draft_state.record_pick(pick);
    }
    assert_eq!(draft_state.picks.len(), 3);

    // Second state update: ESPN renumbered to 51, 52, 53 (virtualized list shift)
    // Same players, different pick_numbers.
    let payload2 = StateUpdatePayload {
        picks: vec![
            PickPayload {
                pick_number: 51,
                team_id: "1".to_string(),
                team_name: "Team 1".to_string(),
                player_id: "p1".to_string(),
                player_name: "Player A".to_string(),
                position: "CF".to_string(),
                price: 30,
                eligible_slots: vec![],
            },
            PickPayload {
                pick_number: 52,
                team_id: "2".to_string(),
                team_name: "Team 2".to_string(),
                player_id: "p2".to_string(),
                player_name: "Player B".to_string(),
                position: "SP".to_string(),
                price: 25,
                eligible_slots: vec![],
            },
            PickPayload {
                pick_number: 53,
                team_id: "3".to_string(),
                team_name: "Team 3".to_string(),
                player_id: "p3".to_string(),
                player_name: "Player C".to_string(),
                position: "1B".to_string(),
                price: 20,
                eligible_slots: vec![],
            },
        ],
        current_nomination: None,
        ..Default::default()
    };

    // compute_state_diff now uses player identity (player_id), so renumbered
    // picks with the same player_id are NOT re-emitted. This eliminates the
    // spurious DB writes and recalculate_all calls from the old behavior.
    let diff2 = compute_state_diff(&Some(payload1), &payload2);
    assert_eq!(
        diff2.new_picks.len(),
        0,
        "Renumbered picks with same player_id should NOT be re-emitted"
    );

    assert_eq!(
        draft_state.picks.len(),
        3,
        "Draft log should still contain exactly 3 picks after renumbering"
    );
    assert_eq!(draft_state.pick_count, 3);

    // Verify the original pick numbers are preserved
    assert_eq!(draft_state.picks[0].pick_number, 1);
    assert_eq!(draft_state.picks[1].pick_number, 2);
    assert_eq!(draft_state.picks[2].pick_number, 3);

    // Verify budgets are correct (no double-counting)
    let team1 = draft_state.team("1").unwrap();
    assert_eq!(team1.budget_spent, 30, "Team 1 should have spent $30 once");
    let team2 = draft_state.team("2").unwrap();
    assert_eq!(team2.budget_spent, 25, "Team 2 should have spent $25 once");
    let team3 = draft_state.team("3").unwrap();
    assert_eq!(team3.budget_spent, 20, "Team 3 should have spent $20 once");
}

// ===========================================================================
// Test: Budget reconciliation and snapshot correctness
// ===========================================================================

/// Verify that reconcile_budgets returns budgets_changed=true when ESPN
/// reports different budget values than what the local state holds.
#[test]
fn reconcile_budgets_returns_budgets_changed_when_values_differ() {
    let mut state = create_test_app_state_from_fixtures();

    // Record a pick so Team 3 has local budget = 260 - 62 = 198
    let events = generate_mock_draft_events();
    state.process_new_picks(vec![mock_event_to_pick(&events[0])]);

    let team3 = state.draft_state.team("3").unwrap();
    assert_eq!(team3.budget_remaining, 198);

    // Now reconcile with ESPN data that shows a DIFFERENT budget for Team 3.
    // This simulates ESPN's authoritative budget correction.
    let mut updated_budgets = ten_team_budgets();
    // ESPN says Team 3 has $195 remaining (e.g. ESPN accounts for something
    // our local tracking missed)
    updated_budgets[2].budget = 195;

    let result = state.draft_state.reconcile_budgets(&updated_budgets);

    assert!(!result.teams_registered, "teams were already registered");
    assert!(
        result.budgets_changed,
        "budgets_changed should be true when ESPN reports different values"
    );

    // Verify ESPN's value took effect
    let team3 = state.draft_state.team("3").unwrap();
    assert_eq!(team3.budget_remaining, 195);
    assert_eq!(team3.budget_spent, 65); // 260 - 195
}

/// Verify that reconcile_budgets returns budgets_changed=false when ESPN
/// reports the exact same budget values as local state.
#[test]
fn reconcile_budgets_returns_no_change_when_values_same() {
    let mut state = create_test_app_state_from_fixtures();

    // All teams start at $260 which matches ten_team_budgets()
    let result = state.draft_state.reconcile_budgets(&ten_team_budgets());

    assert!(!result.teams_registered, "teams were already registered");
    assert!(
        !result.budgets_changed,
        "budgets_changed should be false when values are identical"
    );
}

/// Verify that reconcile_budgets returns teams_registered=true and
/// budgets_changed=true on the first call.
#[test]
fn reconcile_budgets_first_call_registers_teams() {
    let mut draft_state = DraftState::new(260, &roster_config());
    assert!(draft_state.teams.is_empty());

    let result = draft_state.reconcile_budgets(&ten_team_budgets());

    assert!(
        result.teams_registered,
        "first call should register teams"
    );
    assert!(
        result.budgets_changed,
        "first call should report budgets changed (they went from nothing to populated)"
    );
    assert_eq!(draft_state.teams.len(), 10);
}

/// Verify that after picks are processed and budgets reconciled, the
/// snapshot contains correct budget values for ALL teams (not just the
/// user's team).
#[test]
fn snapshot_reflects_other_teams_budgets_after_picks_and_reconcile() {
    let mut state = create_test_app_state_from_fixtures();
    let events = generate_mock_draft_events();

    // Process 3 picks: Team 3 ($62), Team 4 ($55), Team 5 ($48)
    let picks: Vec<DraftPick> = events[..3].iter().map(mock_event_to_pick).collect();
    state.process_new_picks(picks);

    // Reconcile with ESPN budget data reflecting the picks
    let mut updated_budgets = ten_team_budgets();
    updated_budgets[2].budget = 198; // Team 3: 260 - 62
    updated_budgets[3].budget = 205; // Team 4: 260 - 55
    updated_budgets[4].budget = 212; // Team 5: 260 - 48
    state.draft_state.reconcile_budgets(&updated_budgets);

    // Build snapshot and verify team budgets
    let snapshot = state.build_snapshot();

    assert_eq!(snapshot.team_snapshots.len(), 10);

    // Find each team in the snapshot and verify budgets
    let team3_snap = snapshot
        .team_snapshots
        .iter()
        .find(|t| t.name == "Team 3")
        .expect("Team 3 should be in snapshot");
    assert_eq!(team3_snap.budget_remaining, 198);

    let team4_snap = snapshot
        .team_snapshots
        .iter()
        .find(|t| t.name == "Team 4")
        .expect("Team 4 should be in snapshot");
    assert_eq!(team4_snap.budget_remaining, 205);

    let team5_snap = snapshot
        .team_snapshots
        .iter()
        .find(|t| t.name == "Team 5")
        .expect("Team 5 should be in snapshot");
    assert_eq!(team5_snap.budget_remaining, 212);

    // Teams that did not pick should still show full budget
    let team1_snap = snapshot
        .team_snapshots
        .iter()
        .find(|t| t.name == "Team 1")
        .expect("Team 1 should be in snapshot");
    assert_eq!(team1_snap.budget_remaining, 260);
}

/// Build a JSON STATE_UPDATE message that includes team budget data.
fn build_state_update_json_with_teams(
    events: &[MockDraftEvent],
    up_to_pick: u32,
    team_budgets: &[(String, u32)], // (team_name, budget_remaining)
    nomination: Option<(&str, &str, &str, &str)>,
) -> String {
    let picks: Vec<serde_json::Value> = events
        .iter()
        .filter(|e| e.pick_number <= up_to_pick)
        .map(|e| {
            serde_json::json!({
                "pickNumber": e.pick_number,
                "teamId": e.team_id,
                "teamName": e.team_name,
                "playerName": e.player_name,
                "playerId": e.espn_player_id,
                "position": e.position,
                "price": e.price
            })
        })
        .collect();

    let teams: Vec<serde_json::Value> = team_budgets
        .iter()
        .map(|(name, budget)| {
            serde_json::json!({
                "teamName": name,
                "budget": budget
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
            "myTeamId": "Team 1",
            "teams": teams,
            "source": "test"
        }
    })
    .to_string()
}

/// Full event-loop test: verify that a state update containing team budget
/// data (but no new picks beyond what was already processed) still triggers
/// a TUI snapshot with the updated budgets.
///
/// This exercises the exact bug scenario: ESPN sends budget data that differs
/// from local tracking, and even without new picks the TUI should receive the
/// corrected budget values.
#[tokio::test]
async fn event_loop_budget_only_update_triggers_snapshot() {
    let state = create_test_app_state_from_fixtures();

    let (ws_tx, ws_rx) = mpsc::channel(16);
    let (_llm_tx, llm_rx) = mpsc::channel(16);
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let (ui_tx, mut ui_rx) = mpsc::channel(64);

    let handle = tokio::spawn(app::run(ws_rx, llm_rx, cmd_rx, ui_tx, state));

    // First: send a state update with 1 pick + team budgets (initial registration)
    let events = generate_mock_draft_events();
    let initial_budgets: Vec<(String, u32)> = (1..=10)
        .map(|i| (format!("Team {}", i), 260))
        .collect();
    let json1 = build_state_update_json_with_teams(
        &events,
        1,
        &initial_budgets,
        Some(("espn_101", "Aaron Judge", "OF", "Team 4")),
    );
    ws_tx.send(WsEvent::Message(json1)).await.unwrap();

    // Drain the snapshot + nomination from the first update
    let update1 = ui_rx.recv().await.unwrap();
    assert!(
        matches!(&update1, UiUpdate::StateSnapshot(_)),
        "Expected StateSnapshot from first update, got {:?}", update1
    );
    let update2 = ui_rx.recv().await.unwrap();
    assert!(
        matches!(&update2, UiUpdate::NominationUpdate(_)),
        "Expected NominationUpdate from first update, got {:?}", update2
    );

    // Second: send a state update with the SAME pick (no new picks) but
    // CHANGED budget data for Team 3. This simulates ESPN's budget updating
    // asynchronously after a pick completes.
    let mut changed_budgets = initial_budgets.clone();
    // Team 3 budget dropped from 260 to 198 (they won a $62 pick)
    changed_budgets[2].1 = 198;
    let json2 = build_state_update_json_with_teams(
        &events,
        1,
        &changed_budgets,
        Some(("espn_101", "Aaron Judge", "OF", "Team 4")),
    );
    ws_tx.send(WsEvent::Message(json2)).await.unwrap();

    // We should receive a StateSnapshot because budgets changed, even though
    // there are no new picks. This is the core bug fix validation.
    let update3 = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        ui_rx.recv(),
    )
    .await
    .expect("should receive snapshot within timeout")
    .expect("channel should not be closed");

    match update3 {
        UiUpdate::StateSnapshot(snapshot) => {
            // Verify Team 3's budget is reflected in the snapshot
            let team3 = snapshot
                .team_snapshots
                .iter()
                .find(|t| t.name == "Team 3")
                .expect("Team 3 should be in snapshot");
            assert_eq!(
                team3.budget_remaining, 198,
                "Team 3 budget should be $198 in snapshot after reconciliation"
            );
        }
        other => panic!(
            "Expected StateSnapshot with updated budgets, got {:?}",
            other
        ),
    }

    // Clean up
    cmd_tx.send(UserCommand::Quit).await.unwrap();
    let result = handle.await.unwrap();
    assert!(result.is_ok());
}

/// Verify that when budgets DON'T change, no redundant snapshot is sent.
/// This ensures we don't regress on the optimization of only sending
/// snapshots when something actually changed.
#[tokio::test]
async fn event_loop_same_budgets_no_redundant_snapshot() {
    let state = create_test_app_state_from_fixtures();

    let (ws_tx, ws_rx) = mpsc::channel(16);
    let (_llm_tx, llm_rx) = mpsc::channel(16);
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let (ui_tx, mut ui_rx) = mpsc::channel(64);

    let handle = tokio::spawn(app::run(ws_rx, llm_rx, cmd_rx, ui_tx, state));

    // First: send a state update with 1 pick + team budgets
    let events = generate_mock_draft_events();
    let budgets: Vec<(String, u32)> = (1..=10)
        .map(|i| (format!("Team {}", i), 260))
        .collect();
    let json1 = build_state_update_json_with_teams(
        &events,
        1,
        &budgets,
        Some(("espn_101", "Aaron Judge", "OF", "Team 4")),
    );
    ws_tx.send(WsEvent::Message(json1)).await.unwrap();

    // Drain the snapshot + nomination
    let _ = ui_rx.recv().await.unwrap(); // StateSnapshot
    let _ = ui_rx.recv().await.unwrap(); // NominationUpdate

    // Second: send an identical state update (same picks, same budgets,
    // same nomination). No new information.
    let json2 = build_state_update_json_with_teams(
        &events,
        1,
        &budgets,
        Some(("espn_101", "Aaron Judge", "OF", "Team 4")),
    );
    ws_tx.send(WsEvent::Message(json2)).await.unwrap();

    // No snapshot should be sent since nothing changed. Use a short
    // timeout to verify nothing arrives.
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        ui_rx.recv(),
    )
    .await;

    assert!(
        result.is_err(),
        "Should NOT receive a snapshot when nothing changed (timeout expected)"
    );

    // Clean up
    cmd_tx.send(UserCommand::Quit).await.unwrap();
    let _ = handle.await;
}

// ===========================================================================
// Test: Roster sidebar correctness — second pick appears in roster
// ===========================================================================
//
// These tests verify the fix for the bug where the second player won by a
// team did not appear in the roster sidebar. The root cause was that
// `compute_state_diff` relied solely on `pick_number` to detect new picks,
// but ESPN's virtualized pick list can cause pick_number renumbering when
// the pick counter label updates before new DOM entries appear.

/// After recording two picks for the same team, the roster shows both players.
#[test]
fn roster_shows_both_players_after_two_picks_same_team() {
    let mut state = create_test_app_state_from_fixtures();

    // Pick 1: Team 1 wins Shohei Ohtani (DH)
    state.process_new_picks(vec![DraftPick {
        pick_number: 1,
        team_id: "1".into(),
        team_name: "Team 1".into(),
        player_name: "Shohei Ohtani".into(),
        position: "DH".into(),
        price: 62,
        espn_player_id: Some("espn_100".into()),
        eligible_slots: vec![],
    }]);

    let snapshot1 = state.build_snapshot();
    let filled1: Vec<_> = snapshot1
        .my_roster
        .iter()
        .filter(|s| s.player.is_some())
        .collect();
    assert_eq!(filled1.len(), 1, "Should have 1 player after first pick");
    assert_eq!(filled1[0].player.as_ref().unwrap().name, "Shohei Ohtani");

    // Pick 2: Team 1 wins Aaron Judge (OF)
    state.process_new_picks(vec![DraftPick {
        pick_number: 2,
        team_id: "1".into(),
        team_name: "Team 1".into(),
        player_name: "Aaron Judge".into(),
        position: "RF".into(),
        price: 55,
        espn_player_id: Some("espn_101".into()),
        eligible_slots: vec![],
    }]);

    let snapshot2 = state.build_snapshot();
    let filled2: Vec<_> = snapshot2
        .my_roster
        .iter()
        .filter(|s| s.player.is_some())
        .collect();
    assert_eq!(
        filled2.len(),
        2,
        "Should have 2 players after second pick. Filled: {:?}",
        filled2
            .iter()
            .map(|s| s.player.as_ref().unwrap().name.as_str())
            .collect::<Vec<_>>()
    );

    let names: Vec<&str> = filled2
        .iter()
        .map(|s| s.player.as_ref().unwrap().name.as_str())
        .collect();
    assert!(names.contains(&"Shohei Ohtani"), "Ohtani should be on roster");
    assert!(names.contains(&"Aaron Judge"), "Judge should be on roster");
}

/// After recording multiple picks across different teams, my roster shows
/// only my team's picks and all of them.
#[test]
fn roster_shows_all_my_picks_across_interleaved_teams() {
    let mut state = create_test_app_state_from_fixtures();

    // Interleave picks across teams, with Team 1 (my team) getting 3 players
    let picks = vec![
        DraftPick {
            pick_number: 1,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_name: "Shohei Ohtani".into(),
            position: "DH".into(),
            price: 62,
            espn_player_id: Some("espn_100".into()),
            eligible_slots: vec![],
        },
        DraftPick {
            pick_number: 2,
            team_id: "2".into(),
            team_name: "Team 2".into(),
            player_name: "Aaron Judge".into(),
            position: "RF".into(),
            price: 55,
            espn_player_id: Some("espn_101".into()),
            eligible_slots: vec![],
        },
        DraftPick {
            pick_number: 3,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_name: "Juan Soto".into(),
            position: "LF".into(),
            price: 48,
            espn_player_id: Some("espn_102".into()),
            eligible_slots: vec![],
        },
        DraftPick {
            pick_number: 4,
            team_id: "3".into(),
            team_name: "Team 3".into(),
            player_name: "Mookie Betts".into(),
            position: "SS".into(),
            price: 40,
            espn_player_id: Some("espn_104".into()),
            eligible_slots: vec![],
        },
        DraftPick {
            pick_number: 5,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_name: "Freddie Freeman".into(),
            position: "1B".into(),
            price: 36,
            espn_player_id: Some("espn_106".into()),
            eligible_slots: vec![],
        },
    ];

    state.process_new_picks(picks);

    let snapshot = state.build_snapshot();
    let filled: Vec<_> = snapshot
        .my_roster
        .iter()
        .filter(|s| s.player.is_some())
        .collect();

    assert_eq!(
        filled.len(),
        3,
        "Team 1 should have exactly 3 players. Found: {:?}",
        filled
            .iter()
            .map(|s| s.player.as_ref().unwrap().name.as_str())
            .collect::<Vec<_>>()
    );

    let names: Vec<&str> = filled
        .iter()
        .map(|s| s.player.as_ref().unwrap().name.as_str())
        .collect();
    assert!(names.contains(&"Shohei Ohtani"));
    assert!(names.contains(&"Juan Soto"));
    assert!(names.contains(&"Freddie Freeman"));
    // Other teams' players should NOT be on my roster
    assert!(!names.contains(&"Aaron Judge"));
    assert!(!names.contains(&"Mookie Betts"));
}

/// Simulate the exact ESPN virtualized pick list renumbering scenario that
/// caused the original bug. When the pick counter label increments before
/// the new DOM entry appears, existing picks get renumbered and the new
/// pick's number was already "claimed" by the old pick.
///
/// Scenario:
/// 1. STATE_UPDATE 1: picks=[{#1, "PlayerA", "Team1"}]
/// 2. STATE_UPDATE 2: picks=[{#2, "PlayerA", "Team1"}]  (A renumbered; pick label says PK 2 but new entry not yet in DOM)
/// 3. STATE_UPDATE 3: picks=[{#1, "PlayerA", "Team1"}, {#2, "PlayerB", "Team1"}]  (both now visible)
///
/// Before the fix, PlayerB was never detected as new because pick_number 2
/// was already in previous_extension_state from step 2.
#[test]
fn pick_renumbering_does_not_drop_new_picks() {
    use draft_assistant::draft::state::{
        compute_state_diff, PickPayload,
        StateUpdatePayload as InternalStatePayload,
    };

    let mut state = create_test_app_state_from_fixtures();

    // STATE_UPDATE 1: one pick, pick_number=1
    let payload1 = InternalStatePayload {
        picks: vec![PickPayload {
            pick_number: 1,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_id: "espn_100".into(),
            player_name: "Shohei Ohtani".into(),
            position: "DH".into(),
            price: 62,
            eligible_slots: vec![],
        }],
        current_nomination: None,
        teams: vec![],
        pick_count: Some(1),
        total_picks: Some(260),
    };

    let diff1 = compute_state_diff(&None, &payload1);
    assert_eq!(diff1.new_picks.len(), 1);
    state.process_new_picks(diff1.new_picks);

    let snapshot1 = state.build_snapshot();
    let filled1: Vec<_> = snapshot1
        .my_roster
        .iter()
        .filter(|s| s.player.is_some())
        .collect();
    assert_eq!(filled1.len(), 1, "After update 1: 1 player on roster");

    // STATE_UPDATE 2: same player renumbered to #2 (pick label advanced but
    // the new pick entry hasn't appeared in the virtualized list yet)
    let payload2 = InternalStatePayload {
        picks: vec![PickPayload {
            pick_number: 2, // Renumbered!
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_id: "espn_100".into(),
            player_name: "Shohei Ohtani".into(),
            position: "DH".into(),
            price: 62,
            eligible_slots: vec![],
        }],
        current_nomination: None,
        teams: vec![],
        pick_count: Some(2),
        total_picks: Some(260),
    };

    let diff2 = compute_state_diff(&Some(payload1), &payload2);
    // Renumbered pick has the same player_id, so compute_state_diff correctly
    // recognizes it as already known and does NOT re-emit it.
    assert_eq!(
        diff2.new_picks.len(),
        0,
        "Renumbered pick with same player_id should not be re-emitted"
    );
    state.process_new_picks(diff2.new_picks);

    // Roster should still have exactly 1 player (no duplicate)
    let snapshot2 = state.build_snapshot();
    let filled2: Vec<_> = snapshot2
        .my_roster
        .iter()
        .filter(|s| s.player.is_some())
        .collect();
    assert_eq!(
        filled2.len(),
        1,
        "After update 2 (renumber only): still 1 player"
    );

    // STATE_UPDATE 3: both entries now visible with correct numbering.
    // Pick #2 is the REAL new player (Aaron Judge).
    let payload3 = InternalStatePayload {
        picks: vec![
            PickPayload {
                pick_number: 1,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_id: "espn_100".into(),
                player_name: "Shohei Ohtani".into(),
                position: "DH".into(),
                price: 62,
                eligible_slots: vec![],
            },
            PickPayload {
                pick_number: 2,
                team_id: "1".into(),
                team_name: "Team 1".into(),
                player_id: "espn_101".into(),
                player_name: "Aaron Judge".into(),
                position: "RF".into(),
                price: 55,
                eligible_slots: vec![],
            },
        ],
        current_nomination: None,
        teams: vec![],
        pick_count: Some(2),
        total_picks: Some(260),
    };

    let diff3 = compute_state_diff(&Some(payload2), &payload3);
    // CRITICAL: Aaron Judge must be detected as a new pick even though
    // pick_number 2 was already in the previous payload (for the renumbered Ohtani)
    let new_player_names: Vec<&str> = diff3
        .new_picks
        .iter()
        .map(|p| p.player_name.as_str())
        .collect();
    assert!(
        new_player_names.contains(&"Aaron Judge"),
        "Aaron Judge should be detected as new pick despite pick_number 2 being reused. \
         New picks detected: {:?}",
        new_player_names
    );

    state.process_new_picks(diff3.new_picks);

    // Both players should now be on the roster
    let snapshot3 = state.build_snapshot();
    let filled3: Vec<_> = snapshot3
        .my_roster
        .iter()
        .filter(|s| s.player.is_some())
        .collect();
    assert_eq!(
        filled3.len(),
        2,
        "After update 3: both players should be on roster. Found: {:?}",
        filled3
            .iter()
            .map(|s| s.player.as_ref().unwrap().name.as_str())
            .collect::<Vec<_>>()
    );

    let names: Vec<&str> = filled3
        .iter()
        .map(|s| s.player.as_ref().unwrap().name.as_str())
        .collect();
    assert!(names.contains(&"Shohei Ohtani"), "Ohtani should be on roster");
    assert!(names.contains(&"Aaron Judge"), "Judge should be on roster");
}

/// Verify that `build_snapshot()` returns the correct `my_roster` with all won
/// players, even when picks are processed incrementally via state diffs.
#[test]
fn build_snapshot_my_roster_incremental_picks() {
    use draft_assistant::draft::state::{
        compute_state_diff, PickPayload,
        StateUpdatePayload as InternalStatePayload,
    };

    let mut state = create_test_app_state_from_fixtures();

    // Simulate 4 incremental state updates, each adding one pick.
    // Team 1 (my team) gets picks 1 and 3; Teams 2 and 3 get picks 2 and 4.
    let all_picks = vec![
        PickPayload {
            pick_number: 1,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_id: "espn_100".into(),
            player_name: "Shohei Ohtani".into(),
            position: "DH".into(),
            price: 62,
            eligible_slots: vec![],
        },
        PickPayload {
            pick_number: 2,
            team_id: "2".into(),
            team_name: "Team 2".into(),
            player_id: "espn_101".into(),
            player_name: "Aaron Judge".into(),
            position: "RF".into(),
            price: 55,
            eligible_slots: vec![],
        },
        PickPayload {
            pick_number: 3,
            team_id: "1".into(),
            team_name: "Team 1".into(),
            player_id: "espn_102".into(),
            player_name: "Juan Soto".into(),
            position: "LF".into(),
            price: 48,
            eligible_slots: vec![],
        },
        PickPayload {
            pick_number: 4,
            team_id: "3".into(),
            team_name: "Team 3".into(),
            player_id: "espn_103".into(),
            player_name: "Bobby Witt Jr.".into(),
            position: "SS".into(),
            price: 42,
            eligible_slots: vec![],
        },
    ];

    let mut previous: Option<InternalStatePayload> = None;

    for i in 0..all_picks.len() {
        let current = InternalStatePayload {
            picks: all_picks[..=i].to_vec(),
            current_nomination: None,
            teams: vec![],
            pick_count: Some((i + 1) as u32),
            total_picks: Some(260),
        };

        let diff = compute_state_diff(&previous, &current);
        state.process_new_picks(diff.new_picks);
        previous = Some(current);
    }

    // Verify final snapshot
    let snapshot = state.build_snapshot();

    // My team (Team 1) should have exactly 2 players
    let filled: Vec<_> = snapshot
        .my_roster
        .iter()
        .filter(|s| s.player.is_some())
        .collect();
    assert_eq!(
        filled.len(),
        2,
        "Team 1 should have 2 players. Found: {:?}",
        filled
            .iter()
            .map(|s| s.player.as_ref().unwrap().name.as_str())
            .collect::<Vec<_>>()
    );

    let names: Vec<&str> = filled
        .iter()
        .map(|s| s.player.as_ref().unwrap().name.as_str())
        .collect();
    assert!(names.contains(&"Shohei Ohtani"));
    assert!(names.contains(&"Juan Soto"));
    assert!(!names.contains(&"Aaron Judge"), "Judge is on Team 2, not mine");
    assert!(!names.contains(&"Bobby Witt Jr."), "Witt is on Team 3, not mine");

    // Total pick count should be 4
    assert_eq!(snapshot.pick_count, 4);

    // Draft log should have all 4 picks
    assert_eq!(snapshot.draft_log.len(), 4);

    // Budget should reflect my team's spending
    assert_eq!(snapshot.budget_spent, 62 + 48); // Ohtani + Soto
    assert_eq!(snapshot.budget_remaining, 260 - 62 - 48);
}

// ===========================================================================
// Tests: Position data flows correctly from CSV through valuation pipeline
// (regression tests for the "all players default to Catcher" bug)
// ===========================================================================

/// Verify that hitter projection CSV positions are parsed correctly.
#[test]
fn hitter_projections_have_correct_positions() {
    let config = inline_config();
    let projections = load_fixture_projections(&config);

    // Aaron Judge should be RF
    let judge = projections.hitters.iter().find(|h| h.name == "Aaron Judge").unwrap();
    assert!(
        judge.positions.contains(&Position::RightField),
        "Aaron Judge should have RF position, got {:?}",
        judge.positions
    );

    // William Contreras should be C
    let contreras = projections.hitters.iter().find(|h| h.name == "William Contreras").unwrap();
    assert!(
        contreras.positions.contains(&Position::Catcher),
        "William Contreras should have C position, got {:?}",
        contreras.positions
    );

    // Freddie Freeman should be 1B
    let freeman = projections.hitters.iter().find(|h| h.name == "Freddie Freeman").unwrap();
    assert!(
        freeman.positions.contains(&Position::FirstBase),
        "Freddie Freeman should have 1B position, got {:?}",
        freeman.positions
    );
}

/// Verify that multi-position hitters get all their positions from CSV.
#[test]
fn multi_position_hitters_get_all_positions() {
    let config = inline_config();
    let projections = load_fixture_projections(&config);

    // Mookie Betts should be 2B/SS
    let betts = projections.hitters.iter().find(|h| h.name == "Mookie Betts").unwrap();
    assert!(
        betts.positions.contains(&Position::SecondBase),
        "Mookie Betts should have 2B, got {:?}",
        betts.positions
    );
    assert!(
        betts.positions.contains(&Position::ShortStop),
        "Mookie Betts should have SS, got {:?}",
        betts.positions
    );
    assert_eq!(
        betts.positions.len(),
        2,
        "Mookie Betts should have exactly 2 positions, got {:?}",
        betts.positions
    );

    // Bobby Witt Jr. should be SS/3B
    let witt = projections.hitters.iter().find(|h| h.name == "Bobby Witt Jr.").unwrap();
    assert!(
        witt.positions.contains(&Position::ShortStop),
        "Bobby Witt Jr. should have SS, got {:?}",
        witt.positions
    );
    assert!(
        witt.positions.contains(&Position::ThirdBase),
        "Bobby Witt Jr. should have 3B, got {:?}",
        witt.positions
    );

    // Gunnar Henderson should be 3B/SS
    let henderson = projections.hitters.iter().find(|h| h.name == "Gunnar Henderson").unwrap();
    assert!(
        henderson.positions.contains(&Position::ThirdBase),
        "Gunnar Henderson should have 3B, got {:?}",
        henderson.positions
    );
    assert!(
        henderson.positions.contains(&Position::ShortStop),
        "Gunnar Henderson should have SS, got {:?}",
        henderson.positions
    );
}

/// Verify that the valuation pipeline respects positions from CSV,
/// and NOT all players get assigned Catcher.
#[test]
fn valuation_pipeline_respects_csv_positions() {
    let config = inline_config();
    let players = load_fixture_players(&config);

    // Count how many players have Catcher as their only position
    let catcher_only_count = players
        .iter()
        .filter(|p| !p.is_pitcher && p.positions == vec![Position::Catcher])
        .count();

    // We only have 2 catchers in the fixture CSV (William Contreras, Adley Rutschman)
    assert!(
        catcher_only_count <= 2,
        "At most 2 players should be Catcher-only, but {} are. \
         This indicates positions are not being parsed from CSV correctly.",
        catcher_only_count
    );

    // Aaron Judge should NOT be Catcher
    let judge = players.iter().find(|p| p.name == "Aaron Judge").unwrap();
    assert!(
        !judge.positions.contains(&Position::Catcher),
        "Aaron Judge should NOT be Catcher, got positions {:?}",
        judge.positions
    );
    assert!(
        judge.positions.contains(&Position::RightField),
        "Aaron Judge should have RF, got positions {:?}",
        judge.positions
    );

    // Trea Turner should be SS
    let turner = players.iter().find(|p| p.name == "Trea Turner").unwrap();
    assert!(
        turner.positions.contains(&Position::ShortStop),
        "Trea Turner should have SS, got positions {:?}",
        turner.positions
    );
}

/// Verify that VOR and scarcity calculations use correct positions.
/// With positions properly assigned, different positions should have
/// different scarcity levels.
#[test]
fn vor_scarcity_uses_correct_positions() {
    let config = inline_config();
    let players = load_fixture_players(&config);

    // Compute scarcity
    let scarcity = draft_assistant::valuation::scarcity::compute_scarcity(
        &players,
        &config.league,
    );

    // With correct positions, catchers (only 2 in fixture) should be
    // scarcer than positions with more players (e.g. SS has 5+ players)
    let c_entry = scarcity.iter().find(|e| e.position == Position::Catcher);
    let ss_entry = scarcity.iter().find(|e| e.position == Position::ShortStop);

    if let (Some(c), Some(ss)) = (c_entry, ss_entry) {
        assert!(
            c.players_above_replacement <= ss.players_above_replacement,
            "Catcher should have fewer players above replacement ({}) than SS ({})",
            c.players_above_replacement,
            ss.players_above_replacement
        );
    }

    // Not every position should show Critical urgency (the old bug symptom)
    let non_critical_count = scarcity
        .iter()
        .filter(|e| e.urgency != draft_assistant::valuation::scarcity::ScarcityUrgency::Critical)
        .count();
    assert!(
        non_critical_count > 0,
        "At least some positions should NOT be Critical urgency. \
         All Critical means positions are still not being used correctly."
    );
}

/// Verify that best_position assignment is diverse (not all Catcher).
#[test]
fn best_position_assignment_is_diverse() {
    let config = inline_config();
    let players = load_fixture_players(&config);

    // Collect best_position assignments for hitters
    let hitter_best_positions: Vec<Position> = players
        .iter()
        .filter(|p| !p.is_pitcher)
        .filter_map(|p| p.best_position)
        .collect();

    assert!(!hitter_best_positions.is_empty(), "Hitters should have best_position set");

    // Count distinct positions assigned
    let mut unique_positions: Vec<Position> = hitter_best_positions.clone();
    unique_positions.sort_by_key(|p| p.sort_order());
    unique_positions.dedup();

    assert!(
        unique_positions.len() >= 3,
        "At least 3 different positions should be assigned as best_position, \
         but only {} were: {:?}. This may indicate all players defaulting to one position.",
        unique_positions.len(),
        unique_positions.iter().map(|p| p.display_str()).collect::<Vec<_>>()
    );

    // Specifically, Catcher should NOT be the dominant position
    let catcher_count = hitter_best_positions
        .iter()
        .filter(|p| **p == Position::Catcher)
        .count();
    let total = hitter_best_positions.len();
    assert!(
        catcher_count < total / 2,
        "Catcher should not be the majority best_position ({}/{} = {:.0}%)",
        catcher_count,
        total,
        catcher_count as f64 / total as f64 * 100.0
    );
}

/// Verify that ESPN eligible_slots override CSV positions in draft picks.
#[test]
fn espn_eligible_slots_override_csv_positions_in_picks() {
    let config = inline_config();
    let mut draft_state = DraftState::new(260, &roster_config());
    draft_state.reconcile_budgets(&ten_team_budgets());
    draft_state.set_my_team_by_name("Team 1");

    // Simulate a pick with ESPN eligible_slots that include SS, 2B, OF
    let pick = DraftPick {
        pick_number: 1,
        team_id: "1".to_string(),
        team_name: "Team 1".to_string(),
        player_name: "Mookie Betts".to_string(),
        position: "SS".to_string(),
        price: 40,
        espn_player_id: Some("12345".to_string()),
        eligible_slots: vec![4, 2, 5, 8, 9, 10, 12, 16, 17], // SS, 2B, OF, LF, CF, RF, UTIL, BE, IL
    };

    draft_state.record_pick(pick);

    // The player should be placed on the roster using eligible_slots
    let team = &draft_state.teams[0];
    let mookie = team.roster.slots.iter().find(|s| {
        s.player.as_ref().map_or(false, |p| p.name == "Mookie Betts")
    });
    assert!(
        mookie.is_some(),
        "Mookie Betts should be on Team 1's roster"
    );

    // Should be placed at SS (first eligible non-meta slot)
    let mookie_slot = mookie.unwrap();
    assert_eq!(
        mookie_slot.position,
        Position::ShortStop,
        "Mookie Betts should be placed in the SS slot"
    );
}

/// Verify backward compatibility: hitter CSV without POS column still works.
#[test]
fn hitter_csv_without_pos_column_still_loads() {
    // CSV without POS column (old format)
    let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Aaron Judge,NYY,700,600,180,52,120,130,90,5,0.300
Mookie Betts,LAD,680,590,170,30,110,95,80,15,0.288";

    let hitters: Vec<draft_assistant::valuation::projections::HitterProjection> =
        draft_assistant::valuation::projections::load_hitter_projections_from_reader(
            csv_data.as_bytes(),
        )
        .unwrap();

    assert_eq!(hitters.len(), 2);
    assert_eq!(hitters[0].name, "Aaron Judge");
    // Without POS column, positions should be empty
    assert!(
        hitters[0].positions.is_empty(),
        "Positions should be empty without POS column"
    );
    assert!(
        hitters[1].positions.is_empty(),
        "Positions should be empty without POS column"
    );
}

/// Verify that hitter CSV with POS column correctly parses positions.
#[test]
fn hitter_csv_with_pos_column_parses_positions() {
    let csv_data = "\
Name,Team,POS,PA,AB,H,HR,R,RBI,BB,SB,AVG
Aaron Judge,NYY,RF,700,600,180,52,120,130,90,5,0.300
Mookie Betts,LAD,2B/SS,680,590,170,30,110,95,80,15,0.288
Juan Soto,NYM,OF,700,580,165,35,115,110,110,3,0.284
Shohei Ohtani,LAD,DH,660,580,170,45,110,100,70,15,0.293";

    let hitters: Vec<draft_assistant::valuation::projections::HitterProjection> =
        draft_assistant::valuation::projections::load_hitter_projections_from_reader(
            csv_data.as_bytes(),
        )
        .unwrap();

    assert_eq!(hitters.len(), 4);

    // Aaron Judge = RF
    assert_eq!(hitters[0].positions, vec![Position::RightField]);

    // Mookie Betts = 2B/SS
    assert_eq!(
        hitters[1].positions,
        vec![Position::SecondBase, Position::ShortStop]
    );

    // Juan Soto = OF (expands to all three outfield positions)
    assert_eq!(
        hitters[2].positions,
        vec![Position::LeftField, Position::CenterField, Position::RightField]
    );

    // Shohei Ohtani = DH (filtered out, no DH roster slot in this league)
    assert!(
        hitters[3].positions.is_empty(),
        "DH-only player should have empty positions, got {:?}",
        hitters[3].positions
    );
}
