// Valuation engine: z-scores, VOR, auction dollar conversion.

pub mod analysis;
pub mod auction;
pub mod projections;
pub mod scarcity;
pub mod vor;
pub mod zscore;

use std::collections::HashMap;

use crate::config::{Config, LeagueConfig, StrategyConfig};
use crate::draft::state::DraftState;
use projections::AllProjections;
use crate::stats::{self, CategoryValues, StatRegistry};
use zscore::{
    CategoryZScores, PlayerValuation,
    compute_generic_pool_stats, compute_player_category_zscores,
    weights_to_category_values,
};

// ---------------------------------------------------------------------------
// Full valuation pipeline
// ---------------------------------------------------------------------------

/// Run the complete initial valuation pipeline:
///
/// 1. **Z-scores** — compute per-category z-scores for every player, producing
///    a `Vec<PlayerValuation>` sorted by total z-score.
/// 2. **VOR** — adjust z-scores by positional replacement level, sort by VOR.
/// 3. **Auction dollars** — convert VOR into dollar values using the league's
///    salary cap, sort by dollar value descending.
///
/// The returned list is sorted by descending dollar value, ready for display
/// or further processing (inflation tracking, scarcity adjustments, etc.).
pub fn compute_initial(
    projections: &AllProjections,
    config: &Config,
    roster_config: &HashMap<String, usize>,
) -> anyhow::Result<Vec<PlayerValuation>> {
    let registry = StatRegistry::from_league_config(&config.league)
        .expect("StatRegistry must be valid for configured categories");
    let weight_values = weights_to_category_values(&config.strategy.weights, &registry);

    // Step 1: Z-scores
    let mut players = zscore::compute_initial_zscores(
        projections, config, &registry, &weight_values,
    );

    // Step 2: VOR adjustment
    vor::apply_vor(&mut players, roster_config, config.league.num_teams);

    // Snapshot initial VOR for stable scarcity computation.
    for player in players.iter_mut() {
        player.initial_vor = player.vor;
    }

    // Step 3: Auction dollar conversion
    auction::apply_auction_values(&mut players, roster_config, config.league.num_teams, config.league.salary_cap, &config.strategy);

    Ok(players)
}

// ---------------------------------------------------------------------------
// Dynamic recalculation (post-pick)
// ---------------------------------------------------------------------------

