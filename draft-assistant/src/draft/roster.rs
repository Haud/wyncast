// Roster construction and slot assignment.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::pick::{Position, positions_from_espn_slot};

/// A player assigned to a roster slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosteredPlayer {
    pub name: String,
    pub price: u32,
    pub position: Position,
    /// ESPN eligible slot IDs. Empty if not available.
    pub eligible_slots: Vec<u16>,
    /// ESPN player ID for deduplication. None if not available.
    #[serde(default)]
    pub espn_player_id: Option<String>,
}

/// A single slot on a team's roster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterSlot {
    /// The position designation of this slot.
    pub position: Position,
    /// The player occupying this slot, if any.
    pub player: Option<RosteredPlayer>,
}

/// A team's complete roster of slots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Roster {
    pub slots: Vec<RosterSlot>,
}

impl Roster {
    /// Create a new roster from a config mapping position strings to slot counts.
    ///
    /// The roster config comes from league.toml `[league.roster]`, e.g.:
    /// `{"C": 1, "1B": 1, "SP": 5, "RP": 6, "BE": 6, "IL": 5, ...}`
    ///
    /// Slots are created in deterministic order based on `Position::sort_order()`.
    pub fn new(roster_config: &HashMap<String, usize>) -> Self {
        let mut slots: Vec<RosterSlot> = Vec::new();

        for (pos_str, &count) in roster_config {
            if let Some(pos) = Position::from_str_pos(pos_str) {
                for _ in 0..count {
                    slots.push(RosterSlot {
                        position: pos,
                        player: None,
                    });
                }
            }
        }

        // Sort by deterministic position order
        slots.sort_by_key(|s| s.position.sort_order());

        Roster { slots }
    }

    /// Whether there is an empty slot for the given position.
    pub fn has_empty_slot(&self, pos: Position) -> bool {
        self.slots
            .iter()
            .any(|s| s.position == pos && s.player.is_none())
    }

    /// Add a player to the roster.
    ///
    /// Slot assignment priority:
    /// 1. Dedicated position slot (exact match)
    /// 2. UTIL slot (for hitters only)
    /// 3. Bench (BE) slot
    ///
    /// Returns `true` if the player was successfully placed, `false` if no slot available.
    pub fn add_player(
        &mut self,
        name: &str,
        position_str: &str,
        price: u32,
        espn_player_id: Option<&str>,
    ) -> bool {
        let pos = match Position::from_str_pos(position_str) {
            Some(p) => p,
            None => return false,
        };

        let player = RosteredPlayer {
            name: name.to_string(),
            price,
            position: pos,
            eligible_slots: vec![],
            espn_player_id: espn_player_id.map(|s| s.to_string()),
        };

        // 1. Try dedicated position slot
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| s.position == pos && s.player.is_none())
        {
            slot.player = Some(player);
            return true;
        }

        // For outfielders (LF/CF/RF), also try the other outfield slots
        if matches!(
            pos,
            Position::LeftField | Position::CenterField | Position::RightField
        ) {
            let of_positions = [
                Position::LeftField,
                Position::CenterField,
                Position::RightField,
            ];
            for &of_pos in &of_positions {
                if of_pos == pos {
                    continue; // Already tried exact match
                }
                if let Some(slot) = self
                    .slots
                    .iter_mut()
                    .find(|s| s.position == of_pos && s.player.is_none())
                {
                    slot.player = Some(player);
                    return true;
                }
            }
        }

        // 2. Try UTIL slot (for hitters only)
        if pos.is_hitter() {
            if let Some(slot) = self
                .slots
                .iter_mut()
                .find(|s| s.position == Position::Utility && s.player.is_none())
            {
                slot.player = Some(player);
                return true;
            }
        }

