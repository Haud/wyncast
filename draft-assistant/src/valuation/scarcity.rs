// Positional scarcity index calculation.
//
// For each position, measures how many quality players remain available and
// how steeply talent drops off after the top options. This drives urgency
// ratings that inform draft-day bidding decisions.

use std::collections::HashMap;

use crate::draft::pick::Position;
use crate::valuation::projections::PitcherType;
use crate::valuation::zscore::PlayerValuation;

// ---------------------------------------------------------------------------
// Scarcity urgency levels
// ---------------------------------------------------------------------------

/// How urgently a position needs to be addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScarcityUrgency {
    /// 0-2 players above replacement: act now or miss out.
    Critical,
    /// 3-4 players above replacement: should address soon.
    High,
    /// 5-7 players above replacement: comfortable window.
    Medium,
    /// 8+ players above replacement: no rush.
    Low,
}

impl ScarcityUrgency {
    /// Determine urgency from the count of players above replacement.
    pub fn from_count(players_above_replacement: usize) -> Self {
        match players_above_replacement {
            0..=2 => ScarcityUrgency::Critical,
            3..=4 => ScarcityUrgency::High,
            5..=7 => ScarcityUrgency::Medium,
            _ => ScarcityUrgency::Low,
        }
    }

    /// Return a human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            ScarcityUrgency::Critical => "CRITICAL",
            ScarcityUrgency::High => "HIGH",
            ScarcityUrgency::Medium => "MEDIUM",
            ScarcityUrgency::Low => "LOW",
        }
    }

    /// Scarcity premium multiplier for bid ceiling calculation.
    ///
    /// Critical = +30%, High = +15%, Medium = +0%, Low = -10%.
    pub fn premium(&self) -> f64 {
        match self {
            ScarcityUrgency::Critical => 0.30,
            ScarcityUrgency::High => 0.15,
            ScarcityUrgency::Medium => 0.0,
            ScarcityUrgency::Low => -0.10,
        }
    }
}

// ---------------------------------------------------------------------------
// Scarcity entry
// ---------------------------------------------------------------------------