/// Recompute z-scores, VOR, and auction dollar values for the remaining
/// available player pool. This should be called when the user changes
/// strategy configuration (e.g. category weights), NOT after every draft
/// pick. Base valuations are computed once at startup via `compute_initial()`
/// and remain stable throughout the draft; only inflation tracking and
/// scarcity indices update as picks happen.
///
/// # Algorithm
/// 1. Separate players into hitter and pitcher sub-pools.
/// 2. Recompute pool statistics and z-scores from embedded projection data.
/// 3. Recompute replacement levels and VOR.
/// 4. Recompute auction values (incorporating current draft budget state).
/// 5. Sort by dollar value descending.
///
/// The `available_players` vector is mutated in place.
pub fn recalculate_all(
    available_players: &mut Vec<PlayerValuation>,
    roster_config: &HashMap<String, usize>,
    league: &LeagueConfig,
    strategy: &StrategyConfig,
    _draft_state: &DraftState,
) {
    if available_players.is_empty() {
        return;
    }

    let registry = StatRegistry::from_league_config(league)
        .expect("StatRegistry must be valid for configured categories");
    let weight_values = weights_to_category_values(&strategy.weights, &registry);

    // ---- 1. Separate into hitter/pitcher/two-way pools ----
    let hitter_indices: Vec<usize> = available_players
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.is_pitcher) // includes two-way (is_pitcher = false)
        .map(|(i, _)| i)
        .collect();

    let pitcher_indices: Vec<usize> = available_players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_pitcher && !p.is_two_way)
        .map(|(i, _)| i)
        .collect();

    let two_way_indices: Vec<usize> = available_players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_two_way)
        .map(|(i, _)| i)
        .collect();

    let all_pitching_indices: Vec<usize> = pitcher_indices
        .iter()
        .chain(two_way_indices.iter())
        .copied()
        .collect();

    // ---- 2. Compute pool stats via generic registry-driven loop ----
    let hitter_pool_data: Vec<stats::ProjectionData> = hitter_indices
        .iter()
        .map(|&i| stats::ProjectionData::from(&available_players[i].projection))
        .collect();
    let pitcher_pool_data: Vec<stats::ProjectionData> = all_pitching_indices
        .iter()
        .map(|&i| stats::ProjectionData::from(&available_players[i].projection))
        .collect();

    let (hitter_stats, hitter_league_avgs) = compute_generic_pool_stats(
        &hitter_pool_data, registry.batting_indices(), &registry,
    );
    let (pitcher_stats, pitcher_league_avgs) = compute_generic_pool_stats(
        &pitcher_pool_data, registry.pitching_indices(), &registry,
    );

    // ---- 3. Recompute z-scores for pure hitters ----
    for &i in &hitter_indices {
        if available_players[i].is_two_way {
            continue; // handled after pitcher pool stats are ready
        }
        let proj = stats::ProjectionData::from(&available_players[i].projection);
        let mut zscores = CategoryValues::zeros(registry.len());
        let total = compute_player_category_zscores(
            &proj, &hitter_stats, &hitter_league_avgs,
            registry.batting_indices(), &registry, &weight_values,
            &mut zscores,
        );
        available_players[i].category_zscores = CategoryZScores::hitter(zscores, total);
        available_players[i].total_zscore = total;
    }

    // ---- 4. Recompute z-scores for pure pitchers ----
    for &i in &pitcher_indices {
        let proj = stats::ProjectionData::from(&available_players[i].projection);
        let mut zscores = CategoryValues::zeros(registry.len());
        let total = compute_player_category_zscores(
            &proj, &pitcher_stats, &pitcher_league_avgs,
            registry.pitching_indices(), &registry, &weight_values,
            &mut zscores,
        );
        available_players[i].category_zscores = CategoryZScores::pitcher(zscores, total);
        available_players[i].total_zscore = total;
    }

    // ---- 5. Recompute two-way player z-scores (needs both pool stats) ----
    for &i in &two_way_indices {
        let proj = stats::ProjectionData::from(&available_players[i].projection);
        let mut zscores = CategoryValues::zeros(registry.len());
        let batting_total = compute_player_category_zscores(
            &proj, &hitter_stats, &hitter_league_avgs,
            registry.batting_indices(), &registry, &weight_values,
            &mut zscores,
        );
        let pitching_total = compute_player_category_zscores(
            &proj, &pitcher_stats, &pitcher_league_avgs,
            registry.pitching_indices(), &registry, &weight_values,
            &mut zscores,
        );
        let combined = batting_total + pitching_total;
        available_players[i].category_zscores =
            CategoryZScores::two_way(zscores, batting_total, pitching_total);
        available_players[i].total_zscore = combined;
    }

    // ---- 6. Recompute VOR ----
    vor::apply_vor(available_players, roster_config, league.num_teams);

    // ---- 7. Recompute auction values ----
    auction::apply_auction_values(available_players, roster_config, league.num_teams, league.salary_cap, strategy);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::draft::pick::Position;
    use crate::draft::state::DraftState;
    use crate::valuation::projections::PitcherType;
    use std::collections::HashMap;

    fn test_league_config() -> LeagueConfig {
        LeagueConfig {
            name: "Test League".into(),
            platform: "espn".into(),
            num_teams: 2,
            scoring_type: "h2h_most_categories".into(),
            salary_cap: 260,
            batting_categories: CategoriesSection {
                categories: vec![
                    "R".into(), "HR".into(), "RBI".into(),
                    "BB".into(), "SB".into(), "AVG".into(),
                ],
            },
            pitching_categories: CategoriesSection {
                categories: vec![
                    "K".into(), "W".into(), "SV".into(),
                    "HD".into(), "ERA".into(), "WHIP".into(),
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

    fn test_strategy_config() -> StrategyConfig {
        StrategyConfig {
            hitting_budget_fraction: 0.65,
            weights: CategoryWeights::from_pairs([
                ("R", 1.0), ("HR", 1.0), ("RBI", 1.0), ("BB", 1.2),
                ("SB", 1.0), ("AVG", 1.0), ("K", 1.0), ("W", 1.0),
                ("SV", 0.7), ("HD", 1.3), ("ERA", 1.0), ("WHIP", 1.0),
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

    fn make_hitter(
        name: &str,
        r: u32, hr: u32, rbi: u32, bb: u32, sb: u32,
        ab: u32, avg: f64,
        positions: Vec<Position>,
    ) -> PlayerValuation {
        PlayerValuation {
            name: name.into(),
            team: "TST".into(),
            positions,
            is_pitcher: false,
            is_two_way: false,
            pitcher_type: None,
            projection: zscore::ProjectionData {
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
            category_zscores: CategoryZScores::zeros_hitter(12),
            vor: 0.0,
            initial_vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
        }
    }

    fn make_pitcher(
        name: &str,
        k: u32, w: u32, sv: u32, hd: u32,
        ip: f64, era: f64, whip: f64,
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
            projection: zscore::ProjectionData {
                values: HashMap::from([
                    ("ip".into(), ip),
                    ("k".into(), k as f64),
                    ("w".into(), w as f64),
                    ("sv".into(), sv as f64),
                    ("hd".into(), hd as f64),
                    ("era".into(), era),
                    ("whip".into(), whip),
                    ("g".into(), 30.0),
                    ("gs".into(), if pitcher_type == PitcherType::SP { 30.0 } else { 0.0 }),
                ]),
            },
            total_zscore: 0.0,
            category_zscores: CategoryZScores::zeros_pitcher(12),
            vor: 0.0,
            initial_vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
        }
    }

    fn test_espn_budgets() -> Vec<crate::draft::state::TeamBudgetPayload> {
        (1..=2)
            .map(|i| crate::draft::state::TeamBudgetPayload {
                team_id: format!("{}", i),
                team_name: format!("Team {}", i),
                budget: 260,
            })
            .collect()
    }

    fn create_test_draft_state() -> DraftState {
        let mut state = DraftState::new(260, &test_roster_config());
        state.reconcile_budgets(&test_espn_budgets());
        state.set_my_team_by_id("1");
        state
    }

    fn test_roster_config() -> HashMap<String, usize> {
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

    #[test]
    fn values_stable_after_player_removal() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = create_test_draft_state();

        // Create a pool of hitters and pitchers with varied stats.
        let mut players = vec![
            make_hitter("H_Star", 100, 40, 100, 70, 20, 550, 0.300, vec![Position::FirstBase]),
            make_hitter("H_Good", 80, 25, 75, 55, 15, 530, 0.280, vec![Position::SecondBase]),
            make_hitter("H_Mid", 60, 15, 55, 40, 10, 500, 0.265, vec![Position::ShortStop]),
            make_hitter("H_Low", 45, 8, 40, 30, 5, 480, 0.250, vec![Position::Catcher]),
            make_pitcher("P_Ace", 250, 18, 0, 0, 200.0, 2.80, 1.00, PitcherType::SP),
            make_pitcher("P_Good", 200, 14, 0, 0, 180.0, 3.20, 1.10, PitcherType::SP),
            make_pitcher("P_Mid", 150, 10, 0, 0, 160.0, 3.80, 1.20, PitcherType::SP),
        ];

        // Initial calculation.
        let roster = test_roster_config();
        recalculate_all(&mut players, &roster, &league, &strategy, &draft_state);

        // Record values for remaining players.
        let mid_value = players.iter().find(|p| p.name == "H_Mid").unwrap().dollar_value;
        let mid_zscore = players.iter().find(|p| p.name == "H_Mid").unwrap().total_zscore;
        let ace_value = players.iter().find(|p| p.name == "P_Ace").unwrap().dollar_value;

        // Remove the star hitter (simulating they were drafted).
        // In the real app, we do NOT call recalculate_all after this.
        players.retain(|p| p.name != "H_Star");

        // Values on remaining players should be unchanged.
        let new_mid_value = players.iter().find(|p| p.name == "H_Mid").unwrap().dollar_value;
        let new_mid_zscore = players.iter().find(|p| p.name == "H_Mid").unwrap().total_zscore;
        let new_ace_value = players.iter().find(|p| p.name == "P_Ace").unwrap().dollar_value;

        assert_eq!(mid_value, new_mid_value, "H_Mid dollar value should be unchanged after removal");
        assert_eq!(mid_zscore, new_mid_zscore, "H_Mid z-score should be unchanged after removal");
        assert_eq!(ace_value, new_ace_value, "P_Ace dollar value should be unchanged after removal");
    }

    #[test]
    fn recalculate_all_empty_pool() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = create_test_draft_state();

        let mut players: Vec<PlayerValuation> = Vec::new();
        let roster = test_roster_config();
        recalculate_all(&mut players, &roster, &league, &strategy, &draft_state);
        assert!(players.is_empty());
    }

    #[test]
    fn recalculate_all_pitchers_only() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = create_test_draft_state();

        let mut players = vec![
            make_pitcher("SP1", 220, 16, 0, 0, 190.0, 3.00, 1.05, PitcherType::SP),
            make_pitcher("SP2", 180, 12, 0, 0, 170.0, 3.40, 1.15, PitcherType::SP),
            make_pitcher("RP1", 80, 2, 35, 0, 65.0, 2.50, 0.95, PitcherType::RP),
        ];

        let roster = test_roster_config();
        recalculate_all(&mut players, &roster, &league, &strategy, &draft_state);

        // All should have valid values.
        for p in &players {
            assert!(p.dollar_value >= 1.0);
            assert!(p.total_zscore.is_finite());
        }
    }

    #[test]
    fn recalculate_all_hitters_only() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = create_test_draft_state();

        let mut players = vec![
            make_hitter("H1", 90, 35, 95, 60, 15, 550, 0.290, vec![Position::FirstBase]),
            make_hitter("H2", 70, 20, 65, 45, 10, 520, 0.270, vec![Position::ThirdBase]),
        ];

        let roster = test_roster_config();
        recalculate_all(&mut players, &roster, &league, &strategy, &draft_state);

        for p in &players {
            assert!(p.dollar_value >= 1.0);
            assert!(p.total_zscore.is_finite());
        }
    }

    // ---- Two-way player tests ----

    fn make_two_way(
        name: &str,
        // Hitting stats
        r: u32, hr: u32, rbi: u32, bb: u32, sb: u32,
        ab: u32, avg: f64,
        // Pitching stats
        k: u32, w: u32, sv: u32, hd: u32,
        ip: f64, era: f64, whip: f64,
        pitcher_type: crate::valuation::projections::PitcherType,
        positions: Vec<Position>,
    ) -> PlayerValuation {
        let pos = match pitcher_type {
            crate::valuation::projections::PitcherType::SP => Position::StartingPitcher,
            crate::valuation::projections::PitcherType::RP => Position::ReliefPitcher,
        };
        let mut all_positions = positions;
        if !all_positions.contains(&pos) {
            all_positions.push(pos);
        }
        PlayerValuation {
            name: name.into(),
            team: "TST".into(),
            positions: all_positions,
            is_pitcher: false,
            is_two_way: true,
            pitcher_type: Some(pitcher_type),
            projection: zscore::ProjectionData {
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
                    ("ip".into(), ip),
                    ("k".into(), k as f64),
                    ("w".into(), w as f64),
                    ("sv".into(), sv as f64),
                    ("hd".into(), hd as f64),
                    ("era".into(), era),
                    ("whip".into(), whip),
                    ("g".into(), 30.0),
                    ("gs".into(), if pitcher_type == crate::valuation::projections::PitcherType::SP { 30.0 } else { 0.0 }),
                ]),
            },
            total_zscore: 0.0,
            category_zscores: CategoryZScores::two_way(CategoryValues::zeros(12), 0.0, 0.0),
            vor: 0.0,
            initial_vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
        }
    }

    #[test]
    fn recalculate_all_with_two_way_player() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = create_test_draft_state();

        let mut players = vec![
            // Two-way player: elite hitter + solid pitcher
            make_two_way(
                "Ohtani", 100, 40, 100, 60, 15, 550, 0.300,
                200, 14, 0, 0, 160.0, 2.80, 1.00,
                PitcherType::SP, vec![Position::Utility],
            ),
            // Regular hitters
            make_hitter("H_Good", 80, 25, 75, 55, 15, 530, 0.280, vec![Position::FirstBase]),
            make_hitter("H_Mid", 60, 15, 55, 40, 10, 500, 0.265, vec![Position::SecondBase]),
            make_hitter("H_Low", 45, 8, 40, 30, 5, 480, 0.250, vec![Position::Catcher]),
            // Regular pitchers
            make_pitcher("P_Ace", 250, 18, 0, 0, 200.0, 2.80, 1.00, PitcherType::SP),
            make_pitcher("P_Good", 200, 14, 0, 0, 180.0, 3.20, 1.10, PitcherType::SP),
            make_pitcher("P_Mid", 150, 10, 0, 0, 160.0, 3.80, 1.20, PitcherType::SP),
        ];

        let roster = test_roster_config();
        recalculate_all(&mut players, &roster, &league, &strategy, &draft_state);

        // The two-way player should have a valid dollar value.
        let ohtani = players.iter().find(|p| p.name == "Ohtani").unwrap();
        assert!(
            ohtani.dollar_value >= 1.0,
            "Two-way player should have value >= $1, got {}",
            ohtani.dollar_value,
        );
        assert!(
            ohtani.total_zscore.is_finite(),
            "Two-way player should have finite z-score",
        );

        // Two-way player should have TwoWay z-scores after recalculation.
        match &ohtani.category_zscores {
            CategoryZScores::TwoWay { batting_total, pitching_total, .. } => {
                assert!(batting_total.is_finite());
                assert!(pitching_total.is_finite());
            }
            other => panic!("Expected TwoWay z-scores after recalculate, got {:?}", other),
        }

        // The two-way player should be valued higher than similar pure hitters
        // because of the combined hitting + pitching contribution.
        let h_good = players.iter().find(|p| p.name == "H_Good").unwrap();
        assert!(
            ohtani.dollar_value > h_good.dollar_value,
            "Two-way player (${}) should be valued higher than a good pure hitter (${})",
            ohtani.dollar_value,
            h_good.dollar_value,
        );

        // All players should have valid values and be sorted.
        for p in &players {
            assert!(p.dollar_value >= 1.0);
            assert!(p.total_zscore.is_finite());
        }
        for i in 1..players.len() {
            assert!(
                players[i - 1].dollar_value >= players[i].dollar_value,
                "Not sorted: {} (${}) >= {} (${})",
                players[i - 1].name,
                players[i - 1].dollar_value,
                players[i].name,
                players[i].dollar_value,
            );
        }
    }

    #[test]
    fn two_way_player_auction_value_reflects_dual_contribution() {
        // Compare the auction dollar value of a two-way player vs. the sum
        // of equivalent pure hitter and pure pitcher.
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = create_test_draft_state();

        // Pool with a two-way player.
        let mut with_two_way = vec![
            make_two_way(
                "TwoWay", 90, 35, 90, 55, 12, 540, 0.290,
                180, 12, 0, 0, 150.0, 3.00, 1.05,
                PitcherType::SP, vec![Position::Utility],
            ),
            make_hitter("FillerH1", 70, 20, 65, 45, 10, 520, 0.270, vec![Position::FirstBase]),
            make_hitter("FillerH2", 50, 10, 45, 30, 5, 480, 0.250, vec![Position::Catcher]),
            make_pitcher("FillerSP1", 200, 15, 0, 0, 190.0, 3.20, 1.10, PitcherType::SP),
            make_pitcher("FillerSP2", 160, 11, 0, 0, 170.0, 3.60, 1.15, PitcherType::SP),
        ];

        let roster = test_roster_config();
        recalculate_all(&mut with_two_way, &roster, &league, &strategy, &draft_state);

        let two_way_value = with_two_way.iter().find(|p| p.name == "TwoWay").unwrap().dollar_value;

        // Equivalent pool with same player split as pure hitter + pure pitcher.
        let mut without_two_way = vec![
            make_hitter("SplitH", 90, 35, 90, 55, 12, 540, 0.290, vec![Position::Utility]),
            make_pitcher("SplitP", 180, 12, 0, 0, 150.0, 3.00, 1.05, PitcherType::SP),
            make_hitter("FillerH1", 70, 20, 65, 45, 10, 520, 0.270, vec![Position::FirstBase]),
            make_hitter("FillerH2", 50, 10, 45, 30, 5, 480, 0.250, vec![Position::Catcher]),
            make_pitcher("FillerSP1", 200, 15, 0, 0, 190.0, 3.20, 1.10, PitcherType::SP),
            make_pitcher("FillerSP2", 160, 11, 0, 0, 170.0, 3.60, 1.15, PitcherType::SP),
        ];

        recalculate_all(&mut without_two_way, &roster, &league, &strategy, &draft_state);

        let split_hitter_value = without_two_way.iter().find(|p| p.name == "SplitH").unwrap().dollar_value;
        let split_pitcher_value = without_two_way.iter().find(|p| p.name == "SplitP").unwrap().dollar_value;

        // The two-way player's single-slot value should be substantial.
        // It won't exactly equal the sum of split values (different pool dynamics),
        // but it should be meaningfully higher than either split alone.
        assert!(
            two_way_value > split_hitter_value,
            "Two-way value (${:.1}) should exceed split hitter value (${:.1})",
            two_way_value,
            split_hitter_value,
        );
        assert!(
            two_way_value > split_pitcher_value,
            "Two-way value (${:.1}) should exceed split pitcher value (${:.1})",
            two_way_value,
            split_pitcher_value,
        );
    }

    #[test]
    fn two_way_values_stable_after_pick_removal() {
        // Verify that removing a two-way player from the pool doesn't
        // affect remaining player valuations.
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = create_test_draft_state();

        let mut players = vec![
            make_two_way(
                "Ohtani", 100, 40, 100, 60, 15, 550, 0.300,
                200, 14, 0, 0, 160.0, 2.80, 1.00,
                PitcherType::SP, vec![Position::Utility],
            ),
            make_hitter("H1", 80, 25, 75, 55, 15, 530, 0.280, vec![Position::FirstBase]),
            make_hitter("H2", 60, 15, 55, 40, 10, 500, 0.265, vec![Position::SecondBase]),
            make_pitcher("SP1", 250, 18, 0, 0, 200.0, 2.80, 1.00, PitcherType::SP),
            make_pitcher("SP2", 200, 14, 0, 0, 180.0, 3.20, 1.10, PitcherType::SP),
        ];

        // Initial calculation with two-way player present.
        let roster = test_roster_config();
        recalculate_all(&mut players, &roster, &league, &strategy, &draft_state);

        // Record values.
        let h1_value = players.iter().find(|p| p.name == "H1").unwrap().dollar_value;
        let sp1_value = players.iter().find(|p| p.name == "SP1").unwrap().dollar_value;

        // Remove the two-way player (drafted).
        players.retain(|p| p.name != "Ohtani");

        // Remaining values should be unchanged (no recalculation).
        let new_h1_value = players.iter().find(|p| p.name == "H1").unwrap().dollar_value;
        let new_sp1_value = players.iter().find(|p| p.name == "SP1").unwrap().dollar_value;

        assert_eq!(h1_value, new_h1_value, "H1 value should be unchanged");
        assert_eq!(sp1_value, new_sp1_value, "SP1 value should be unchanged");

        // All remaining players should still have valid values.
        for p in &players {
            assert!(p.dollar_value >= 1.0, "{} has value < $1", p.name);
            assert!(p.total_zscore.is_finite(), "{} has non-finite z-score", p.name);
        }
    }

    // Snapshot tests: capture exact numerical output to detect any divergence
    // during refactoring. Expected values recorded from the pre-refactor code.

    fn assert_close(actual: f64, expected: f64, label: &str) {
        assert!(
            (actual - expected).abs() < 1e-10,
            "{}: expected {:.15}, got {:.15}, diff={:.2e}",
            label, expected, actual, (actual - expected).abs(),
        );
    }


    fn find_player<'a>(players: &'a [PlayerValuation], name: &str) -> &'a PlayerValuation {
        players.iter().find(|p| p.name == name).unwrap()
    }

    #[test]
    fn snapshot_mixed_pool_values() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = create_test_draft_state();

        let mut players = vec![
            make_hitter("H_Star", 100, 40, 100, 70, 20, 550, 0.300, vec![Position::FirstBase]),
            make_hitter("H_Good", 80, 25, 75, 55, 15, 530, 0.280, vec![Position::SecondBase]),
            make_hitter("H_Mid", 60, 15, 55, 40, 10, 500, 0.265, vec![Position::ShortStop]),
            make_hitter("H_Low", 45, 8, 40, 30, 5, 480, 0.250, vec![Position::Catcher]),
            make_pitcher("P_Ace", 250, 18, 0, 0, 200.0, 2.80, 1.00, PitcherType::SP),
            make_pitcher("P_Good", 200, 14, 0, 0, 180.0, 3.20, 1.10, PitcherType::SP),
            make_pitcher("P_Mid", 150, 10, 0, 0, 160.0, 3.80, 1.20, PitcherType::SP),
        ];

        let roster = test_roster_config();
        recalculate_all(&mut players, &roster, &league, &strategy, &draft_state);

        // Snapshot values from registry-driven generic computation.
        // Note: hitter z-scores differ slightly from the pre-refactor manual code
        // because the generic formula computes league avg AVG as sum(AB*AVG)/sum(AB)
        // rather than sum(H)/sum(AB), consistent with compute_initial_zscores.
        // Pitcher values are identical since ERA/WHIP formulas are unchanged.
        assert_close(find_player(&players, "P_Ace").total_zscore, 4.903591098172604, "P_Ace zscore");
        assert_close(find_player(&players, "P_Ace").vor, 10.789528695091104, "P_Ace vor");
        assert_close(find_player(&players, "P_Ace").dollar_value, 101.087412931525733, "P_Ace dollar");

        assert_close(find_player(&players, "H_Star").total_zscore, 8.808019630015533, "H_Star zscore");
        assert_close(find_player(&players, "H_Star").vor, 1.0, "H_Star vor");
        assert_close(find_player(&players, "H_Star").dollar_value, 77.049999999999997, "H_Star dollar");

        assert_close(find_player(&players, "P_Good").total_zscore, -0.017653501254104, "P_Good zscore");
        assert_close(find_player(&players, "P_Good").vor, 5.868284095664396, "P_Good vor");
        assert_close(find_player(&players, "P_Good").dollar_value, 55.436239995310395, "P_Good dollar");

        assert_close(find_player(&players, "H_Good").total_zscore, 2.240440508846727, "H_Good zscore");
        assert_close(find_player(&players, "H_Mid").total_zscore, -3.328403489497330, "H_Mid zscore");
        assert_close(find_player(&players, "H_Low").total_zscore, -7.720056649364929, "H_Low zscore");
        assert_close(find_player(&players, "P_Mid").total_zscore, -4.885937596918500, "P_Mid zscore");
    }

    #[test]
    fn snapshot_two_way_values() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = create_test_draft_state();

        let mut players = vec![
            make_two_way(
                "Ohtani", 100, 40, 100, 60, 15, 550, 0.300,
                200, 14, 0, 0, 160.0, 2.80, 1.00,
                PitcherType::SP, vec![Position::Utility],
            ),
            make_hitter("H_Good", 80, 25, 75, 55, 15, 530, 0.280, vec![Position::FirstBase]),
            make_hitter("H_Mid", 60, 15, 55, 40, 10, 500, 0.265, vec![Position::SecondBase]),
            make_hitter("H_Low", 45, 8, 40, 30, 5, 480, 0.250, vec![Position::Catcher]),
            make_pitcher("P_Ace", 250, 18, 0, 0, 200.0, 2.80, 1.00, PitcherType::SP),
            make_pitcher("P_Good", 200, 14, 0, 0, 180.0, 3.20, 1.10, PitcherType::SP),
            make_pitcher("P_Mid", 150, 10, 0, 0, 160.0, 3.80, 1.20, PitcherType::SP),
        ];

        let roster = test_roster_config();
        recalculate_all(&mut players, &roster, &league, &strategy, &draft_state);

        let ohtani = find_player(&players, "Ohtani");
        assert_close(ohtani.total_zscore, 9.661721255392411, "Ohtani zscore");
        assert_close(ohtani.vor, 18.698403208516297, "Ohtani vor");
        assert_close(ohtani.dollar_value, 246.556410135758682, "Ohtani dollar");
        match &ohtani.category_zscores {
            CategoryZScores::TwoWay { batting_total, pitching_total, .. } => {
                assert_close(*batting_total, 8.072085220083544, "Ohtani batting_total");
                assert_close(*pitching_total, 1.589636035308868, "Ohtani pitching_total");
            }
            other => panic!("Expected TwoWay, got {:?}", other),
        }

        assert_close(find_player(&players, "P_Ace").total_zscore, 4.815472168882275, "P_Ace zscore");
        assert_close(find_player(&players, "P_Ace").vor, 11.673275899511701, "P_Ace vor");
        assert_close(find_player(&players, "H_Good").total_zscore, 3.083448550621077, "H_Good zscore");
        assert_close(find_player(&players, "P_Mid").total_zscore, -5.857803730629427, "P_Mid zscore");
    }
}