        // 3. Try bench slot
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| s.position == Position::Bench && s.player.is_none())
        {
            slot.player = Some(player);
            return true;
        }

        false
    }

    /// Add a player using ESPN eligible slot IDs for multi-position placement.
    ///
    /// Slot assignment priority:
    /// 1. Try each eligible position slot (mapped from ESPN slot IDs, in slot order)
    /// 2. Try UTIL slot (for hitters)
    /// 3. Try bench slot
    ///
    /// Falls back to single-position `add_player()` if eligible_slots is empty.
    pub fn add_player_with_slots(
        &mut self,
        name: &str,
        position_str: &str,
        price: u32,
        eligible_slots: &[u16],
        espn_player_id: Option<&str>,
    ) -> bool {
        // Fall back to single-position logic if no slots provided
        if eligible_slots.is_empty() {
            return self.add_player(name, position_str, price, espn_player_id);
        }

        let parsed_pos = Position::from_str_pos(position_str);
        let display_pos = parsed_pos.unwrap_or(Position::Bench);
        let is_hitter = match parsed_pos {
            Some(pos) => pos.is_hitter(),
            None => {
                // Unknown position string — derive from eligible_slots
                eligible_slots.iter().any(|&slot_id| {
                    positions_from_espn_slot(slot_id).iter().any(|p| p.is_hitter())
                })
            }
        };

        let player = RosteredPlayer {
            name: name.to_string(),
            price,
            position: display_pos,
            eligible_slots: eligible_slots.to_vec(),
            espn_player_id: espn_player_id.map(|s| s.to_string()),
        };

        // 1. Try each eligible position slot (skip meta-slots like UTIL/BE/IL)
        for &slot_id in eligible_slots {
            for pos in positions_from_espn_slot(slot_id) {
                if pos.is_meta_slot() {
                    continue;
                }
                if let Some(slot) = self.slots.iter_mut().find(|s| s.position == pos && s.player.is_none()) {
                    slot.player = Some(player);
                    return true;
                }
            }
        }

        // 2. Try UTIL slot (for hitters only)
        if is_hitter {
            if let Some(slot) = self.slots.iter_mut().find(|s| s.position == Position::Utility && s.player.is_none()) {
                slot.player = Some(player);
                return true;
            }
        }

        // 3. Try bench slot
        if let Some(slot) = self.slots.iter_mut().find(|s| s.position == Position::Bench && s.player.is_none()) {
            slot.player = Some(player);
            return true;
        }

        false
    }

    /// Whether there is an empty slot for any of the given ESPN eligible slots.
    ///
    /// Checks eligible position slots, UTIL (if hitter), and bench.
    pub fn has_empty_slot_for_slots(&self, eligible_slots: &[u16], is_hitter: bool) -> bool {
        // Check each eligible position slot
        for &slot_id in eligible_slots {
            for pos in positions_from_espn_slot(slot_id) {
                if pos.is_meta_slot() {
                    continue;
                }
                if self.has_empty_slot(pos) {
                    return true;
                }
            }
        }

        // Check UTIL for hitters
        if is_hitter && self.has_empty_slot(Position::Utility) {
            return true;
        }

        // Check bench
        self.has_empty_slot(Position::Bench)
    }

    /// Count of empty slots, excluding IL slots.
    pub fn empty_slots(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.position != Position::InjuredList && s.player.is_none())
            .count()
    }

    /// Maximum bid a team can make given their remaining budget.
    ///
    /// Must reserve $1 per empty slot (excluding the slot about to be filled).
    pub fn max_bid(&self, budget_remaining: u32) -> u32 {
        let remaining_empty = self.empty_slots();
        if remaining_empty == 0 {
            return 0;
        }
        // Reserve $1 for each remaining slot that still needs to be filled after this one
        let reserved = (remaining_empty - 1) as u32;
        budget_remaining.saturating_sub(reserved)
    }

    /// Whether a player is already on this roster.
    ///
    /// Matching strategy:
    /// - When **both** the query and the rostered player carry an ESPN player ID,
    ///   the ID comparison is **authoritative**: if the IDs differ the players are
    ///   considered different, even if the names happen to match. ESPN player IDs
    ///   are globally unique and stable, so two distinct IDs always mean two
    ///   distinct players (name collisions, nicknames, and typos are irrelevant).
    /// - When **either** side lacks an ESPN player ID (e.g. manual entry, old
    ///   data, or DOM scraping that didn't capture the ID), the method falls back
    ///   to an exact name comparison.
    pub fn has_player(&self, name: &str, espn_player_id: Option<&str>) -> bool {
        self.slots.iter().any(|s| {
            s.player.as_ref().map_or(false, |p| {
                // If both sides have an ESPN player ID, match on that
                if let (Some(query_id), Some(rostered_id)) =
                    (espn_player_id, p.espn_player_id.as_deref())
                {
                    return query_id == rostered_id;
                }
                // Fall back to name comparison
                p.name == name
            })
        })
    }

    /// Number of filled (non-empty) slots.
    pub fn filled_count(&self) -> usize {
        self.slots.iter().filter(|s| s.player.is_some()).count()
    }

    /// Total number of slots (including IL).
    pub fn total_count(&self) -> usize {
        self.slots.len()
    }

    /// Total number of draftable slots (excluding IL).
    pub fn draftable_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.position != Position::InjuredList)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_roster_config() -> HashMap<String, usize> {
        let mut config = HashMap::new();
        config.insert("C".to_string(), 1);
        config.insert("1B".to_string(), 1);
        config.insert("2B".to_string(), 1);
        config.insert("3B".to_string(), 1);
        config.insert("SS".to_string(), 1);
        config.insert("LF".to_string(), 1);
        config.insert("CF".to_string(), 1);
        config.insert("RF".to_string(), 1);
        config.insert("UTIL".to_string(), 1);
        config.insert("SP".to_string(), 5);
        config.insert("RP".to_string(), 6);
        config.insert("BE".to_string(), 6);
        config.insert("IL".to_string(), 5);
        config
    }

    #[test]
    fn new_roster_correct_slot_count() {
        let roster = Roster::new(&test_roster_config());
        // C(1)+1B(1)+2B(1)+3B(1)+SS(1)+LF(1)+CF(1)+RF(1)+UTIL(1)+SP(5)+RP(6)+BE(6)+IL(5) = 31
        assert_eq!(roster.total_count(), 31);
    }

    #[test]
    fn new_roster_deterministic_order() {
        let roster = Roster::new(&test_roster_config());
        // First slot should be C, then 1B, 2B, 3B, SS, etc.
        assert_eq!(roster.slots[0].position, Position::Catcher);
        assert_eq!(roster.slots[1].position, Position::FirstBase);
        assert_eq!(roster.slots[2].position, Position::SecondBase);
        assert_eq!(roster.slots[3].position, Position::ThirdBase);
        assert_eq!(roster.slots[4].position, Position::ShortStop);
        // Last slots should be IL
        assert_eq!(
            roster.slots[roster.slots.len() - 1].position,
            Position::InjuredList
        );
    }

    #[test]
    fn new_roster_all_slots_empty() {
        let roster = Roster::new(&test_roster_config());
        assert_eq!(roster.filled_count(), 0);
        // empty_slots excludes IL(5), so: 31 - 5 = 26
        assert_eq!(roster.empty_slots(), 26);
    }

    #[test]
    fn add_player_dedicated_slot() {
        let mut roster = Roster::new(&test_roster_config());
        assert!(roster.add_player("Mike Trout", "CF", 45, None));
        assert_eq!(roster.filled_count(), 1);

        // CF slot should be filled
        let cf_slot = roster
            .slots
            .iter()
            .find(|s| s.position == Position::CenterField)
            .unwrap();
        assert!(cf_slot.player.is_some());
        assert_eq!(cf_slot.player.as_ref().unwrap().name, "Mike Trout");
    }

    #[test]
    fn add_player_util_fallback() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill the C slot first
        assert!(roster.add_player("Salvador Perez", "C", 10, None));
        // Second catcher should go to UTIL
        assert!(roster.add_player("Adley Rutschman", "C", 15, None));

        let util_slot = roster
            .slots
            .iter()
            .find(|s| s.position == Position::Utility)
            .unwrap();
        assert!(util_slot.player.is_some());
        assert_eq!(util_slot.player.as_ref().unwrap().name, "Adley Rutschman");
    }

    #[test]
    fn add_player_bench_fallback() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill C slot
        assert!(roster.add_player("Salvador Perez", "C", 10, None));
        // Fill UTIL slot
        assert!(roster.add_player("Adley Rutschman", "C", 15, None));
        // Third catcher should go to bench
        assert!(roster.add_player("Will Smith", "C", 8, None));

        let bench_slots: Vec<_> = roster
            .slots
            .iter()
            .filter(|s| s.position == Position::Bench && s.player.is_some())
            .collect();
        assert_eq!(bench_slots.len(), 1);
        assert_eq!(bench_slots[0].player.as_ref().unwrap().name, "Will Smith");
    }

    #[test]
    fn add_player_pitcher_skips_util() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill all 5 SP slots
        for i in 0..5 {
            assert!(roster.add_player(&format!("Pitcher {}", i), "SP", 10, None));
        }
        // 6th SP should go to bench (not UTIL)
        assert!(roster.add_player("Extra SP", "SP", 5, None));

        let util_slot = roster
            .slots
            .iter()
            .find(|s| s.position == Position::Utility)
            .unwrap();
        assert!(
            util_slot.player.is_none(),
            "UTIL should remain empty for pitchers"
        );

        let bench_pitchers: Vec<_> = roster
            .slots
            .iter()
            .filter(|s| s.position == Position::Bench && s.player.is_some())
            .collect();
        assert_eq!(bench_pitchers.len(), 1);
    }

    #[test]
    fn add_player_outfield_cross_slot() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill LF slot with an LF player
        assert!(roster.add_player("LF Player", "LF", 10, None));
        // A second LF player should go to CF or RF slot (cross-slot outfield)
        assert!(roster.add_player("LF Player 2", "LF", 8, None));

        // One of CF or RF should now be filled
        let of_filled: Vec<_> = roster
            .slots
            .iter()
            .filter(|s| {
                matches!(
                    s.position,
                    Position::CenterField | Position::RightField
                ) && s.player.is_some()
            })
            .collect();
        assert_eq!(of_filled.len(), 1);
    }

    #[test]
    fn has_empty_slot() {
        let mut roster = Roster::new(&test_roster_config());
        assert!(roster.has_empty_slot(Position::Catcher));
        assert!(roster.has_empty_slot(Position::StartingPitcher));

        roster.add_player("Test", "C", 5, None);
        assert!(!roster.has_empty_slot(Position::Catcher));
    }

    #[test]
    fn has_player_empty_roster() {
        let roster = Roster::new(&test_roster_config());
        assert!(!roster.has_player("Mike Trout", None));
    }

    #[test]
    fn has_player_after_add() {
        let mut roster = Roster::new(&test_roster_config());
        roster.add_player("Mike Trout", "CF", 45, None);
        assert!(roster.has_player("Mike Trout", None));
        assert!(!roster.has_player("Shohei Ohtani", None));
    }

    #[test]
    fn has_player_multiple_players() {
        let mut roster = Roster::new(&test_roster_config());
        roster.add_player("Mike Trout", "CF", 45, None);
        roster.add_player("Mookie Betts", "RF", 35, None);
        assert!(roster.has_player("Mike Trout", None));
        assert!(roster.has_player("Mookie Betts", None));
        assert!(!roster.has_player("Aaron Judge", None));
    }

    // -- ESPN ID matching tests for has_player --

    #[test]
    fn has_player_same_id_matches() {
        let mut roster = Roster::new(&test_roster_config());
        roster.add_player("Mike Trout", "CF", 45, Some("33003"));
        // Same ESPN ID should match regardless of name
        assert!(roster.has_player("Mike Trout", Some("33003")));
    }

    #[test]
    fn has_player_different_id_does_not_match_even_if_same_name() {
        let mut roster = Roster::new(&test_roster_config());
        roster.add_player("Mike Trout", "CF", 45, Some("33003"));
        // Same name but different ESPN ID — IDs are authoritative, so no match
        assert!(
            !roster.has_player("Mike Trout", Some("99999")),
            "different ESPN IDs should NOT match, even with the same name"
        );
    }

    #[test]
    fn has_player_same_id_matches_different_name() {
        let mut roster = Roster::new(&test_roster_config());
        roster.add_player("Mike Trout", "CF", 45, Some("33003"));
        // Same ESPN ID but different name (e.g. nickname, typo) — should match
        assert!(
            roster.has_player("M. Trout", Some("33003")),
            "same ESPN ID should match even when names differ"
        );
    }

    #[test]
    fn has_player_query_has_id_roster_does_not_falls_back_to_name() {
        let mut roster = Roster::new(&test_roster_config());
        // Player added without an ESPN ID (e.g. manual entry)
        roster.add_player("Mike Trout", "CF", 45, None);
        // Query has an ESPN ID but rostered player does not — falls back to name
        assert!(
            roster.has_player("Mike Trout", Some("33003")),
            "should fall back to name match when rostered player has no ID"
        );
        assert!(
            !roster.has_player("Someone Else", Some("33003")),
            "name mismatch should not match when rostered player has no ID"
        );
    }

    #[test]
    fn has_player_roster_has_id_query_does_not_falls_back_to_name() {
        let mut roster = Roster::new(&test_roster_config());
        // Player added with an ESPN ID
        roster.add_player("Mike Trout", "CF", 45, Some("33003"));
        // Query without an ESPN ID — falls back to name
        assert!(
            roster.has_player("Mike Trout", None),
            "should fall back to name match when query has no ID"
        );
        assert!(
            !roster.has_player("Someone Else", None),
            "name mismatch should not match when query has no ID"
        );
    }

    #[test]
    fn has_player_multiple_players_with_ids() {
        let mut roster = Roster::new(&test_roster_config());
        roster.add_player("Mike Trout", "CF", 45, Some("33003"));
        roster.add_player("Mookie Betts", "RF", 35, Some("33204"));
        // Each player matched by their own ID
        assert!(roster.has_player("Mike Trout", Some("33003")));
        assert!(roster.has_player("Mookie Betts", Some("33204")));
        // Mookie's ID matches Mookie on the roster, even though the name says
        // "Mike Trout" — ID is authoritative, name is ignored when both present
        assert!(
            roster.has_player("Mike Trout", Some("33204")),
            "ID 33204 matches Mookie Betts on the roster; name is irrelevant"
        );
        // An ID not on any rostered player should not match
        assert!(!roster.has_player("Aaron Judge", Some("55555")));
    }

    #[test]
    fn max_bid_full_budget() {
        let roster = Roster::new(&test_roster_config());
        // 26 draftable slots, all empty. Budget = 260.
        // Max bid = 260 - (26-1) = 235
        assert_eq!(roster.max_bid(260), 235);
    }

    #[test]
    fn max_bid_one_slot_left() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill all but one slot
        let positions = [
            "C", "1B", "2B", "3B", "SS", "LF", "CF", "RF", "SP", "SP", "SP", "SP", "SP", "RP",
            "RP", "RP", "RP", "RP", "RP",
        ];
        for (i, pos) in positions.iter().enumerate() {
            roster.add_player(&format!("Player {}", i), pos, 5, None);
        }
        // Fill UTIL
        roster.add_player("UTIL Player", "C", 5, None);
        // Fill 5 bench slots
        for i in 0..5 {
            roster.add_player(&format!("Bench {}", i), "C", 5, None);
        }
        // Now only 1 empty BE slot left
        assert_eq!(roster.empty_slots(), 1);
        // Max bid with 10 remaining = 10 (no reservation needed, last slot)
        assert_eq!(roster.max_bid(10), 10);
    }

    #[test]
    fn draftable_count() {
        let roster = Roster::new(&test_roster_config());
        // Total 31 - IL(5) = 26
        assert_eq!(roster.draftable_count(), 26);
    }

    #[test]
    fn add_player_returns_false_when_full() {
        let mut config = HashMap::new();
        config.insert("C".to_string(), 1);
        // No UTIL, no BE slots
        let mut roster = Roster::new(&config);
        assert!(roster.add_player("Player 1", "C", 5, None));
        assert!(!roster.add_player("Player 2", "C", 5, None));
    }

    #[test]
    fn add_player_invalid_position() {
        let mut roster = Roster::new(&test_roster_config());
        assert!(!roster.add_player("Player", "XX", 5, None));
    }

    // -- Multi-position (eligible_slots) tests --

    #[test]
    fn add_player_with_slots_multi_position() {
        let mut roster = Roster::new(&test_roster_config());
        // Mookie Betts: eligible at SS(4), 2B(2), RF(10), UTIL(12), BE(16), IL(17)
        let slots = vec![4, 2, 10, 12, 16, 17];
        // Fill SS slot first
        roster.add_player("Other SS", "SS", 10, None);
        // Mookie should go to 2B (next eligible position)
        assert!(roster.add_player_with_slots("Mookie Betts", "SS", 40, &slots, None));
        let slot_2b = roster.slots.iter().find(|s| s.position == Position::SecondBase).unwrap();
        assert!(slot_2b.player.is_some());
        assert_eq!(slot_2b.player.as_ref().unwrap().name, "Mookie Betts");
    }

    #[test]
    fn add_player_with_slots_falls_back_to_util() {
        let mut roster = Roster::new(&test_roster_config());
        // Player eligible only at C(0), UTIL(12), BE(16)
        let slots = vec![0, 12, 16, 17];
        // Fill C slot
        roster.add_player("Other C", "C", 10, None);
        // Should go to UTIL since C is full
        assert!(roster.add_player_with_slots("Player 2", "C", 8, &slots, None));
        let util = roster.slots.iter().find(|s| s.position == Position::Utility).unwrap();
        assert!(util.player.is_some());
        assert_eq!(util.player.as_ref().unwrap().name, "Player 2");
    }

    #[test]
    fn add_player_with_slots_empty_falls_back() {
        let mut roster = Roster::new(&test_roster_config());
        // Empty eligible_slots should use single-position fallback
        assert!(roster.add_player_with_slots("Mike Trout", "CF", 45, &[], None));
        let cf = roster.slots.iter().find(|s| s.position == Position::CenterField).unwrap();
        assert!(cf.player.is_some());
        assert_eq!(cf.player.as_ref().unwrap().name, "Mike Trout");
    }

    #[test]
    fn add_player_with_slots_pitcher_skips_util() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill all 5 SP slots
        for i in 0..5 {
            roster.add_player(&format!("SP {}", i), "SP", 10, None);
        }
        // SP with eligible_slots should go to bench, not UTIL
        let slots = vec![14, 16, 17]; // SP, BE, IL
        assert!(roster.add_player_with_slots("Extra SP", "SP", 5, &slots, None));
        let util = roster.slots.iter().find(|s| s.position == Position::Utility).unwrap();
        assert!(util.player.is_none());
    }

    #[test]
    fn has_empty_slot_for_slots_multi_position() {
        let mut roster = Roster::new(&test_roster_config());
        roster.add_player("Player", "SS", 10, None);
        // SS(4) is full, but 2B(2) should still be available
        let slots = vec![4, 2, 12, 16, 17];
        assert!(roster.has_empty_slot_for_slots(&slots, true));
    }

    #[test]
    fn has_empty_slot_for_slots_only_bench_left() {
        let mut config = HashMap::new();
        config.insert("C".to_string(), 1);
        config.insert("BE".to_string(), 1);
        let mut roster = Roster::new(&config);
        roster.add_player("Player", "C", 10, None);
        // C is full, no UTIL, but bench is available
        let slots = vec![0, 16]; // C, BE
        assert!(roster.has_empty_slot_for_slots(&slots, true));
    }

    #[test]
    fn has_empty_slot_for_slots_all_full() {
        let mut config = HashMap::new();
        config.insert("C".to_string(), 1);
        let mut roster = Roster::new(&config);
        roster.add_player("Player", "C", 10, None);
        // No UTIL, no bench
        let slots = vec![0]; // just C
        assert!(!roster.has_empty_slot_for_slots(&slots, true));
    }

    #[test]
    fn max_bid_returns_zero_when_roster_full() {
        let mut config = HashMap::new();
        config.insert("C".to_string(), 1);
        let mut roster = Roster::new(&config);
        roster.add_player("Player 1", "C", 5, None);
        assert_eq!(roster.empty_slots(), 0);
        assert_eq!(roster.max_bid(250), 0);
    }

    // -- Combo/generic slot expansion tests --

    #[test]
    fn add_player_with_slots_generic_of_slot() {
        let mut roster = Roster::new(&test_roster_config());
        // Player eligible only at generic OF (slot 5), UTIL (12), BE (16), IL (17)
        // Does NOT have individual LF/CF/RF slots — only the generic OF combo slot
        let slots = vec![5, 12, 16, 17];
        assert!(roster.add_player_with_slots("Juan Soto", "OF", 40, &slots, None));
        // Should be placed in one of LF, CF, or RF via the expanded OF slot
        let of_filled: Vec<_> = roster
            .slots
            .iter()
            .filter(|s| {
                matches!(
                    s.position,
                    Position::LeftField | Position::CenterField | Position::RightField
                ) && s.player.is_some()
            })
            .collect();
        assert_eq!(of_filled.len(), 1);
        assert_eq!(of_filled[0].player.as_ref().unwrap().name, "Juan Soto");
    }

    #[test]
    fn add_player_with_slots_combo_mi_slot() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill the 2B slot first
        roster.add_player("Other 2B", "2B", 10, None);
        // Player eligible at MI (slot 6) only — not at individual 2B or SS
        let slots = vec![6, 12, 16, 17];
        assert!(roster.add_player_with_slots("MI Player", "2B", 15, &slots, None));
        // 2B is full, so MI expansion should place in SS
        let ss_slot = roster
            .slots
            .iter()
            .find(|s| s.position == Position::ShortStop)
            .unwrap();
        assert!(ss_slot.player.is_some());
        assert_eq!(ss_slot.player.as_ref().unwrap().name, "MI Player");
    }

    #[test]
    fn add_player_with_slots_combo_ci_slot() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill 1B slot
        roster.add_player("Other 1B", "1B", 10, None);
        // Player eligible at CI (slot 7) only
        let slots = vec![7, 12, 16, 17];
        assert!(roster.add_player_with_slots("CI Player", "1B", 15, &slots, None));
        // 1B is full, so CI expansion should place in 3B
        let slot_3b = roster
            .slots
            .iter()
            .find(|s| s.position == Position::ThirdBase)
            .unwrap();
        assert!(slot_3b.player.is_some());
        assert_eq!(slot_3b.player.as_ref().unwrap().name, "CI Player");
    }

    #[test]
    fn has_empty_slot_for_slots_combo_of_all_full() {
        // Minimal config: only OF slots, no UTIL, no bench
        let mut config = HashMap::new();
        config.insert("LF".to_string(), 1);
        config.insert("CF".to_string(), 1);
        config.insert("RF".to_string(), 1);
        let mut roster = Roster::new(&config);
        // Fill all individual OF slots
        roster.add_player("LF Player", "LF", 10, None);
        roster.add_player("CF Player", "CF", 10, None);
        roster.add_player("RF Player", "RF", 10, None);
        // Generic OF slot should find no empty positions (no OF, no UTIL, no bench)
        let slots = vec![5]; // just OF combo
        assert!(!roster.has_empty_slot_for_slots(&slots, true));
    }

    #[test]
    fn has_empty_slot_for_slots_combo_of_with_util_fallback() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill all individual OF slots
        roster.add_player("LF Player", "LF", 10, None);
        roster.add_player("CF Player", "CF", 10, None);
        roster.add_player("RF Player", "RF", 10, None);
        // Generic OF slot finds no OF positions, but is_hitter=true finds UTIL
        let slots = vec![5]; // just OF combo
        assert!(roster.has_empty_slot_for_slots(&slots, true));
    }

    #[test]
    fn has_empty_slot_for_slots_combo_mi_finds_empty() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill 2B but leave SS open
        roster.add_player("2B Player", "2B", 10, None);
        // MI slot should find SS is still available
        let slots = vec![6]; // just MI combo
        assert!(roster.has_empty_slot_for_slots(&slots, true));
    }

    // -- is_hitter derivation from eligible_slots --

    #[test]
    fn is_hitter_derived_from_eligible_slots_for_unknown_position() {
        let mut roster = Roster::new(&test_roster_config());
        // Unknown position string "UNKNOWN", but eligible slots include hitter positions
        let hitter_slots = vec![5, 12, 16, 17]; // OF, UTIL, BE, IL
        assert!(roster.add_player_with_slots("Mystery Hitter", "UNKNOWN", 10, &hitter_slots, None));
        // Should be placed as a hitter — check that UTIL was used (since "UNKNOWN" maps to Bench,
        // and the eligible slot expansion puts them in an OF slot first)
        let of_filled: Vec<_> = roster
            .slots
            .iter()
            .filter(|s| {
                matches!(
                    s.position,
                    Position::LeftField | Position::CenterField | Position::RightField
                ) && s.player.is_some()
            })
            .collect();
        assert_eq!(of_filled.len(), 1);
        assert_eq!(of_filled[0].player.as_ref().unwrap().name, "Mystery Hitter");
    }

    #[test]
    fn is_hitter_false_from_eligible_slots_for_unknown_pitcher() {
        let mut roster = Roster::new(&test_roster_config());
        // Unknown position string, but eligible slots are pitcher-only
        let pitcher_slots = vec![14, 16, 17]; // SP, BE, IL
        // Fill all SP slots
        for i in 0..5 {
            roster.add_player(&format!("SP {}", i), "SP", 10, None);
        }
        assert!(roster.add_player_with_slots("Mystery Pitcher", "UNKNOWN", 5, &pitcher_slots, None));
        // is_hitter should be false (derived from pitcher-only slots), so UTIL should NOT be used
        let util = roster
            .slots
            .iter()
            .find(|s| s.position == Position::Utility)
            .unwrap();
        assert!(util.player.is_none(), "UTIL should remain empty for pitcher-derived unknown position");
        // Should be on the bench
        let bench_players: Vec<_> = roster
            .slots
            .iter()
            .filter(|s| s.position == Position::Bench && s.player.is_some())
            .collect();
        assert_eq!(bench_players.len(), 1);
        assert_eq!(bench_players[0].player.as_ref().unwrap().name, "Mystery Pitcher");
    }
}
