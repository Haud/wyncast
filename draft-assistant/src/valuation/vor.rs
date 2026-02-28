// Value Over Replacement (VOR) positional adjustment.
//
// Adjusts raw z-score totals by subtracting the replacement level for each
// position, producing a single "value over replacement" number that accounts
// for positional scarcity.

use std::collections::HashMap;

use crate::config::LeagueConfig;
use crate::draft::pick::Position;
use crate::valuation::projections::PitcherType;
use crate::valuation::zscore::PlayerValuation;

// ---------------------------------------------------------------------------
// Roster key -> Position mapping
// ---------------------------------------------------------------------------

/// Map a roster config key (e.g. "C", "1B", "SP") to the corresponding
/// `Position` enum variant. Returns `None` for keys that do not represent
/// draftable starter positions (e.g. "BE", "IL").
fn roster_key_to_position(key: &str) -> Option<Position> {
    Position::from_str_pos(key)
}

/// Positions that represent dedicated hitter roster slots (excluding UTIL).
const HITTER_POSITION_SLOTS: &[Position] = &[
    Position::Catcher,
    Position::FirstBase,
    Position::SecondBase,
    Position::ThirdBase,
    Position::ShortStop,
    Position::LeftField,
    Position::CenterField,
    Position::RightField,
];

// ---------------------------------------------------------------------------
// Replacement level computation
// ---------------------------------------------------------------------------