/// Scarcity analysis for a single position.
#[derive(Debug, Clone)]
pub struct ScarcityEntry {
    /// The position being analyzed.
    pub position: Position,
    /// Number of available players at this position with positive VOR.
    pub players_above_replacement: usize,
    /// VOR of the top available player at this position.
    pub top_available_vor: f64,
    /// VOR of the 3rd-best available player (replacement-level proxy).
    /// If fewer than 3 are available, uses the worst available or 0.0.
    pub replacement_vor: f64,
    /// Difference between top and replacement VOR.
    pub dropoff: f64,
    /// Urgency rating based on available count.
    pub urgency: ScarcityUrgency,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Derive tracked positions from roster config keys.
///
/// Skips meta slots (BE, IL). For combo slots (OF, MI, CI, P), expands to
/// their constituent concrete positions so scarcity is tracked at the
/// concrete level (e.g. LF, CF, RF instead of just OF).
fn derive_tracked_positions(roster_config: &HashMap<String, usize>) -> Vec<Position> {
    let mut positions = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for key in roster_config.keys() {
        if let Some(pos) = Position::from_roster_slot_str(key) {
            if pos.is_meta_slot() {
                continue;
            }
            if pos.is_combo_slot() {
                // Expand combo slots to concrete positions for tracking
                for concrete in pos.accepted_positions() {
                    if seen.insert(concrete) {
                        positions.push(concrete);
                    }
                }
            } else if seen.insert(pos) {
                positions.push(pos);
            }
        }
    }

    positions
}

/// Check whether a player is eligible at a given position.
///
/// Primary check: the `positions` list (populated from ESPN eligible_slots
/// overlay or backfilled by the VOR pipeline). Fallback: `best_position`
/// (set by VOR computation) and `pitcher_type` (always known for pitchers).
fn player_eligible_at(p: &PlayerValuation, pos: Position) -> bool {
    // Primary: explicit positions list
    if p.positions.contains(&pos) {
        return true;
    }
    // For combo positions (e.g. Outfield), check if the player is eligible
    // at any of the constituent concrete positions.
    if pos.is_combo_slot() {
        for concrete in pos.accepted_positions() {
            if p.positions.contains(&concrete) {
                return true;
            }
        }
    }
    // Fallback: best_position assigned by VOR
    if p.best_position == Some(pos) {
        return true;
    }
    // Fallback: pitcher_type for SP/RP
    match (pos, p.pitcher_type) {
        (Position::StartingPitcher, Some(PitcherType::SP)) => true,
        (Position::ReliefPitcher, Some(PitcherType::RP)) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Core computation
// ---------------------------------------------------------------------------

/// Compute positional scarcity for all tracked positions.
///
/// For each position:
/// 1. Collect available players eligible at that position with positive VOR.
/// 2. Sort them descending by VOR.
/// 3. Count how many are above replacement (VOR > 0).
/// 4. Find the top VOR and the 3rd-best VOR.
/// 5. Compute dropoff = top - 3rd-best.
/// 6. Assign urgency based on count thresholds.
pub fn compute_scarcity(
    available_players: &[PlayerValuation],
    roster_config: &HashMap<String, usize>,
) -> Vec<ScarcityEntry> {
    let tracked = derive_tracked_positions(roster_config);
    let mut entries = Vec::new();

    for &pos in &tracked {
        // Collect players eligible at this position with positive VOR.
        // Check positions list first; fall back to best_position and
        // pitcher_type for players that lack ESPN position overlay data.
        let mut eligible: Vec<f64> = available_players
            .iter()
            .filter(|p| p.initial_vor > 0.0 && player_eligible_at(p, pos))
            .map(|p| p.vor)
            .collect();

        eligible.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

        let players_above_replacement = eligible.len();

        let top_available_vor = eligible.first().copied().unwrap_or(0.0);

        // 3rd-best VOR (index 2), or the last available, or 0.0
        let replacement_vor = if eligible.len() >= 3 {
            eligible[2]
        } else if let Some(&last) = eligible.last() {
            last
        } else {
            0.0
        };

        let dropoff = top_available_vor - replacement_vor;

        let urgency = ScarcityUrgency::from_count(players_above_replacement);

        entries.push(ScarcityEntry {
            position: pos,
            players_above_replacement,
            top_available_vor,
            replacement_vor,
            dropoff,
            urgency,
        });
    }

    // Sort by urgency (most urgent first), then by dropoff descending.
    entries.sort_by(|a, b| {
        let urgency_order = |u: &ScarcityUrgency| -> u8 {
            match u {
                ScarcityUrgency::Critical => 0,
                ScarcityUrgency::High => 1,
                ScarcityUrgency::Medium => 2,
                ScarcityUrgency::Low => 3,
            }
        };
        urgency_order(&a.urgency)
            .cmp(&urgency_order(&b.urgency))
            .then_with(|| {
                b.dropoff
                    .partial_cmp(&a.dropoff)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    entries
}

/// Look up the scarcity entry for a given position.
pub fn scarcity_for_position(
    scarcity: &[ScarcityEntry],
    position: Position,
) -> Option<&ScarcityEntry> {
    scarcity.iter().find(|e| e.position == position)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::valuation::projections::PitcherType;
    use crate::stats::CategoryValues;
    use crate::valuation::zscore::{
        CategoryZScores, ProjectionData,
    };
    use std::collections::HashMap;

    fn approx_eq(a: f64, b: f64, epsilon: f64) -> bool {
        (a - b).abs() < epsilon
    }

    fn test_roster_config() -> HashMap<String, usize> {
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
        roster
    }

    fn make_hitter(name: &str, vor: f64, positions: Vec<Position>) -> PlayerValuation {
        let best_pos = positions.first().copied();
        PlayerValuation {
            name: name.into(),
            team: "TST".into(),
            positions,
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
            category_zscores: CategoryZScores::hitter(CategoryValues::zeros(12), vor + 2.0),
            vor,
            initial_vor: vor,
            best_position: best_pos,
            dollar_value: vor.max(1.0) * 5.0 + 1.0,
        }
    }

    fn make_pitcher(name: &str, vor: f64, pitcher_type: PitcherType) -> PlayerValuation {
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
                    ("ip".into(), 180.0), ("k".into(), 200.0), ("w".into(), 14.0),
                    ("sv".into(), 0.0), ("hd".into(), 0.0), ("era".into(), 3.20),
                    ("whip".into(), 1.10), ("g".into(), 30.0), ("gs".into(), 30.0),
                ]),
            },
            total_zscore: vor + 1.0,
            category_zscores: CategoryZScores::pitcher(CategoryValues::zeros(12), vor + 1.0),
            vor,
            initial_vor: vor,
            best_position: Some(pos),
            dollar_value: vor.max(1.0) * 5.0 + 1.0,
        }
    }

    #[test]
    fn urgency_thresholds() {
        assert_eq!(ScarcityUrgency::from_count(0), ScarcityUrgency::Critical);
        assert_eq!(ScarcityUrgency::from_count(1), ScarcityUrgency::Critical);
        assert_eq!(ScarcityUrgency::from_count(2), ScarcityUrgency::Critical);
        assert_eq!(ScarcityUrgency::from_count(3), ScarcityUrgency::High);
        assert_eq!(ScarcityUrgency::from_count(4), ScarcityUrgency::High);
        assert_eq!(ScarcityUrgency::from_count(5), ScarcityUrgency::Medium);
        assert_eq!(ScarcityUrgency::from_count(6), ScarcityUrgency::Medium);
        assert_eq!(ScarcityUrgency::from_count(7), ScarcityUrgency::Medium);
        assert_eq!(ScarcityUrgency::from_count(8), ScarcityUrgency::Low);
        assert_eq!(ScarcityUrgency::from_count(15), ScarcityUrgency::Low);
    }

    #[test]
    fn scarcity_dropoff_calculation() {
        let roster = test_roster_config();

        // Create a pool with known VOR values at catcher
        let players = vec![
            make_hitter("C1", 8.0, vec![Position::Catcher]),
            make_hitter("C2", 5.0, vec![Position::Catcher]),
            make_hitter("C3", 2.0, vec![Position::Catcher]),
            make_hitter("C4", 1.0, vec![Position::Catcher]),
        ];

        let scarcity = compute_scarcity(&players, &roster);
        let c_entry = scarcity_for_position(&scarcity, Position::Catcher).unwrap();

        assert_eq!(c_entry.players_above_replacement, 4);
        assert!(approx_eq(c_entry.top_available_vor, 8.0, 0.01));
        assert!(approx_eq(c_entry.replacement_vor, 2.0, 0.01));
        assert!(approx_eq(c_entry.dropoff, 6.0, 0.01));
        assert_eq!(c_entry.urgency, ScarcityUrgency::High);
    }

    #[test]
    fn scarcity_critical_with_few_players() {
        let roster = test_roster_config();

        // Only 2 shortstops with positive VOR -> Critical
        let players = vec![
            make_hitter("SS1", 5.0, vec![Position::ShortStop]),
            make_hitter("SS2", 2.0, vec![Position::ShortStop]),
        ];

        let scarcity = compute_scarcity(&players, &roster);
        let ss_entry = scarcity_for_position(&scarcity, Position::ShortStop).unwrap();

        assert_eq!(ss_entry.players_above_replacement, 2);
        assert_eq!(ss_entry.urgency, ScarcityUrgency::Critical);
        // With < 3 players, replacement_vor = last player's VOR
        assert!(approx_eq(ss_entry.replacement_vor, 2.0, 0.01));
        assert!(approx_eq(ss_entry.dropoff, 3.0, 0.01));
    }

    #[test]
    fn scarcity_empty_position() {
        let roster = test_roster_config();
        let players: Vec<PlayerValuation> = Vec::new();

        let scarcity = compute_scarcity(&players, &roster);
        let c_entry = scarcity_for_position(&scarcity, Position::Catcher).unwrap();

        assert_eq!(c_entry.players_above_replacement, 0);
        assert_eq!(c_entry.urgency, ScarcityUrgency::Critical);
        assert!(approx_eq(c_entry.top_available_vor, 0.0, 0.01));
        assert!(approx_eq(c_entry.dropoff, 0.0, 0.01));
    }

    #[test]
    fn scarcity_low_with_many_players() {
        let roster = test_roster_config();

        // 10 first basemen with positive VOR -> Low urgency
        let players: Vec<PlayerValuation> = (0..10)
            .map(|i| {
                make_hitter(
                    &format!("1B_{}", i + 1),
                    10.0 - i as f64,
                    vec![Position::FirstBase],
                )
            })
            .collect();

        let scarcity = compute_scarcity(&players, &roster);
        let fb_entry = scarcity_for_position(&scarcity, Position::FirstBase).unwrap();

        assert_eq!(fb_entry.players_above_replacement, 10);
        assert_eq!(fb_entry.urgency, ScarcityUrgency::Low);
        assert!(approx_eq(fb_entry.top_available_vor, 10.0, 0.01));
        assert!(approx_eq(fb_entry.replacement_vor, 8.0, 0.01));
        assert!(approx_eq(fb_entry.dropoff, 2.0, 0.01));
    }

    #[test]
    fn scarcity_excludes_negative_vor() {
        let roster = test_roster_config();

        let players = vec![
            make_hitter("2B_good", 3.0, vec![Position::SecondBase]),
            make_hitter("2B_bad1", -1.0, vec![Position::SecondBase]),
            make_hitter("2B_bad2", -3.0, vec![Position::SecondBase]),
        ];

        let scarcity = compute_scarcity(&players, &roster);
        let sb_entry = scarcity_for_position(&scarcity, Position::SecondBase).unwrap();

        // Only 1 player with positive VOR
        assert_eq!(sb_entry.players_above_replacement, 1);
        assert_eq!(sb_entry.urgency, ScarcityUrgency::Critical);
    }

    #[test]
    fn scarcity_pitcher_positions() {
        let roster = test_roster_config();

        let mut players = Vec::new();
        for i in 0..6 {
            players.push(make_pitcher(
                &format!("SP_{}", i + 1),
                8.0 - i as f64,
                PitcherType::SP,
            ));
        }
        for i in 0..3 {
            players.push(make_pitcher(
                &format!("RP_{}", i + 1),
                4.0 - i as f64,
                PitcherType::RP,
            ));
        }

        let scarcity = compute_scarcity(&players, &roster);

        let sp_entry = scarcity_for_position(&scarcity, Position::StartingPitcher).unwrap();
        assert_eq!(sp_entry.players_above_replacement, 6);
        assert_eq!(sp_entry.urgency, ScarcityUrgency::Medium);

        let rp_entry = scarcity_for_position(&scarcity, Position::ReliefPitcher).unwrap();
        assert_eq!(rp_entry.players_above_replacement, 3);
        assert_eq!(rp_entry.urgency, ScarcityUrgency::High);
    }

    #[test]
    fn scarcity_sorted_by_urgency() {
        let roster = test_roster_config();

        // Create a mix: catchers (2 = Critical), SS (4 = High), 1B (10 = Low)
        let mut players = Vec::new();
        for i in 0..2 {
            players.push(make_hitter(&format!("C_{}", i), 5.0 - i as f64, vec![Position::Catcher]));
        }
        for i in 0..4 {
            players.push(make_hitter(&format!("SS_{}", i), 5.0 - i as f64, vec![Position::ShortStop]));
        }
        for i in 0..10 {
            players.push(make_hitter(&format!("1B_{}", i), 10.0 - i as f64, vec![Position::FirstBase]));
        }

        let scarcity = compute_scarcity(&players, &roster);

        // Critical positions should come first
        let first_urgency = scarcity[0].urgency;
        assert!(
            first_urgency == ScarcityUrgency::Critical,
            "First entry should be Critical, got {:?}",
            first_urgency
        );
    }

    #[test]
    fn premium_values() {
        assert!(approx_eq(ScarcityUrgency::Critical.premium(), 0.30, 0.001));
        assert!(approx_eq(ScarcityUrgency::High.premium(), 0.15, 0.001));
        assert!(approx_eq(ScarcityUrgency::Medium.premium(), 0.0, 0.001));
        assert!(approx_eq(ScarcityUrgency::Low.premium(), -0.10, 0.001));
    }

    #[test]
    fn player_eligible_at_uses_positions_list() {
        let player = make_hitter("Multi", 5.0, vec![Position::SecondBase, Position::ShortStop]);
        assert!(super::player_eligible_at(&player, Position::SecondBase));
        assert!(super::player_eligible_at(&player, Position::ShortStop));
        assert!(!super::player_eligible_at(&player, Position::Catcher));
    }

    #[test]
    fn player_eligible_at_falls_back_to_best_position() {
        let mut player = make_hitter("Assigned", 5.0, vec![]);
        player.best_position = Some(Position::FirstBase);
        assert!(super::player_eligible_at(&player, Position::FirstBase));
        assert!(!super::player_eligible_at(&player, Position::Catcher));
    }

    #[test]
    fn player_eligible_at_falls_back_to_pitcher_type() {
        let mut player = make_pitcher("Ace", 5.0, PitcherType::SP);
        // Clear positions to test fallback path
        player.positions.clear();
        player.best_position = None;
        assert!(super::player_eligible_at(&player, Position::StartingPitcher));
        assert!(!super::player_eligible_at(&player, Position::ReliefPitcher));
    }

    /// Integration test: with a full player pool at draft start (produced by
    /// the VOR pipeline), most positions should show Low or Medium urgency,
    /// not Critical. This verifies the fix for the bug where empty `positions`
    /// caused all scarcity gauges to show Critical.
    #[test]
    fn full_pool_draft_start_not_all_critical() {
        let roster = test_roster_config();

        // Simulate a full draft pool: 15 players per hitter position
        // and 15 SP + 15 RP, all with positive VOR and best_position set.
        // This mimics what the VOR pipeline produces at startup.
        let mut players = Vec::new();

        let hitter_positions = [
            Position::Catcher,
            Position::FirstBase,
            Position::SecondBase,
            Position::ThirdBase,
            Position::ShortStop,
            Position::LeftField,
            Position::CenterField,
            Position::RightField,
        ];

        for &pos in &hitter_positions {
            for i in 0..15 {
                let mut p = make_hitter(
                    &format!("{}_{}", pos.display_str(), i + 1),
                    15.0 - i as f64,
                    vec![pos],
                );
                p.best_position = Some(pos);
                players.push(p);
            }
        }

        for i in 0..15 {
            let mut p = make_pitcher(
                &format!("SP_{}", i + 1),
                15.0 - i as f64,
                PitcherType::SP,
            );
            p.best_position = Some(Position::StartingPitcher);
            players.push(p);
        }

        for i in 0..15 {
            let mut p = make_pitcher(
                &format!("RP_{}", i + 1),
                12.0 - i as f64,
                PitcherType::RP,
            );
            p.best_position = Some(Position::ReliefPitcher);
            players.push(p);
        }

        let scarcity = compute_scarcity(&players, &roster);

        // No position should be Critical with 15 players per position
        let critical_count = scarcity
            .iter()
            .filter(|e| e.urgency == ScarcityUrgency::Critical)
            .count();
        assert_eq!(
            critical_count, 0,
            "No positions should be Critical at draft start with a full pool, \
             but {} are: {:?}",
            critical_count,
            scarcity
                .iter()
                .filter(|e| e.urgency == ScarcityUrgency::Critical)
                .map(|e| e.position.display_str())
                .collect::<Vec<_>>()
        );

        // All positions should show Low urgency (8+ players above replacement)
        for entry in &scarcity {
            assert_eq!(
                entry.urgency,
                ScarcityUrgency::Low,
                "Position {} should be Low urgency with {} players above replacement, got {:?}",
                entry.position.display_str(),
                entry.players_above_replacement,
                entry.urgency
            );
        }
    }

    /// Integration test: players with empty positions but best_position set
    /// (as produced by the VOR pipeline backfill) still get counted by
    /// compute_scarcity via the best_position fallback.
    #[test]
    fn scarcity_uses_best_position_fallback() {
        let roster = test_roster_config();

        // Create players with empty positions but best_position set,
        // simulating what VOR backfill produces for players without ESPN data.
        let mut players = Vec::new();
        for i in 0..10 {
            let mut p = make_hitter(
                &format!("SS_{}", i + 1),
                10.0 - i as f64,
                vec![], // empty positions
            );
            p.best_position = Some(Position::ShortStop);
            players.push(p);
        }

        let scarcity = compute_scarcity(&players, &roster);
        let ss_entry = scarcity_for_position(&scarcity, Position::ShortStop).unwrap();

        assert_eq!(ss_entry.players_above_replacement, 10);
        assert_eq!(ss_entry.urgency, ScarcityUrgency::Low);
    }
}
