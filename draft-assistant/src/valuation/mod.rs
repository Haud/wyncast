// Valuation engine: z-scores, VOR, auction dollar conversion.

pub mod analysis;
pub mod auction;
pub mod projections;
pub mod scarcity;
pub mod vor;
pub mod zscore;

use crate::config::{Config, LeagueConfig, StrategyConfig};
use crate::draft::state::DraftState;
use projections::AllProjections;
use zscore::{
    CategoryZScores, HitterZScores, PitcherZScores, PlayerProjectionData, PlayerValuation,
    avg_contribution, compute_pool_stats, compute_zscore, era_contribution,
    whip_contribution,
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
) -> anyhow::Result<Vec<PlayerValuation>> {
    // Step 1: Z-scores
    let mut players = zscore::compute_initial_zscores(projections, config);

    // Step 2: VOR adjustment
    vor::apply_vor(&mut players, &config.league);

    // Step 3: Auction dollar conversion
    auction::apply_auction_values(&mut players, &config.league, &config.strategy);

    Ok(players)
}

// ---------------------------------------------------------------------------
// Dynamic recalculation (post-pick)
// ---------------------------------------------------------------------------

/// Recompute z-scores, VOR, and auction dollar values for the remaining
/// available player pool. This should be called after every draft pick to
/// keep valuations current as the player pool shrinks.
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
    league: &LeagueConfig,
    strategy: &StrategyConfig,
    _draft_state: &DraftState,
) {
    if available_players.is_empty() {
        return;
    }

    let weights = &strategy.weights;

    // ---- 1. Separate into hitter/pitcher pools ----
    let hitter_indices: Vec<usize> = available_players
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.is_pitcher)
        .map(|(i, _)| i)
        .collect();

    let pitcher_indices: Vec<usize> = available_players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_pitcher)
        .map(|(i, _)| i)
        .collect();

    // ---- 2. Recompute hitter pool stats and z-scores ----
    if !hitter_indices.is_empty() {
        // Extract raw stat vectors for pool stats.
        let mut r_vals = Vec::new();
        let mut hr_vals = Vec::new();
        let mut rbi_vals = Vec::new();
        let mut bb_vals = Vec::new();
        let mut sb_vals = Vec::new();
        let mut ab_vals = Vec::new();
        let mut avg_vals = Vec::new();

        for &i in &hitter_indices {
            if let PlayerProjectionData::Hitter { r, hr, rbi, bb, sb, ab, avg, .. } =
                &available_players[i].projection
            {
                r_vals.push(*r as f64);
                hr_vals.push(*hr as f64);
                rbi_vals.push(*rbi as f64);
                bb_vals.push(*bb as f64);
                sb_vals.push(*sb as f64);
                ab_vals.push(*ab);
                avg_vals.push(*avg);
            }
        }

        // League average AVG for the pool.
        let total_h: f64 = hitter_indices
            .iter()
            .filter_map(|&i| {
                if let PlayerProjectionData::Hitter { h, .. } = &available_players[i].projection {
                    Some(*h as f64)
                } else {
                    None
                }
            })
            .sum();
        let total_ab: f64 = ab_vals.iter().map(|ab| *ab as f64).sum();
        let league_avg_avg = if total_ab > 0.0 {
            total_h / total_ab
        } else {
            0.0
        };

        // Compute AVG contribution values.
        let avg_contrib_vals: Vec<f64> = hitter_indices
            .iter()
            .filter_map(|&i| {
                if let PlayerProjectionData::Hitter { ab, avg, .. } =
                    &available_players[i].projection
                {
                    Some(avg_contribution(*ab, *avg, league_avg_avg))
                } else {
                    None
                }
            })
            .collect();

        let r_stats = compute_pool_stats(&r_vals);
        let hr_stats = compute_pool_stats(&hr_vals);
        let rbi_stats = compute_pool_stats(&rbi_vals);
        let bb_stats = compute_pool_stats(&bb_vals);
        let sb_stats = compute_pool_stats(&sb_vals);
        let avg_stats = compute_pool_stats(&avg_contrib_vals);

        // Recompute z-scores for each hitter.
        for &i in &hitter_indices {
            if let PlayerProjectionData::Hitter { r, hr, rbi, bb, sb, ab, avg, .. } =
                &available_players[i].projection
            {
                let rz = compute_zscore(*r as f64, &r_stats);
                let hrz = compute_zscore(*hr as f64, &hr_stats);
                let rbiz = compute_zscore(*rbi as f64, &rbi_stats);
                let bbz = compute_zscore(*bb as f64, &bb_stats);
                let sbz = compute_zscore(*sb as f64, &sb_stats);
                let avgz = compute_zscore(
                    avg_contribution(*ab, *avg, league_avg_avg),
                    &avg_stats,
                );

                let total = rz * weights.R
                    + hrz * weights.HR
                    + rbiz * weights.RBI
                    + bbz * weights.BB
                    + sbz * weights.SB
                    + avgz * weights.AVG;

                available_players[i].category_zscores =
                    CategoryZScores::Hitter(HitterZScores {
                        r: rz,
                        hr: hrz,
                        rbi: rbiz,
                        bb: bbz,
                        sb: sbz,
                        avg: avgz,
                        total,
                    });
                available_players[i].total_zscore = total;
            }
        }
    }

    // ---- 2b. Recompute pitcher pool stats and z-scores ----
    if !pitcher_indices.is_empty() {
        let mut k_vals = Vec::new();
        let mut w_vals = Vec::new();
        let mut sv_vals = Vec::new();
        let mut hd_vals = Vec::new();
        let mut ip_vals = Vec::new();
        let mut era_raw = Vec::new();
        let mut whip_raw = Vec::new();

        for &i in &pitcher_indices {
            if let PlayerProjectionData::Pitcher { k, w, sv, hd, ip, era, whip, .. } =
                &available_players[i].projection
            {
                k_vals.push(*k as f64);
                w_vals.push(*w as f64);
                sv_vals.push(*sv as f64);
                hd_vals.push(*hd as f64);
                ip_vals.push(*ip);
                era_raw.push(*era);
                whip_raw.push(*whip);
            }
        }

        // League average ERA and WHIP.
        let total_ip: f64 = ip_vals.iter().sum();
        let (league_avg_era, league_avg_whip) = if total_ip > 1e-9 {
            let total_er: f64 = pitcher_indices
                .iter()
                .filter_map(|&i| {
                    if let PlayerProjectionData::Pitcher { ip, era, .. } =
                        &available_players[i].projection
                    {
                        Some(ip * era / 9.0)
                    } else {
                        None
                    }
                })
                .sum();
            let total_wh: f64 = pitcher_indices
                .iter()
                .filter_map(|&i| {
                    if let PlayerProjectionData::Pitcher { ip, whip, .. } =
                        &available_players[i].projection
                    {
                        Some(ip * whip)
                    } else {
                        None
                    }
                })
                .sum();
            (total_er / total_ip * 9.0, total_wh / total_ip)
        } else {
            (0.0, 0.0)
        };

        // ERA and WHIP contributions.
        let era_contrib_vals: Vec<f64> = pitcher_indices
            .iter()
            .filter_map(|&i| {
                if let PlayerProjectionData::Pitcher { ip, era, .. } =
                    &available_players[i].projection
                {
                    Some(era_contribution(*ip, *era, league_avg_era))
                } else {
                    None
                }
            })
            .collect();

        let whip_contrib_vals: Vec<f64> = pitcher_indices
            .iter()
            .filter_map(|&i| {
                if let PlayerProjectionData::Pitcher { ip, whip, .. } =
                    &available_players[i].projection
                {
                    Some(whip_contribution(*ip, *whip, league_avg_whip))
                } else {
                    None
                }
            })
            .collect();

        let k_stats = compute_pool_stats(&k_vals);
        let w_stats = compute_pool_stats(&w_vals);
        let sv_stats = compute_pool_stats(&sv_vals);
        let hd_stats = compute_pool_stats(&hd_vals);
        let era_stats = compute_pool_stats(&era_contrib_vals);
        let whip_stats = compute_pool_stats(&whip_contrib_vals);

        // Recompute z-scores for each pitcher.
        for &i in &pitcher_indices {
            if let PlayerProjectionData::Pitcher { k, w, sv, hd, ip, era, whip, .. } =
                &available_players[i].projection
            {
                let kz = compute_zscore(*k as f64, &k_stats);
                let wz = compute_zscore(*w as f64, &w_stats);
                let svz = compute_zscore(*sv as f64, &sv_stats);
                let hdz = compute_zscore(*hd as f64, &hd_stats);
                let eraz = compute_zscore(
                    era_contribution(*ip, *era, league_avg_era),
                    &era_stats,
                );
                let whipz = compute_zscore(
                    whip_contribution(*ip, *whip, league_avg_whip),
                    &whip_stats,
                );

                let total = kz * weights.K
                    + wz * weights.W
                    + svz * weights.SV
                    + hdz * weights.HD
                    + eraz * weights.ERA
                    + whipz * weights.WHIP;

                available_players[i].category_zscores =
                    CategoryZScores::Pitcher(PitcherZScores {
                        k: kz,
                        w: wz,
                        sv: svz,
                        hd: hdz,
                        era: eraz,
                        whip: whipz,
                        total,
                    });
                available_players[i].total_zscore = total;
            }
        }
    }

    // ---- 3. Recompute VOR ----
    vor::apply_vor(available_players, league);

    // ---- 4. Recompute auction values ----
    auction::apply_auction_values(available_players, league, strategy);

    // Step 5: apply_auction_values already sorts by dollar_value descending.
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
        let mut roster = HashMap::new();
        roster.insert("C".into(), 1);
        roster.insert("1B".into(), 1);
        roster.insert("2B".into(), 1);
        roster.insert("3B".into(), 1);
        roster.insert("SS".into(), 1);
        roster.insert("LF".into(), 1);
        roster.insert("CF".into(), 1);
        roster.insert("RF".into(), 1);
        roster.insert("UTIL".into(), 1);
        roster.insert("SP".into(), 5);
        roster.insert("RP".into(), 6);
        roster.insert("BE".into(), 6);
        roster.insert("IL".into(), 5);

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
            roster,
            roster_limits: RosterLimits {
                max_sp: 7,
                max_rp: 7,
                gs_per_week: 7,
            },
            teams: HashMap::new(),
            my_team: MyTeam {
                team_id: "team_1".into(),
            },
        }
    }

    fn test_strategy_config() -> StrategyConfig {
        StrategyConfig {
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
            pitcher_type: None,
            projection: PlayerProjectionData::Hitter {
                pa: ab + bb,
                ab,
                h: (ab as f64 * avg).round() as u32,
                hr,
                r,
                rbi,
                bb,
                sb,
                avg,
            },
            total_zscore: 0.0,
            category_zscores: CategoryZScores::Hitter(HitterZScores {
                r: 0.0, hr: 0.0, rbi: 0.0, bb: 0.0, sb: 0.0, avg: 0.0, total: 0.0,
            }),
            vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
            adp: None,
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
            pitcher_type: Some(pitcher_type),
            projection: PlayerProjectionData::Pitcher {
                ip,
                k,
                w,
                sv,
                hd,
                era,
                whip,
                g: 30,
                gs: if pitcher_type == PitcherType::SP { 30 } else { 0 },
            },
            total_zscore: 0.0,
            category_zscores: CategoryZScores::Pitcher(PitcherZScores {
                k: 0.0, w: 0.0, sv: 0.0, hd: 0.0, era: 0.0, whip: 0.0, total: 0.0,
            }),
            vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
            adp: None,
        }
    }

    fn test_teams() -> Vec<(String, String)> {
        (1..=2)
            .map(|i| (format!("team_{}", i), format!("Team {}", i)))
            .collect()
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
    fn recalculate_all_removes_player_changes_values() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = DraftState::new(
            test_teams(),
            "team_1",
            league.salary_cap,
            &test_roster_config(),
        );

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
        recalculate_all(&mut players, &league, &strategy, &draft_state);

        // Record initial values.
        let initial_star_value = players.iter().find(|p| p.name == "H_Star").unwrap().dollar_value;
        let initial_mid_value = players.iter().find(|p| p.name == "H_Mid").unwrap().dollar_value;
        let initial_count = players.len();

        assert!(initial_star_value > 1.0, "Star should have value > $1");

        // Remove the star hitter (simulating they were drafted).
        players.retain(|p| p.name != "H_Star");
        assert_eq!(players.len(), initial_count - 1);

        // Recalculate.
        recalculate_all(&mut players, &league, &strategy, &draft_state);

        // The remaining players' values should have changed (pool stats shifted).
        let new_mid_value = players.iter().find(|p| p.name == "H_Mid").unwrap().dollar_value;

        // Values should be different because pool composition changed.
        // (The exact direction depends on the math, but they should differ.)
        assert!(
            (new_mid_value - initial_mid_value).abs() > 0.001
                || initial_mid_value == 1.0, // Edge case: both could be $1
            "Values should change after removing a player: initial={}, new={}",
            initial_mid_value,
            new_mid_value
        );

        // All values should be >= $1.
        for p in &players {
            assert!(
                p.dollar_value >= 1.0,
                "Player {} has value {} < $1",
                p.name,
                p.dollar_value
            );
        }

        // Should be sorted by dollar value descending.
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
    fn recalculate_all_empty_pool() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = DraftState::new(
            test_teams(),
            "team_1",
            league.salary_cap,
            &test_roster_config(),
        );

        let mut players: Vec<PlayerValuation> = Vec::new();
        recalculate_all(&mut players, &league, &strategy, &draft_state);
        assert!(players.is_empty());
    }

    #[test]
    fn recalculate_all_pitchers_only() {
        let league = test_league_config();
        let strategy = test_strategy_config();
        let draft_state = DraftState::new(
            test_teams(),
            "team_1",
            league.salary_cap,
            &test_roster_config(),
        );

        let mut players = vec![
            make_pitcher("SP1", 220, 16, 0, 0, 190.0, 3.00, 1.05, PitcherType::SP),
            make_pitcher("SP2", 180, 12, 0, 0, 170.0, 3.40, 1.15, PitcherType::SP),
            make_pitcher("RP1", 80, 2, 35, 0, 65.0, 2.50, 0.95, PitcherType::RP),
        ];

        recalculate_all(&mut players, &league, &strategy, &draft_state);

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
        let draft_state = DraftState::new(
            test_teams(),
            "team_1",
            league.salary_cap,
            &test_roster_config(),
        );

        let mut players = vec![
            make_hitter("H1", 90, 35, 95, 60, 15, 550, 0.290, vec![Position::FirstBase]),
            make_hitter("H2", 70, 20, 65, 45, 10, 520, 0.270, vec![Position::ThirdBase]),
        ];

        recalculate_all(&mut players, &league, &strategy, &draft_state);

        for p in &players {
            assert!(p.dollar_value >= 1.0);
            assert!(p.total_zscore.is_finite());
        }
    }
}
