// Instant analysis engine for real-time player evaluation during draft.
//
// Combines inflation-adjusted dollar values, positional scarcity, roster
// needs, and category impact into a single actionable verdict for each
// nominated player.

use crate::draft::pick::Position;
use crate::draft::roster::Roster;
use crate::stats::{CategoryValues, StatRegistry};
use crate::valuation::auction::InflationTracker;
use crate::valuation::scarcity::{ScarcityEntry, ScarcityUrgency, scarcity_for_position};
use crate::valuation::zscore::PlayerValuation;

// ---------------------------------------------------------------------------
// Instant verdict
// ---------------------------------------------------------------------------

/// High-level draft verdict for a nominated player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantVerdict {
    /// Strongly recommend targeting this player.
    StrongTarget,
    /// Player is useful but not urgent; target conditionally.
    ConditionalTarget,
    /// Player does not fill a pressing need; pass.
    Pass,
}

impl InstantVerdict {
    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            InstantVerdict::StrongTarget => "STRONG TARGET",
            InstantVerdict::ConditionalTarget => "CONDITIONAL",
            InstantVerdict::Pass => "PASS",
        }
    }
}

// ---------------------------------------------------------------------------
// Similar player
// ---------------------------------------------------------------------------

/// A comparable player available later in the draft.
#[derive(Debug, Clone)]
pub struct SimilarPlayer {
    pub name: String,
    pub position: String,
    pub dollar_value: f64,
    pub vor: f64,
    pub key_difference: String,
}

// ---------------------------------------------------------------------------
// Instant analysis result
// ---------------------------------------------------------------------------

/// Complete instant analysis for a single player nomination.
#[derive(Debug, Clone)]
pub struct InstantAnalysis {
    /// The player being analyzed.
    pub player_name: String,
    /// Pre-draft dollar value.
    pub dollar_value: f64,
    /// Inflation-adjusted dollar value.
    pub adjusted_value: f64,
    /// Value Over Replacement.
    pub vor: f64,
    /// Whether this player fills an empty dedicated roster slot.
    pub fills_empty_slot: bool,
    /// The position this player would fill, if applicable.
    pub fills_position: Option<Position>,
    /// Scarcity urgency at the player's best position.
    pub scarcity_at_position: ScarcityUrgency,
    /// Top 3 category impacts: (category_name, need_weighted_zscore).
    pub category_impact: Vec<(String, f64)>,
    /// Minimum recommended bid (70% of adjusted value).
    pub bid_floor: u32,
    /// Maximum recommended bid (adjusted value + scarcity premium).
    pub bid_ceiling: u32,
    /// Overall verdict.
    pub verdict: InstantVerdict,
    /// 2-3 similar available players for comparison.
    pub similar_players: Vec<SimilarPlayer>,
}

// ---------------------------------------------------------------------------
// Core computation
// ---------------------------------------------------------------------------

