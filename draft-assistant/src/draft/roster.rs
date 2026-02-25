// Roster construction and slot assignment.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::pick::Position;

/// A player assigned to a roster slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosteredPlayer {
    pub name: String,
    pub price: u32,
    pub position: Position,
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
    pub fn add_player(&mut self, name: &str, position_str: &str, price: u32) -> bool {
        let pos = match Position::from_str_pos(position_str) {
            Some(p) => p,
            None => return false,
        };

        let player = RosteredPlayer {
            name: name.to_string(),
            price,
            position: pos,
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
        if matches!(pos, Position::LF | Position::CF | Position::RF) {
            let of_positions = [Position::LF, Position::CF, Position::RF];
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
                .find(|s| s.position == Position::UTIL && s.player.is_none())
            {
                slot.player = Some(player);
                return true;
            }
        }

        // 3. Try bench slot
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| s.position == Position::BE && s.player.is_none())
        {
            slot.player = Some(player);
            return true;
        }

        false
    }

    /// Count of empty slots, excluding IL slots.
    pub fn empty_slots(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.position != Position::IL && s.player.is_none())
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
            .filter(|s| s.position != Position::IL)
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
        assert_eq!(roster.slots[0].position, Position::C);
        assert_eq!(roster.slots[1].position, Position::FirstBase);
        assert_eq!(roster.slots[2].position, Position::SecondBase);
        assert_eq!(roster.slots[3].position, Position::ThirdBase);
        assert_eq!(roster.slots[4].position, Position::SS);
        // Last slots should be IL
        assert_eq!(roster.slots[roster.slots.len() - 1].position, Position::IL);
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
        assert!(roster.add_player("Mike Trout", "CF", 45));
        assert_eq!(roster.filled_count(), 1);

        // CF slot should be filled
        let cf_slot = roster
            .slots
            .iter()
            .find(|s| s.position == Position::CF)
            .unwrap();
        assert!(cf_slot.player.is_some());
        assert_eq!(cf_slot.player.as_ref().unwrap().name, "Mike Trout");
    }

    #[test]
    fn add_player_util_fallback() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill the C slot first
        assert!(roster.add_player("Salvador Perez", "C", 10));
        // Second catcher should go to UTIL
        assert!(roster.add_player("Adley Rutschman", "C", 15));

        let util_slot = roster
            .slots
            .iter()
            .find(|s| s.position == Position::UTIL)
            .unwrap();
        assert!(util_slot.player.is_some());
        assert_eq!(util_slot.player.as_ref().unwrap().name, "Adley Rutschman");
    }

    #[test]
    fn add_player_bench_fallback() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill C slot
        assert!(roster.add_player("Salvador Perez", "C", 10));
        // Fill UTIL slot
        assert!(roster.add_player("Adley Rutschman", "C", 15));
        // Third catcher should go to bench
        assert!(roster.add_player("Will Smith", "C", 8));

        let bench_slots: Vec<_> = roster
            .slots
            .iter()
            .filter(|s| s.position == Position::BE && s.player.is_some())
            .collect();
        assert_eq!(bench_slots.len(), 1);
        assert_eq!(bench_slots[0].player.as_ref().unwrap().name, "Will Smith");
    }

    #[test]
    fn add_player_pitcher_skips_util() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill all 5 SP slots
        for i in 0..5 {
            assert!(roster.add_player(&format!("Pitcher {}", i), "SP", 10));
        }
        // 6th SP should go to bench (not UTIL)
        assert!(roster.add_player("Extra SP", "SP", 5));

        let util_slot = roster
            .slots
            .iter()
            .find(|s| s.position == Position::UTIL)
            .unwrap();
        assert!(util_slot.player.is_none(), "UTIL should remain empty for pitchers");

        let bench_pitchers: Vec<_> = roster
            .slots
            .iter()
            .filter(|s| s.position == Position::BE && s.player.is_some())
            .collect();
        assert_eq!(bench_pitchers.len(), 1);
    }

    #[test]
    fn add_player_outfield_cross_slot() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill LF slot with an LF player
        assert!(roster.add_player("LF Player", "LF", 10));
        // A second LF player should go to CF or RF slot (cross-slot outfield)
        assert!(roster.add_player("LF Player 2", "LF", 8));

        // One of CF or RF should now be filled
        let of_filled: Vec<_> = roster
            .slots
            .iter()
            .filter(|s| matches!(s.position, Position::CF | Position::RF) && s.player.is_some())
            .collect();
        assert_eq!(of_filled.len(), 1);
    }

    #[test]
    fn has_empty_slot() {
        let mut roster = Roster::new(&test_roster_config());
        assert!(roster.has_empty_slot(Position::C));
        assert!(roster.has_empty_slot(Position::SP));

        roster.add_player("Test", "C", 5);
        assert!(!roster.has_empty_slot(Position::C));
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
            roster.add_player(&format!("Player {}", i), pos, 5);
        }
        // Fill UTIL
        roster.add_player("UTIL Player", "C", 5);
        // Fill 5 bench slots
        for i in 0..5 {
            roster.add_player(&format!("Bench {}", i), "C", 5);
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
        assert!(roster.add_player("Player 1", "C", 5));
        assert!(!roster.add_player("Player 2", "C", 5));
    }

    #[test]
    fn add_player_invalid_position() {
        let mut roster = Roster::new(&test_roster_config());
        assert!(!roster.add_player("Player", "XX", 5));
    }

    #[test]
    fn max_bid_returns_zero_when_roster_full() {
        let mut config = HashMap::new();
        config.insert("C".to_string(), 1);
        let mut roster = Roster::new(&config);
        roster.add_player("Player 1", "C", 5);
        assert_eq!(roster.empty_slots(), 0);
        assert_eq!(roster.max_bid(250), 0);
    }
}