/// Determine the replacement-level z-score for every relevant position.
///
/// Algorithm:
/// 1. For each dedicated hitter position, find the (N+1)th best player at that
///    position, where N = slots_per_team * num_teams.
/// 2. Fill UTIL slots with the best remaining hitters not already slotted into
///    a dedicated position.
/// 3. The overall hitter replacement level is the z-score of the first hitter
///    who misses out on all slots (dedicated + UTIL).
/// 4. For each hitter position, replacement = max(position_specific, overall_hitter).
/// 5. SP and RP have independent replacement levels computed from their own pools.
pub fn determine_replacement_levels(
    players: &[PlayerValuation],
    league: &LeagueConfig,
) -> HashMap<Position, f64> {
    let num_teams = league.num_teams;
    let mut replacement_levels: HashMap<Position, f64> = HashMap::new();

    // ---- Hitter replacement levels ----

    // Determine how many starters exist per position from the roster config.
    let mut position_slots: HashMap<Position, usize> = HashMap::new();
    let mut util_slots: usize = 0;

    for (key, &count) in &league.roster {
        if let Some(pos) = roster_key_to_position(key) {
            if pos == Position::Utility {
                util_slots = count;
            } else if HITTER_POSITION_SLOTS.contains(&pos) {
                position_slots.insert(pos, count);
            }
        }
    }

    // Collect all hitters, sorted descending by total_zscore.
    let mut hitters: Vec<&PlayerValuation> = players
        .iter()
        .filter(|p| !p.is_pitcher)
        .collect();
    hitters.sort_by(|a, b| {
        b.total_zscore
            .partial_cmp(&a.total_zscore)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // For each dedicated hitter position, sort eligible players by zscore and
    // find the replacement level (N*num_teams + 1)th player.
    for &pos in HITTER_POSITION_SLOTS {
        let slots = position_slots.get(&pos).copied().unwrap_or(0);
        if slots == 0 {
            continue;
        }
        let total_starters = slots * num_teams;

        // Find all players eligible at this position.
        let mut eligible: Vec<f64> = players
            .iter()
            .filter(|p| !p.is_pitcher && p.positions.contains(&pos))
            .map(|p| p.total_zscore)
            .collect();
        eligible.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

        // Replacement level = the (total_starters)th index (0-based), which is
        // the (total_starters + 1)th player.
        let repl = if eligible.len() > total_starters {
            eligible[total_starters]
        } else if let Some(&last) = eligible.last() {
            // Not enough players to fill all slots; use a very low sentinel.
            last - 1.0
        } else {
            // No eligible players at all.
            f64::NEG_INFINITY
        };

        replacement_levels.insert(pos, repl);
    }

    // Overall hitter replacement level with UTIL consideration.
    // Total hitter starters = sum of all dedicated slots + UTIL slots, all * num_teams.
    let dedicated_per_team: usize = position_slots.values().sum();
    let total_hitter_starters = (dedicated_per_team + util_slots) * num_teams;

    let overall_hitter_repl = if hitters.len() > total_hitter_starters {
        hitters[total_hitter_starters].total_zscore
    } else if let Some(last) = hitters.last() {
        last.total_zscore - 1.0
    } else {
        f64::NEG_INFINITY
    };

    // For each hitter position, take the max of the position-specific
    // replacement and the overall hitter replacement.
    for &pos in HITTER_POSITION_SLOTS {
        if let Some(pos_repl) = replacement_levels.get(&pos).copied() {
            let effective = pos_repl.max(overall_hitter_repl);
            replacement_levels.insert(pos, effective);
        }
    }

    // Store the overall hitter replacement under the Utility position key,
    // so that hitters with no specific position still get a baseline.
    replacement_levels.insert(Position::Utility, overall_hitter_repl);

    // ---- Pitcher replacement levels ----

    let sp_slots = league.roster.get("SP").copied().unwrap_or(0);
    let rp_slots = league.roster.get("RP").copied().unwrap_or(0);

    // SP replacement level
    let mut sp_zscores: Vec<f64> = players
        .iter()
        .filter(|p| p.pitcher_type == Some(PitcherType::SP))
        .map(|p| p.total_zscore)
        .collect();
    sp_zscores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let sp_starters = sp_slots * num_teams;
    let sp_repl = if sp_zscores.len() > sp_starters {
        sp_zscores[sp_starters]
    } else if let Some(&last) = sp_zscores.last() {
        last - 1.0
    } else {
        f64::NEG_INFINITY
    };
    replacement_levels.insert(Position::StartingPitcher, sp_repl);

    // RP replacement level
    let mut rp_zscores: Vec<f64> = players
        .iter()
        .filter(|p| p.pitcher_type == Some(PitcherType::RP))
        .map(|p| p.total_zscore)
        .collect();
    rp_zscores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let rp_starters = rp_slots * num_teams;
    let rp_repl = if rp_zscores.len() > rp_starters {
        rp_zscores[rp_starters]
    } else if let Some(&last) = rp_zscores.last() {
        last - 1.0
    } else {
        f64::NEG_INFINITY
    };
    replacement_levels.insert(Position::ReliefPitcher, rp_repl);

    replacement_levels
}

// ---------------------------------------------------------------------------
// Per-player VOR computation
// ---------------------------------------------------------------------------

/// Compute VOR for a single player and set `player.vor` and `player.best_position`.
///
/// For multi-position hitters, VOR is calculated at each eligible position and
/// the position yielding the highest VOR (zscore - replacement) is chosen.
///
/// Pitchers use their pitcher type (SP or RP) as the single relevant position.
pub fn compute_vor(
    player: &mut PlayerValuation,
    replacement_levels: &HashMap<Position, f64>,
) {
    if player.is_pitcher {
        // Pitchers have exactly one position: SP or RP.
        let pos = match player.pitcher_type {
            Some(PitcherType::SP) => Position::StartingPitcher,
            Some(PitcherType::RP) => Position::ReliefPitcher,
            None => {
                // Shouldn't happen, but handle gracefully.
                player.vor = player.total_zscore;
                return;
            }
        };
        let repl = replacement_levels.get(&pos).copied().unwrap_or(0.0);
        player.vor = player.total_zscore - repl;
        player.best_position = Some(pos);
    } else {
        // Hitter: evaluate each eligible position and keep the best VOR.
        let mut best_vor = f64::NEG_INFINITY;
        let mut best_pos: Option<Position> = None;

        // If the player has explicit position data, use it. Otherwise, try
        // all hitter positions so that players without ESPN position overlay
        // still get a meaningful positional assignment and VOR.
        let candidate_positions: &[Position] = if player.positions.is_empty() {
            HITTER_POSITION_SLOTS
        } else {
            &player.positions
        };

        for &pos in candidate_positions {
            // Skip non-starter positions (Bench, IL, DH, Utility as a "position").
            // Only consider positions that have a replacement level entry.
            if let Some(&repl) = replacement_levels.get(&pos) {
                let vor = player.total_zscore - repl;
                if vor > best_vor {
                    best_vor = vor;
                    best_pos = Some(pos);
                }
            }
        }

        if best_pos.is_some() {
            player.vor = best_vor;
            player.best_position = best_pos;
        } else {
            // Player has no recognized positions with replacement levels.
            // Fall back to overall hitter replacement (UTIL).
            let repl = replacement_levels
                .get(&Position::Utility)
                .copied()
                .unwrap_or(0.0);
            player.vor = player.total_zscore - repl;
            player.best_position = Some(Position::Utility);
        }
    }
}

// ---------------------------------------------------------------------------
// Pipeline entry point
// ---------------------------------------------------------------------------

/// Apply VOR adjustment to all players.
///
/// 1. Compute positional replacement levels from the current player pool.
/// 2. Compute VOR for each player (setting `vor` and `best_position`).
/// 3. Sort players descending by VOR.
pub fn apply_vor(players: &mut Vec<PlayerValuation>, league: &LeagueConfig) {
    let replacement_levels = determine_replacement_levels(players, league);

    for player in players.iter_mut() {
        compute_vor(player, &replacement_levels);
    }

    // Backfill positions for players that lack ESPN position data.
    // After VOR computation, best_position is set for every player.
    // For players with empty positions (no ESPN overlay yet), populate
    // positions from best_position so downstream consumers like
    // compute_scarcity() can find them.
    for player in players.iter_mut() {
        if player.positions.is_empty() {
            if let Some(pos) = player.best_position {
                if !pos.is_meta_slot() {
                    player.positions = vec![pos];
                }
            }
        }
    }

    // Sort descending by VOR.
    players.sort_by(|a, b| {
        b.vor
            .partial_cmp(&a.vor)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::valuation::projections::PitcherType;
    use crate::valuation::zscore::{
        CategoryZScores, HitterZScores, PitcherZScores, PlayerProjectionData,
    };

    // ---- Test helpers ----

    fn approx_eq(a: f64, b: f64, epsilon: f64) -> bool {
        (a - b).abs() < epsilon
    }

    /// Build a minimal LeagueConfig for VOR testing.
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
                categories: vec!["R".into(), "HR".into(), "RBI".into(), "BB".into(), "SB".into(), "AVG".into()],
            },
            pitching_categories: CategoriesSection {
                categories: vec!["K".into(), "W".into(), "SV".into(), "HD".into(), "ERA".into(), "WHIP".into()],
            },
            roster,
            roster_limits: RosterLimits {
                max_sp: 7,
                max_rp: 7,
                gs_per_week: 7,
            },
            teams: HashMap::new(),
            my_team: None,
        }
    }

    fn default_hitter_zscores(total: f64) -> CategoryZScores {
        CategoryZScores::Hitter(HitterZScores {
            r: 0.0,
            hr: 0.0,
            rbi: 0.0,
            bb: 0.0,
            sb: 0.0,
            avg: 0.0,
            total,
        })
    }

    fn default_pitcher_zscores(total: f64) -> CategoryZScores {
        CategoryZScores::Pitcher(PitcherZScores {
            k: 0.0,
            w: 0.0,
            sv: 0.0,
            hd: 0.0,
            era: 0.0,
            whip: 0.0,
            total,
        })
    }

    fn make_hitter_valuation(
        name: &str,
        total_zscore: f64,
        positions: Vec<Position>,
    ) -> PlayerValuation {
        PlayerValuation {
            name: name.into(),
            team: "TST".into(),
            positions,
            is_pitcher: false,
            pitcher_type: None,
            projection: PlayerProjectionData::Hitter {
                pa: 600,
                ab: 550,
                h: 150,
                hr: 25,
                r: 80,
                rbi: 85,
                bb: 50,
                sb: 10,
                avg: 0.273,
            },
            total_zscore,
            category_zscores: default_hitter_zscores(total_zscore),
            vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
        }
    }

    fn make_pitcher_valuation(
        name: &str,
        total_zscore: f64,
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
                ip: 180.0,
                k: 200,
                w: 14,
                sv: 0,
                hd: 0,
                era: 3.20,
                whip: 1.10,
                g: 30,
                gs: 30,
            },
            total_zscore,
            category_zscores: default_pitcher_zscores(total_zscore),
            vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
        }
    }

    // ---- Replacement level tests ----

    #[test]
    fn replacement_levels_basic() {
        // 2-team league, 1 C slot each = 2 C starters, replacement = 3rd best C.
        let league = test_league_config();

        // Create 5 catchers with decreasing z-scores.
        let mut players: Vec<PlayerValuation> = (0..5)
            .map(|i| {
                make_hitter_valuation(
                    &format!("C{}", i + 1),
                    10.0 - (i as f64) * 2.0, // 10, 8, 6, 4, 2
                    vec![Position::Catcher],
                )
            })
            .collect();

        // Add enough other hitters to fill all other positions and UTIL.
        // We need: 1B(2), 2B(2), 3B(2), SS(2), LF(2), CF(2), RF(2), UTIL(2) = 16 more
        for i in 0..20 {
            players.push(make_hitter_valuation(
                &format!("1B_{}", i + 1),
                20.0 - (i as f64) * 0.5,
                vec![Position::FirstBase],
            ));
        }

        let levels = determine_replacement_levels(&players, &league);

        // C: 2 starters -> replacement is 3rd best = index 2 = zscore 6.0
        let c_repl = levels[&Position::Catcher];
        // But we also need to check overall hitter replacement.
        // Total dedicated slots: C(1)+1B(1)+2B(1)+3B(1)+SS(1)+LF(1)+CF(1)+RF(1) = 8 per team
        // UTIL = 1 per team
        // Total hitter starters = (8+1)*2 = 18
        // We have 5 catchers + 20 1B = 25 hitters. 19th hitter (index 18) is the replacement.
        // Sorted by zscore: 1B hitters have 20,19.5,...,10.5 and catchers have 10,8,6,4,2
        // So sorted: 20, 19.5, 19, 18.5, 18, 17.5, 17, 16.5, 16, 15.5, 15, 14.5, 14, 13.5, 13, 12.5, 12, 11.5, 11, 10.5, 10, 8, 6, 4, 2
        // Index 18 = 11.0
        // C position replacement = max(6.0, 11.0) = 11.0

        // Actually, let me re-verify. The overall hitter repl is the (total_hitter_starters)th index.
        // total_hitter_starters = 18. Index 18 (0-based) = 19th player = zscore 11.0
        assert!(
            approx_eq(c_repl, 11.0, 0.01),
            "C replacement should be 11.0 (overall hitter repl dominates), got {}",
            c_repl
        );
    }

    #[test]
    fn replacement_levels_position_specific_dominates() {
        // When a position is scarce, its replacement level should be LOWER than
        // the overall hitter replacement. But since we take max(), the overall
        // replacement will still dominate IF it's higher.
        //
        // To make position-specific dominate, we need a position where there
        // are many eligible players with relatively high z-scores but few slots.
        //
        // Actually, position-specific dominates when there are FEW eligible
        // players, causing the replacement level to be LOW. max() means the
        // overall hitter replacement (higher) would dominate.
        //
        // The only way position-specific > overall is if there are many
        // eligible players at a position, making its replacement level HIGH
        // (higher than the overall hitter replacement).

        let mut league = test_league_config();
        league.num_teams = 2; // 2 teams, 1 SS slot each = 2 SS starters

        // Create a pool where SS has many good players (high replacement level)
        // but the overall pool is mediocre.
        let mut players = Vec::new();

        // 5 shortstops, all very good
        for i in 0..5 {
            players.push(make_hitter_valuation(
                &format!("SS_{}", i + 1),
                15.0 - (i as f64), // 15, 14, 13, 12, 11
                vec![Position::ShortStop],
            ));
        }

        // Fill other positions with mediocre players
        for pos in &[
            Position::Catcher,
            Position::FirstBase,
            Position::SecondBase,
            Position::ThirdBase,
            Position::LeftField,
            Position::CenterField,
            Position::RightField,
        ] {
            for i in 0..5 {
                players.push(make_hitter_valuation(
                    &format!("{}_{}", pos.display_str(), i + 1),
                    5.0 - (i as f64), // 5, 4, 3, 2, 1
                    vec![*pos],
                ));
            }
        }

        let levels = determine_replacement_levels(&players, &league);

        // SS: 2 starters -> replacement = 3rd best SS = z 13.0
        // Overall: (8+1)*2 = 18 starters. We have 5+7*5 = 40 players.
        // Sorted: 15,14,13,12,11, then 7 groups of (5,4,3,2,1)
        // Index 18 = 19th player.
        // Let me count: 15,14,13,12,11(5), then 5,5,5,5,5,5,5(7 at z=5),
        //   4,4,4,4,4,4,4(7 at z=4), ...
        // Sorted descending: 15,14,13,12,11, 5,5,5,5,5,5,5, 4,4,4,4,4,4,4, 3,...
        // Index: 0=15, 1=14, 2=13, 3=12, 4=11, 5-11=5, 12-18=4
        // Index 18 = 4.0
        // SS replacement = max(13.0, 4.0) = 13.0
        assert!(
            approx_eq(levels[&Position::ShortStop], 13.0, 0.01),
            "SS replacement should be 13.0 (position-specific dominates), got {}",
            levels[&Position::ShortStop]
        );
    }

    #[test]
    fn multi_position_vor_picks_better_position() {
        let league = test_league_config();

        let mut replacement_levels = HashMap::new();
        // 2B has high replacement (scarce position) -> lower VOR
        replacement_levels.insert(Position::SecondBase, 8.0);
        // SS has low replacement (deep position) -> higher VOR
        replacement_levels.insert(Position::ShortStop, 3.0);
        replacement_levels.insert(Position::Utility, 2.0);

        let mut player = make_hitter_valuation(
            "Multi-Position Guy",
            10.0,
            vec![Position::SecondBase, Position::ShortStop],
        );

        compute_vor(&mut player, &replacement_levels);

        // VOR at 2B = 10.0 - 8.0 = 2.0
        // VOR at SS = 10.0 - 3.0 = 7.0
        // Should pick SS (higher VOR).
        assert_eq!(player.best_position, Some(Position::ShortStop));
        assert!(
            approx_eq(player.vor, 7.0, 1e-10),
            "VOR should be 7.0 (SS), got {}",
            player.vor
        );
    }

    #[test]
    fn util_handling_overall_hitter_replacement() {
        // Test that UTIL slots affect the overall hitter replacement level.
        let mut league = test_league_config();
        league.num_teams = 2;
        // With UTIL=1: total hitter starters = (8+1)*2 = 18
        // Without UTIL: total hitter starters = 8*2 = 16

        // Create exactly 20 hitters (each at a unique position for simplicity).
        let mut players = Vec::new();
        for i in 0..20 {
            // Cycle through positions
            let pos = match i % 8 {
                0 => Position::Catcher,
                1 => Position::FirstBase,
                2 => Position::SecondBase,
                3 => Position::ThirdBase,
                4 => Position::ShortStop,
                5 => Position::LeftField,
                6 => Position::CenterField,
                7 => Position::RightField,
                _ => unreachable!(),
            };
            players.push(make_hitter_valuation(
                &format!("Hitter_{}", i + 1),
                20.0 - (i as f64), // 20, 19, 18, ..., 1
                vec![pos],
            ));
        }

        let levels = determine_replacement_levels(&players, &league);

        // Total hitter starters with UTIL = (8+1)*2 = 18.
        // Overall hitter replacement = player at index 18 (0-based) = 19th player = zscore 2.0
        let util_repl = levels[&Position::Utility];
        assert!(
            approx_eq(util_repl, 2.0, 0.01),
            "Overall hitter replacement should be 2.0, got {}",
            util_repl
        );
    }

    #[test]
    fn pitcher_separate_replacement_levels() {
        let league = test_league_config();
        // SP=5, RP=6, num_teams=2 => SP starters=10, RP starters=12

        let mut players = Vec::new();

        // 15 SPs with decreasing z-scores.
        for i in 0..15 {
            players.push(make_pitcher_valuation(
                &format!("SP_{}", i + 1),
                10.0 - (i as f64) * 0.5, // 10.0, 9.5, 9.0, ..., 3.0
                PitcherType::SP,
            ));
        }

        // 15 RPs with decreasing z-scores.
        for i in 0..15 {
            players.push(make_pitcher_valuation(
                &format!("RP_{}", i + 1),
                8.0 - (i as f64) * 0.5, // 8.0, 7.5, 7.0, ..., 1.0
                PitcherType::RP,
            ));
        }

        let levels = determine_replacement_levels(&players, &league);

        // SP: 10 starters -> replacement = index 10 = 10.0 - 10*0.5 = 5.0
        assert!(
            approx_eq(levels[&Position::StartingPitcher], 5.0, 0.01),
            "SP replacement should be 5.0, got {}",
            levels[&Position::StartingPitcher]
        );

        // RP: 12 starters -> replacement = index 12 = 8.0 - 12*0.5 = 2.0
        assert!(
            approx_eq(levels[&Position::ReliefPitcher], 2.0, 0.01),
            "RP replacement should be 2.0, got {}",
            levels[&Position::ReliefPitcher]
        );
    }

    #[test]
    fn pitchers_dont_interact_with_util() {
        // Pitchers should not affect the hitter replacement levels
        // and should not be counted in UTIL slots.
        let mut league = test_league_config();
        league.num_teams = 1;

        let mut players = Vec::new();

        // 12 hitters
        for i in 0..12 {
            let pos = match i % 8 {
                0 => Position::Catcher,
                1 => Position::FirstBase,
                2 => Position::SecondBase,
                3 => Position::ThirdBase,
                4 => Position::ShortStop,
                5 => Position::LeftField,
                6 => Position::CenterField,
                7 => Position::RightField,
                _ => unreachable!(),
            };
            players.push(make_hitter_valuation(
                &format!("H_{}", i + 1),
                12.0 - (i as f64),
                vec![pos],
            ));
        }

        // 10 SPs with high z-scores that should NOT fill UTIL.
        for i in 0..10 {
            players.push(make_pitcher_valuation(
                &format!("SP_{}", i + 1),
                20.0 - (i as f64),
                PitcherType::SP,
            ));
        }

        let levels = determine_replacement_levels(&players, &league);

        // 1 team: hitter starters = (8+1)*1 = 9. 12 hitters total.
        // Overall hitter repl = index 9 = 12.0 - 9.0 = 3.0
        let util_repl = levels[&Position::Utility];
        assert!(
            approx_eq(util_repl, 3.0, 0.01),
            "Hitter replacement should ignore pitchers, expected 3.0, got {}",
            util_repl
        );
    }

    #[test]
    fn negative_vor_below_replacement() {
        let mut replacement_levels = HashMap::new();
        replacement_levels.insert(Position::Catcher, 5.0);
        replacement_levels.insert(Position::Utility, 3.0);

        let mut player = make_hitter_valuation(
            "Bad Catcher",
            2.0, // below both replacement levels
            vec![Position::Catcher],
        );

        compute_vor(&mut player, &replacement_levels);

        // VOR at C = 2.0 - 5.0 = -3.0
        assert_eq!(player.best_position, Some(Position::Catcher));
        assert!(
            approx_eq(player.vor, -3.0, 1e-10),
            "VOR should be -3.0 (below replacement), got {}",
            player.vor
        );
    }

    #[test]
    fn apply_vor_sorts_by_vor() {
        let mut league = test_league_config();
        league.num_teams = 1;

        let mut players = vec![
            make_hitter_valuation("Low Z", 2.0, vec![Position::Catcher]),
            make_hitter_valuation("High Z", 10.0, vec![Position::Catcher]),
            make_hitter_valuation("Mid Z", 5.0, vec![Position::Catcher]),
        ];

        // Add enough other position players so there are replacement levels.
        for pos in &[
            Position::FirstBase,
            Position::SecondBase,
            Position::ThirdBase,
            Position::ShortStop,
            Position::LeftField,
            Position::CenterField,
            Position::RightField,
        ] {
            for i in 0..3 {
                players.push(make_hitter_valuation(
                    &format!("{}_{}", pos.display_str(), i + 1),
                    6.0 - (i as f64),
                    vec![*pos],
                ));
            }
        }

        apply_vor(&mut players, &league);

        // After sorting by VOR, the first player should be "High Z".
        assert_eq!(players[0].name, "High Z");
        // All players should have best_position set.
        for player in &players {
            assert!(
                player.best_position.is_some(),
                "Player {} should have best_position set",
                player.name
            );
        }
    }

    #[test]
    fn pitcher_vor_computation() {
        let mut replacement_levels = HashMap::new();
        replacement_levels.insert(Position::StartingPitcher, 4.0);
        replacement_levels.insert(Position::ReliefPitcher, 2.0);

        let mut sp = make_pitcher_valuation("Ace SP", 8.0, PitcherType::SP);
        compute_vor(&mut sp, &replacement_levels);
        assert_eq!(sp.best_position, Some(Position::StartingPitcher));
        assert!(
            approx_eq(sp.vor, 4.0, 1e-10),
            "SP VOR = 8.0 - 4.0 = 4.0, got {}",
            sp.vor
        );

        let mut rp = make_pitcher_valuation("Closer RP", 5.0, PitcherType::RP);
        compute_vor(&mut rp, &replacement_levels);
        assert_eq!(rp.best_position, Some(Position::ReliefPitcher));
        assert!(
            approx_eq(rp.vor, 3.0, 1e-10),
            "RP VOR = 5.0 - 2.0 = 3.0, got {}",
            rp.vor
        );
    }

    #[test]
    fn hitter_no_positions_tries_all_hitter_positions() {
        let mut replacement_levels = HashMap::new();
        replacement_levels.insert(Position::Utility, 3.0);
        replacement_levels.insert(Position::Catcher, 5.0);

        // Player with empty positions list — should try all hitter
        // positions and pick the one with the highest VOR.
        let mut player = make_hitter_valuation("DH Only", 7.0, vec![]);

        compute_vor(&mut player, &replacement_levels);

        // Only Catcher has a replacement level in hitter positions,
        // so VOR = 7.0 - 5.0 = 2.0 at Catcher.
        assert_eq!(player.best_position, Some(Position::Catcher));
        assert!(
            approx_eq(player.vor, 2.0, 1e-10),
            "VOR should be 7.0 - 5.0 = 2.0, got {}",
            player.vor
        );
    }

    #[test]
    fn hitter_no_positions_no_replacement_levels_falls_back_to_util() {
        let mut replacement_levels = HashMap::new();
        replacement_levels.insert(Position::Utility, 3.0);
        // No hitter position replacement levels.

        let mut player = make_hitter_valuation("DH Only", 7.0, vec![]);

        compute_vor(&mut player, &replacement_levels);

        // No hitter positions have replacement levels, so fall back to UTIL.
        assert_eq!(player.best_position, Some(Position::Utility));
        assert!(
            approx_eq(player.vor, 4.0, 1e-10),
            "VOR should be 7.0 - 3.0 = 4.0, got {}",
            player.vor
        );
    }

    #[test]
    fn apply_vor_end_to_end() {
        // Full end-to-end test with mixed hitters and pitchers.
        let mut league = test_league_config();
        league.num_teams = 1;

        let mut players = Vec::new();

        // One player per hitter position, plus extras for replacement.
        for (i, pos) in [
            Position::Catcher,
            Position::FirstBase,
            Position::SecondBase,
            Position::ThirdBase,
            Position::ShortStop,
            Position::LeftField,
            Position::CenterField,
            Position::RightField,
        ]
        .iter()
        .enumerate()
        {
            // Starter quality
            players.push(make_hitter_valuation(
                &format!("{}_starter", pos.display_str()),
                10.0 - (i as f64) * 0.5,
                vec![*pos],
            ));
            // Backup quality
            players.push(make_hitter_valuation(
                &format!("{}_backup", pos.display_str()),
                3.0 - (i as f64) * 0.2,
                vec![*pos],
            ));
        }

        // A few SPs
        for i in 0..8 {
            players.push(make_pitcher_valuation(
                &format!("SP_{}", i + 1),
                8.0 - (i as f64),
                PitcherType::SP,
            ));
        }

        // A few RPs
        for i in 0..8 {
            players.push(make_pitcher_valuation(
                &format!("RP_{}", i + 1),
                6.0 - (i as f64),
                PitcherType::RP,
            ));
        }

        apply_vor(&mut players, &league);

        // Verify sorted descending by VOR.
        for i in 1..players.len() {
            assert!(
                players[i - 1].vor >= players[i].vor || (players[i-1].vor.is_nan() || players[i].vor.is_nan()),
                "Players should be sorted descending by VOR: {} ({}) >= {} ({})",
                players[i - 1].name,
                players[i - 1].vor,
                players[i].name,
                players[i].vor
            );
        }

        // Verify every player has a best_position.
        for player in &players {
            assert!(
                player.best_position.is_some(),
                "Player {} should have best_position set",
                player.name
            );
        }
    }

    #[test]
    fn multi_position_prefers_scarcer_position() {
        // A "2B,SS" player should get the position with the LOWER replacement
        // level, giving HIGHER VOR.
        let mut league = test_league_config();
        league.num_teams = 1;

        let mut players = Vec::new();

        // The multi-position player.
        players.push(make_hitter_valuation(
            "Versatile Guy",
            8.0,
            vec![Position::SecondBase, Position::ShortStop],
        ));

        // 2B has many options (high replacement level).
        for i in 0..5 {
            players.push(make_hitter_valuation(
                &format!("2B_{}", i + 1),
                9.0 - (i as f64), // 9, 8, 7, 6, 5
                vec![Position::SecondBase],
            ));
        }

        // SS has few options (low replacement level).
        players.push(make_hitter_valuation(
            "SS_1",
            7.0,
            vec![Position::ShortStop],
        ));

        // Fill remaining positions.
        for pos in &[
            Position::Catcher,
            Position::FirstBase,
            Position::ThirdBase,
            Position::LeftField,
            Position::CenterField,
            Position::RightField,
        ] {
            for i in 0..3 {
                players.push(make_hitter_valuation(
                    &format!("{}_{}", pos.display_str(), i + 1),
                    6.0 - (i as f64),
                    vec![*pos],
                ));
            }
        }

        apply_vor(&mut players, &league);

        // Find our multi-position player.
        let versatile = players.iter().find(|p| p.name == "Versatile Guy").unwrap();

        // SS should have a lower replacement level than 2B, so the player
        // should be assigned SS for higher VOR.
        // 2B: 1 slot, 1 team = 1 starter. 6 eligible (5 pure + 1 multi).
        //   Position replacement = index 1 = 8.0 (but multi is at 8.0 too, so sorted: 9,8,8,7,6,5 -> index 1 = 8.0)
        // SS: 1 slot, 1 team = 1 starter. 2 eligible (1 pure + 1 multi).
        //   Position replacement = index 1 = 7.0 (sorted: 8.0, 7.0 -> index 1 = 7.0)
        // Overall hitter: 9 starters (8 dedicated + 1 UTIL).
        // VOR at 2B = 8.0 - max(8.0, overall) vs VOR at SS = 8.0 - max(7.0, overall)
        // If overall < 7.0, then SS wins: VOR_SS = 8.0 - 7.0 = 1.0 vs VOR_2B = 8.0 - 8.0 = 0.0
        // So SS should be the best position.

        assert_eq!(
            versatile.best_position,
            Some(Position::ShortStop),
            "Multi-position player should be assigned SS (lower replacement level), got {:?}",
            versatile.best_position
        );
    }

    #[test]
    fn empty_player_pool() {
        let league = test_league_config();
        let players: Vec<PlayerValuation> = Vec::new();

        let levels = determine_replacement_levels(&players, &league);

        // All replacement levels should be NEG_INFINITY or simply not present
        // for positions with no eligible players.
        assert!(
            levels.get(&Position::Catcher).copied().unwrap_or(f64::NEG_INFINITY) <= f64::NEG_INFINITY,
            "C replacement should be NEG_INFINITY for empty pool"
        );
    }

    #[test]
    fn too_few_players_for_slots() {
        // When there aren't enough players to fill all slots, the replacement
        // level should be below the worst player.
        let mut league = test_league_config();
        league.num_teams = 2; // 2 C slots needed

        let players = vec![make_hitter_valuation(
            "Only Catcher",
            5.0,
            vec![Position::Catcher],
        )];

        let levels = determine_replacement_levels(&players, &league);

        // C: 2 starters needed, only 1 available -> replacement = 5.0 - 1.0 = 4.0
        // But overall hitter replacement comes into play too.
        // Only 1 hitter total, 18 starters needed -> repl = 5.0 - 1.0 = 4.0
        // max(4.0, 4.0) = 4.0
        let c_repl = levels[&Position::Catcher];
        assert!(
            approx_eq(c_repl, 4.0, 0.01),
            "C replacement with too few players should be 4.0, got {}",
            c_repl
        );
    }
}