/// Compute instant analysis for a player being nominated.
///
/// # Arguments
/// - `player` - The nominated player's valuation data.
/// - `my_roster` - The user's current roster state.
/// - `available_players` - All undrafted players.
/// - `scarcity` - Pre-computed scarcity entries.
/// - `inflation` - Current inflation tracker state.
/// - `category_needs` - The user's per-category need levels.
/// - `registry` - Stat registry for category metadata.
pub fn compute_instant_analysis(
    player: &PlayerValuation,
    my_roster: &Roster,
    available_players: &[PlayerValuation],
    scarcity: &[ScarcityEntry],
    inflation: &InflationTracker,
    category_needs: &CategoryValues,
    registry: &StatRegistry,
) -> InstantAnalysis {
    let adjusted_value = inflation.adjust(player.dollar_value);
    let vor = player.vor;

    // Determine which position this player would fill.
    let best_pos = player.best_position.unwrap_or(Position::Utility);

    // Check if the player fills an empty dedicated slot on our roster.
    let fills_empty_slot = player
        .positions
        .iter()
        .any(|pos| my_roster.has_empty_slot(*pos));

    let fills_position = if fills_empty_slot {
        player
            .positions
            .iter()
            .find(|pos| my_roster.has_empty_slot(**pos))
            .copied()
    } else {
        None
    };

    // Look up scarcity at the player's best position.
    let scarcity_at_position = scarcity_for_position(scarcity, best_pos)
        .map(|e| e.urgency)
        .unwrap_or(ScarcityUrgency::Low);

    // Compute category impact: z-score * category need for each category.
    let category_impact = compute_category_impact(player, category_needs, registry);

    // Bid range calculation.
    let bid_floor = (adjusted_value * 0.70).round().max(1.0) as u32;
    let premium = scarcity_at_position.premium();
    let bid_ceiling = (adjusted_value * (1.0 + premium)).round().max(1.0) as u32;

    // Determine verdict.
    let verdict = compute_verdict(
        fills_empty_slot,
        scarcity_at_position,
        player,
        available_players,
        best_pos,
    );

    // Find similar players.
    let similar_players = find_similar_players(player, available_players, best_pos);

    InstantAnalysis {
        player_name: player.name.clone(),
        dollar_value: player.dollar_value,
        adjusted_value,
        vor,
        fills_empty_slot,
        fills_position,
        scarcity_at_position,
        category_impact,
        bid_floor,
        bid_ceiling,
        verdict,
        similar_players,
    }
}

// ---------------------------------------------------------------------------
// Verdict logic
// ---------------------------------------------------------------------------

/// Determine the instant verdict for a player.
///
/// StrongTarget if:
/// - Fills an empty roster slot AND position is High/Critical urgency, OR
/// - Is a top-3 available player at a needed position.
///
/// ConditionalTarget if:
/// - Player is useful but scarcity is Medium/Low.
///
/// Pass if:
/// - Doesn't fill a pressing need at all.
fn compute_verdict(
    fills_empty_slot: bool,
    scarcity: ScarcityUrgency,
    player: &PlayerValuation,
    available_players: &[PlayerValuation],
    best_pos: Position,
) -> InstantVerdict {
    // Check if player is top 3 at position among available.
    let is_top3 = is_top_n_at_position(player, available_players, best_pos, 3);

    if fills_empty_slot
        && matches!(scarcity, ScarcityUrgency::Critical | ScarcityUrgency::High)
    {
        return InstantVerdict::StrongTarget;
    }

    if is_top3 && fills_empty_slot {
        return InstantVerdict::StrongTarget;
    }

    if fills_empty_slot || player.vor > 0.0 {
        return InstantVerdict::ConditionalTarget;
    }

    InstantVerdict::Pass
}

/// Check if a player is among the top N available at a given position.
fn is_top_n_at_position(
    player: &PlayerValuation,
    available_players: &[PlayerValuation],
    position: Position,
    n: usize,
) -> bool {
    let mut eligible_vors: Vec<f64> = available_players
        .iter()
        .filter(|p| p.positions.contains(&position) && p.vor > 0.0)
        .map(|p| p.vor)
        .collect();

    eligible_vors.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    if let Some(&nth_vor) = eligible_vors.get(n.saturating_sub(1)) {
        player.vor >= nth_vor
    } else {
        // Fewer than N players available; this player is automatically top N.
        player.positions.contains(&position) && player.vor > 0.0
    }
}

// ---------------------------------------------------------------------------
// Category impact
// ---------------------------------------------------------------------------

/// Compute per-category impact scores and return top 3.
///
/// For each category, multiply the player's z-score by the category need.
/// Return the top 3 by absolute impact.
fn compute_category_impact(
    player: &PlayerValuation,
    needs: &CategoryValues,
    registry: &StatRegistry,
) -> Vec<(String, f64)> {
    let zscores = player.category_zscores.zscores();
    let mut impacts: Vec<(String, f64)> = registry
        .all_stats()
        .iter()
        .enumerate()
        .filter_map(|(idx, stat)| {
            let z = zscores.get(idx).unwrap_or(0.0);
            let need = needs.get(idx).unwrap_or(0.0);
            let impact = z * need;
            if impact.abs() > 1e-12 {
                Some((stat.abbrev.clone(), impact))
            } else {
                None
            }
        })
        .collect();


    impacts.sort_by(|a, b| {
        b.1.abs()
            .partial_cmp(&a.1.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    impacts.truncate(3);
    impacts
}

// ---------------------------------------------------------------------------
// Similar players
// ---------------------------------------------------------------------------

/// Find 2-3 similar available players at the same position with VOR within 30%.
fn find_similar_players(
    player: &PlayerValuation,
    available_players: &[PlayerValuation],
    position: Position,
) -> Vec<SimilarPlayer> {
    if player.vor <= 0.0 {
        return Vec::new();
    }

    let vor_threshold = player.vor * 0.30;
    let min_vor = player.vor - vor_threshold;
    let max_vor = player.vor + vor_threshold;

    let mut similar: Vec<SimilarPlayer> = available_players
        .iter()
        .filter(|p| {
            p.name != player.name
                && p.positions.contains(&position)
                && p.vor >= min_vor
                && p.vor <= max_vor
                && p.vor > 0.0
        })
        .map(|p| {
            let key_difference = if p.dollar_value > player.dollar_value * 1.1 {
                "More expensive".to_string()
            } else if p.dollar_value < player.dollar_value * 0.9 {
                "Cheaper option".to_string()
            } else if p.vor > player.vor {
                "Higher VOR".to_string()
            } else {
                "Similar value".to_string()
            };

            SimilarPlayer {
                name: p.name.clone(),
                position: position.display_str().to_string(),
                dollar_value: p.dollar_value,
                vor: p.vor,
                key_difference,
            }
        })
        .collect();

    // Sort by VOR descending, take top 3.
    similar.sort_by(|a, b| {
        b.vor
            .partial_cmp(&a.vor)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    similar.truncate(3);

    similar
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::CategoryValues;
    use crate::test_utils::{approx_eq, test_registry, test_roster_config};
    use crate::valuation::auction::InflationTracker;
    use crate::valuation::scarcity::compute_scarcity;
    use crate::valuation::zscore::{CategoryZScores, ProjectionData};
    use std::collections::HashMap;

    fn make_hitter(name: &str, vor: f64, positions: Vec<Position>, dollar: f64) -> PlayerValuation {
        let registry = test_registry();
        let mut zscores = CategoryValues::zeros(registry.len());
        zscores.set(registry.index_of("R").unwrap(), 1.5);
        zscores.set(registry.index_of("HR").unwrap(), 1.2);
        zscores.set(registry.index_of("RBI").unwrap(), 0.8);
        zscores.set(registry.index_of("BB").unwrap(), 2.0);
        zscores.set(registry.index_of("SB").unwrap(), 0.3);
        zscores.set(registry.index_of("AVG").unwrap(), 0.5);
        PlayerValuation {
            name: name.into(),
            team: "TST".into(),
            positions: positions.clone(),
            is_pitcher: false,
            is_two_way: false,
            pitcher_type: None,
            projection: ProjectionData {
                values: HashMap::from([
                    ("pa".into(), 600.0), ("ab".into(), 550.0), ("h".into(), 150.0),
                    ("hr".into(), 25.0), ("r".into(), 80.0), ("rbi".into(), 85.0),
                    ("bb".into(), 50.0), ("sb".into(), 10.0), ("avg".into(), 0.273),
                ]),
            },
            total_zscore: vor + 2.0,
            category_zscores: CategoryZScores::hitter(zscores, vor + 2.0),
            vor,
            initial_vor: vor,
            best_position: positions.first().copied(),
            dollar_value: dollar,
        }
    }

    #[test]
    fn strong_target_fills_critical_position() {
        let registry = test_registry();
        let roster = Roster::new(&test_roster_config()); // Empty roster

        // Only 2 catchers available -> Critical scarcity
        let available = vec![
            make_hitter("Target C", 6.0, vec![Position::Catcher], 30.0),
            make_hitter("Other C", 3.0, vec![Position::Catcher], 15.0),
        ];

        let scarcity = compute_scarcity(&available, &test_roster_config());
        let inflation = InflationTracker::new();
        let needs = CategoryValues::uniform(registry.len(), 0.5);

        let analysis = compute_instant_analysis(
            &available[0],
            &roster,
            &available,
            &scarcity,
            &inflation,
            &needs,
            &registry,
        );

        assert_eq!(analysis.verdict, InstantVerdict::StrongTarget);
        assert!(analysis.fills_empty_slot);
        assert_eq!(analysis.scarcity_at_position, ScarcityUrgency::Critical);
    }

    #[test]
    fn pass_when_no_need() {
        let registry = test_registry();
        let mut roster = Roster::new(&test_roster_config());
        // Fill the catcher slot
        roster.add_player("Existing C", "C", 10, None);
        // Fill UTIL
        roster.add_player("Existing UTIL", "C", 5, None);

        // Player with negative VOR at a filled position
        let player = make_hitter("Bad C", -2.0, vec![Position::Catcher], 1.0);
        let available = vec![player.clone()];

        let scarcity = compute_scarcity(&available, &test_roster_config());
        let inflation = InflationTracker::new();
        let needs = CategoryValues::uniform(registry.len(), 0.5);

        let analysis = compute_instant_analysis(
            &player,
            &roster,
            &available,
            &scarcity,
            &inflation,
            &needs,
            &registry,
        );

        assert_eq!(analysis.verdict, InstantVerdict::Pass);
    }

    #[test]
    fn bid_floor_and_ceiling_known_values() {
        let registry = test_registry();
        let roster = Roster::new(&test_roster_config());

        // Player worth $30. With neutral inflation (1.0):
        // adjusted_value = (30-1)*1.0 + 1.0 = 30.0
        // bid_floor = round(30.0 * 0.70) = 21
        // With Critical scarcity: premium = +30%
        // bid_ceiling = round(30.0 * 1.30) = 39
        let available = vec![
            make_hitter("Star C", 10.0, vec![Position::Catcher], 30.0),
            make_hitter("Other C", 3.0, vec![Position::Catcher], 10.0),
        ];

        let scarcity = compute_scarcity(&available, &test_roster_config());
        let inflation = InflationTracker::new(); // rate = 1.0
        let needs = CategoryValues::uniform(registry.len(), 0.5);

        let analysis = compute_instant_analysis(
            &available[0],
            &roster,
            &available,
            &scarcity,
            &inflation,
            &needs,
            &registry,
        );

        assert_eq!(analysis.bid_floor, 21);
        assert_eq!(analysis.bid_ceiling, 39);
    }

    #[test]
    fn bid_range_with_inflation() {
        let registry = test_registry();
        let roster = Roster::new(&test_roster_config());

        // Player worth $30. With inflation rate 1.1 (deflation):
        // adjusted_value = (30-1)*1.1 + 1.0 = 31.9 + 1.0 = 32.9
        // bid_floor = round(32.9 * 0.70) = 23
        let mut available = Vec::new();
        // Put enough players at different positions so scarcity is Medium
        for i in 0..6 {
            available.push(make_hitter(
                &format!("1B_{}", i),
                10.0 - i as f64,
                vec![Position::FirstBase],
                30.0 - i as f64 * 3.0,
            ));
        }

        let scarcity = compute_scarcity(&available, &test_roster_config());
        let mut inflation = InflationTracker::new();
        inflation.inflation_rate = 1.1;
        let needs = CategoryValues::uniform(registry.len(), 0.5);

        let analysis = compute_instant_analysis(
            &available[0],
            &roster,
            &available,
            &scarcity,
            &inflation,
            &needs,
            &registry,
        );

        // adjusted = (30.0 - 1.0) * 1.1 + 1.0 = 32.9
        assert!(
            approx_eq(analysis.adjusted_value, 32.9, 0.1),
            "adjusted_value should be ~32.9, got {}",
            analysis.adjusted_value
        );
        // bid_floor = round(32.9 * 0.70) = round(23.03) = 23
        assert_eq!(analysis.bid_floor, 23);
    }

    #[test]
    fn category_impact_returns_top_3() {
        let registry = test_registry();
        let player = make_hitter("Test", 5.0, vec![Position::FirstBase], 20.0);
        // Registry order: R, HR, RBI, BB, SB, AVG, K, W, SV, HD, ERA, WHIP
        let needs = CategoryValues::from_vec(vec![
            0.8, 0.5, 0.3, 1.0, 0.1, 0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ]);

        let impact = compute_category_impact(&player, &needs, &registry);
        assert_eq!(impact.len(), 3);

        // BB has highest impact: z=2.0 * need=1.0 = 2.0
        assert_eq!(impact[0].0, "BB");
        assert!(approx_eq(impact[0].1, 2.0, 0.01));
    }

    #[test]
    fn similar_players_found() {
        let target = make_hitter("Target", 5.0, vec![Position::FirstBase], 20.0);
        let available = vec![
            target.clone(),
            make_hitter("Similar1", 4.5, vec![Position::FirstBase], 18.0),
            make_hitter("Similar2", 5.5, vec![Position::FirstBase], 22.0),
            make_hitter("TooFar", 1.0, vec![Position::FirstBase], 5.0),
            make_hitter("WrongPos", 5.0, vec![Position::Catcher], 20.0),
        ];

        let similar = find_similar_players(&target, &available, Position::FirstBase);

        assert_eq!(similar.len(), 2);
        // Should NOT include the target itself or the wrong-position player or too-far player
        let names: Vec<&str> = similar.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Similar1"));
        assert!(names.contains(&"Similar2"));
    }

    #[test]
    fn conditional_target_when_fills_slot_low_scarcity() {
        let registry = test_registry();
        let roster = Roster::new(&test_roster_config()); // Empty roster

        // 10 first basemen -> Low urgency, but roster slot is empty
        let mut available = Vec::new();
        for i in 0..10 {
            available.push(make_hitter(
                &format!("1B_{}", i),
                10.0 - i as f64,
                vec![Position::FirstBase],
                (10.0 - i as f64) * 5.0 + 1.0,
            ));
        }

        let scarcity = compute_scarcity(&available, &test_roster_config());
        let inflation = InflationTracker::new();
        let needs = CategoryValues::uniform(registry.len(), 0.5);

        // Analyze the 5th best (not top 3, but fills empty slot)
        let analysis = compute_instant_analysis(
            &available[4],
            &roster,
            &available,
            &scarcity,
            &inflation,
            &needs,
            &registry,
        );

        // Should be ConditionalTarget (fills slot but Low scarcity and not top 3)
        assert_eq!(analysis.verdict, InstantVerdict::ConditionalTarget);
    }

    #[test]
    fn strong_target_when_top3_and_fills_slot() {
        let registry = test_registry();
        let roster = Roster::new(&test_roster_config()); // Empty roster

        // 10 first basemen -> Low urgency, but player is top 3
        let mut available = Vec::new();
        for i in 0..10 {
            available.push(make_hitter(
                &format!("1B_{}", i),
                10.0 - i as f64,
                vec![Position::FirstBase],
                (10.0 - i as f64) * 5.0 + 1.0,
            ));
        }

        let scarcity = compute_scarcity(&available, &test_roster_config());
        let inflation = InflationTracker::new();
        let needs = CategoryValues::uniform(registry.len(), 0.5);

        // Analyze the 2nd best (top 3 + fills empty slot = StrongTarget)
        let analysis = compute_instant_analysis(
            &available[1],
            &roster,
            &available,
            &scarcity,
            &inflation,
            &needs,
            &registry,
        );

        assert_eq!(analysis.verdict, InstantVerdict::StrongTarget);
    }
}
