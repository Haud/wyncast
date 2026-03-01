// Individual pick representation and processing.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// ESPN slot ID constants (from ESPN Fantasy API v3)
// ---------------------------------------------------------------------------

pub const ESPN_SLOT_C: u16 = 0;
pub const ESPN_SLOT_1B: u16 = 1;
pub const ESPN_SLOT_2B: u16 = 2;
pub const ESPN_SLOT_3B: u16 = 3;
pub const ESPN_SLOT_SS: u16 = 4;
pub const ESPN_SLOT_OF: u16 = 5;
pub const ESPN_SLOT_MI: u16 = 6; // 2B/SS combo
pub const ESPN_SLOT_CI: u16 = 7; // 1B/3B combo
pub const ESPN_SLOT_LF: u16 = 8;
pub const ESPN_SLOT_CF: u16 = 9;
pub const ESPN_SLOT_RF: u16 = 10;
pub const ESPN_SLOT_DH: u16 = 11;
pub const ESPN_SLOT_UTIL: u16 = 12;
pub const ESPN_SLOT_P: u16 = 13; // Generic pitcher
pub const ESPN_SLOT_SP: u16 = 14;
pub const ESPN_SLOT_RP: u16 = 15;
pub const ESPN_SLOT_BE: u16 = 16;
pub const ESPN_SLOT_IL: u16 = 17;

/// Baseball positions used for roster slot assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Position {
    Catcher,
    FirstBase,
    SecondBase,
    ThirdBase,
    ShortStop,
    LeftField,
    CenterField,
    RightField,
    StartingPitcher,
    ReliefPitcher,
    DesignatedHitter,
    Utility,
    Bench,
    InjuredList,
}

impl Position {
    /// Parse a position string into a Position enum.
    ///
    /// Handles ESPN-style abbreviations:
    /// - "1B" -> FirstBase, "2B" -> SecondBase, "3B" -> ThirdBase
    /// - "OF" -> CenterField (generic outfield maps to CenterField slot)
    /// - "DH" -> DesignatedHitter, "UTIL" -> Utility, "BE"/"BN" -> Bench, "IL"/"DL" -> InjuredList
    pub fn from_str_pos(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "C" => Some(Position::Catcher),
            "1B" => Some(Position::FirstBase),
            "2B" => Some(Position::SecondBase),
            "3B" => Some(Position::ThirdBase),
            "SS" => Some(Position::ShortStop),
            "LF" => Some(Position::LeftField),
            "CF" => Some(Position::CenterField),
            "RF" => Some(Position::RightField),
            "OF" => Some(Position::CenterField),
            "SP" => Some(Position::StartingPitcher),
            "RP" => Some(Position::ReliefPitcher),
            "DH" => Some(Position::DesignatedHitter),
            "UTIL" => Some(Position::Utility),
            "BE" | "BN" => Some(Position::Bench),
            "IL" | "DL" => Some(Position::InjuredList),
            _ => None,
        }
    }

    /// Return the display string for this position.
    pub fn display_str(&self) -> &'static str {
        match self {
            Position::Catcher => "C",
            Position::FirstBase => "1B",
            Position::SecondBase => "2B",
            Position::ThirdBase => "3B",
            Position::ShortStop => "SS",
            Position::LeftField => "LF",
            Position::CenterField => "CF",
            Position::RightField => "RF",
            Position::StartingPitcher => "SP",
            Position::ReliefPitcher => "RP",
            Position::DesignatedHitter => "DH",
            Position::Utility => "UTIL",
            Position::Bench => "BE",
            Position::InjuredList => "IL",
        }
    }

    /// Whether this position is a hitting position (not a pitcher).
    pub fn is_hitter(&self) -> bool {
        matches!(
            self,
            Position::Catcher
                | Position::FirstBase
                | Position::SecondBase
                | Position::ThirdBase
                | Position::ShortStop
                | Position::LeftField
                | Position::CenterField
                | Position::RightField
                | Position::DesignatedHitter
                | Position::Utility
        )
    }

    /// Whether this is a meta-slot (not a concrete playing position).
    pub fn is_meta_slot(&self) -> bool {
        matches!(self, Position::Utility | Position::Bench | Position::InjuredList)
    }

    /// Deterministic ordering index for roster slot display.
    pub fn sort_order(&self) -> u8 {
        match self {
            Position::Catcher => 0,
            Position::FirstBase => 1,
            Position::SecondBase => 2,
            Position::ThirdBase => 3,
            Position::ShortStop => 4,
            Position::LeftField => 5,
            Position::CenterField => 6,
            Position::RightField => 7,
            Position::Utility => 8,
            Position::DesignatedHitter => 9,
            Position::StartingPitcher => 10,
            Position::ReliefPitcher => 11,
            Position::Bench => 12,
            Position::InjuredList => 13,
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_str())
    }
}

// ---------------------------------------------------------------------------
// ESPN slot ID mapping functions
// ---------------------------------------------------------------------------

/// Map an ESPN slot ID to a Position enum value.
/// Returns None for combo/flex slots not in our league (MI, CI, P, generic OF).
pub fn position_from_espn_slot(slot_id: u16) -> Option<Position> {
    match slot_id {
        ESPN_SLOT_C => Some(Position::Catcher),
        ESPN_SLOT_1B => Some(Position::FirstBase),
        ESPN_SLOT_2B => Some(Position::SecondBase),
        ESPN_SLOT_3B => Some(Position::ThirdBase),
        ESPN_SLOT_SS => Some(Position::ShortStop),
        ESPN_SLOT_LF => Some(Position::LeftField),
        ESPN_SLOT_CF => Some(Position::CenterField),
        ESPN_SLOT_RF => Some(Position::RightField),
        ESPN_SLOT_DH => Some(Position::DesignatedHitter),
        ESPN_SLOT_UTIL => Some(Position::Utility),
        ESPN_SLOT_SP => Some(Position::StartingPitcher),
        ESPN_SLOT_RP => Some(Position::ReliefPitcher),
        ESPN_SLOT_BE => Some(Position::Bench),
        ESPN_SLOT_IL => Some(Position::InjuredList),
        _ => None, // Combo slots (MI, CI, P, generic OF) not directly mappable
    }
}

/// Expand an ESPN slot ID into all concrete positions it represents.
/// Regular slots return a single position; combo/generic slots expand to multiple.
pub fn positions_from_espn_slot(slot_id: u16) -> Vec<Position> {
    match slot_id {
        ESPN_SLOT_OF => vec![Position::LeftField, Position::CenterField, Position::RightField],
        ESPN_SLOT_MI => vec![Position::SecondBase, Position::ShortStop],
        ESPN_SLOT_CI => vec![Position::FirstBase, Position::ThirdBase],
        ESPN_SLOT_P => vec![Position::StartingPitcher, Position::ReliefPitcher],
        other => position_from_espn_slot(other).into_iter().collect(),
    }
}

/// Map a Position enum to its primary ESPN slot ID.
pub fn espn_slot_from_position(pos: Position) -> u16 {
    match pos {
        Position::Catcher => ESPN_SLOT_C,
        Position::FirstBase => ESPN_SLOT_1B,
        Position::SecondBase => ESPN_SLOT_2B,
        Position::ThirdBase => ESPN_SLOT_3B,
        Position::ShortStop => ESPN_SLOT_SS,
        Position::LeftField => ESPN_SLOT_LF,
        Position::CenterField => ESPN_SLOT_CF,
        Position::RightField => ESPN_SLOT_RF,
        Position::DesignatedHitter => ESPN_SLOT_DH,
        Position::Utility => ESPN_SLOT_UTIL,
        Position::StartingPitcher => ESPN_SLOT_SP,
        Position::ReliefPitcher => ESPN_SLOT_RP,
        Position::Bench => ESPN_SLOT_BE,
        Position::InjuredList => ESPN_SLOT_IL,
    }
}

/// Extract all concrete playing positions from ESPN eligible slots,
/// filtering out meta-slots (UTIL, BE, IL) and combo slots.
pub fn playing_positions_from_slots(eligible_slots: &[u16]) -> Vec<Position> {
    eligible_slots
        .iter()
        .flat_map(|&slot_id| positions_from_espn_slot(slot_id))
        .filter(|pos| !pos.is_meta_slot())
        .collect()
}

/// A single draft pick record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPick {
    /// Sequential pick number (1-indexed).
    pub pick_number: u32,
    /// ID of the team that won the player.
    pub team_id: String,
    /// Display name of the team.
    pub team_name: String,
    /// Name of the drafted player.
    pub player_name: String,
    /// The position string as reported by ESPN (e.g. "SP", "1B", "OF").
    pub position: String,
    /// Auction price paid for the player.
    pub price: u32,
    /// ESPN external player ID, if available.
    pub espn_player_id: Option<String>,
    /// ESPN eligible slot IDs for multi-position awareness.
    /// Empty if not available (manual entry, old data).
    #[serde(default)]
    pub eligible_slots: Vec<u16>,
    /// The roster slot ESPN assigned this player to (e.g. "UTIL", "BE", "SS").
    /// When present, this is used directly instead of running local slot
    /// assignment logic, so the app matches ESPN's placement exactly.
    #[serde(default)]
    pub roster_slot: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_pos_standard_positions() {
        assert_eq!(Position::from_str_pos("C"), Some(Position::Catcher));
        assert_eq!(Position::from_str_pos("SS"), Some(Position::ShortStop));
        assert_eq!(Position::from_str_pos("SP"), Some(Position::StartingPitcher));
        assert_eq!(Position::from_str_pos("RP"), Some(Position::ReliefPitcher));
        assert_eq!(Position::from_str_pos("LF"), Some(Position::LeftField));
        assert_eq!(Position::from_str_pos("CF"), Some(Position::CenterField));
        assert_eq!(Position::from_str_pos("RF"), Some(Position::RightField));
    }

    #[test]
    fn from_str_pos_numbered_bases() {
        assert_eq!(Position::from_str_pos("1B"), Some(Position::FirstBase));
        assert_eq!(Position::from_str_pos("2B"), Some(Position::SecondBase));
        assert_eq!(Position::from_str_pos("3B"), Some(Position::ThirdBase));
    }

    #[test]
    fn from_str_pos_generic_outfield() {
        assert_eq!(Position::from_str_pos("OF"), Some(Position::CenterField));
    }

    #[test]
    fn from_str_pos_special_slots() {
        assert_eq!(Position::from_str_pos("UTIL"), Some(Position::Utility));
        assert_eq!(Position::from_str_pos("DH"), Some(Position::DesignatedHitter));
        assert_eq!(Position::from_str_pos("BE"), Some(Position::Bench));
        assert_eq!(Position::from_str_pos("BN"), Some(Position::Bench));
        assert_eq!(Position::from_str_pos("IL"), Some(Position::InjuredList));
        assert_eq!(Position::from_str_pos("DL"), Some(Position::InjuredList));
    }

    #[test]
    fn from_str_pos_case_insensitive() {
        assert_eq!(Position::from_str_pos("sp"), Some(Position::StartingPitcher));
        assert_eq!(Position::from_str_pos("Ss"), Some(Position::ShortStop));
        assert_eq!(Position::from_str_pos("1b"), Some(Position::FirstBase));
        assert_eq!(Position::from_str_pos("util"), Some(Position::Utility));
    }

    #[test]
    fn from_str_pos_invalid() {
        assert_eq!(Position::from_str_pos("XX"), None);
        assert_eq!(Position::from_str_pos(""), None);
        assert_eq!(Position::from_str_pos("4B"), None);
    }

    #[test]
    fn display_str_roundtrip() {
        let positions = [
            Position::Catcher,
            Position::FirstBase,
            Position::SecondBase,
            Position::ThirdBase,
            Position::ShortStop,
            Position::LeftField,
            Position::CenterField,
            Position::RightField,
            Position::StartingPitcher,
            Position::ReliefPitcher,
            Position::DesignatedHitter,
            Position::Utility,
            Position::Bench,
            Position::InjuredList,
        ];
        for pos in positions {
            let s = pos.display_str();
            let parsed = Position::from_str_pos(s);
            assert_eq!(parsed, Some(pos), "Roundtrip failed for {}", s);
        }
    }

    #[test]
    fn is_hitter_correct() {
        assert!(Position::Catcher.is_hitter());
        assert!(Position::FirstBase.is_hitter());
        assert!(Position::SecondBase.is_hitter());
        assert!(Position::ThirdBase.is_hitter());
        assert!(Position::ShortStop.is_hitter());
        assert!(Position::LeftField.is_hitter());
        assert!(Position::CenterField.is_hitter());
        assert!(Position::RightField.is_hitter());
        assert!(Position::DesignatedHitter.is_hitter());
        assert!(Position::Utility.is_hitter());
        assert!(!Position::StartingPitcher.is_hitter());
        assert!(!Position::ReliefPitcher.is_hitter());
        assert!(!Position::Bench.is_hitter());
        assert!(!Position::InjuredList.is_hitter());
    }

    #[test]
    fn display_trait_works() {
        assert_eq!(format!("{}", Position::FirstBase), "1B");
        assert_eq!(format!("{}", Position::SecondBase), "2B");
        assert_eq!(format!("{}", Position::ThirdBase), "3B");
        assert_eq!(format!("{}", Position::StartingPitcher), "SP");
    }

    #[test]
    fn draft_pick_creation() {
        let pick = DraftPick {
            pick_number: 1,
            team_id: "team_1".to_string(),
            team_name: "My Team".to_string(),
            player_name: "Mike Trout".to_string(),
            position: "OF".to_string(),
            price: 45,
            espn_player_id: Some("12345".to_string()),
            eligible_slots: vec![],
            roster_slot: None,
        };
        assert_eq!(pick.pick_number, 1);
        assert_eq!(pick.price, 45);
        assert_eq!(pick.position, "OF");
        assert_eq!(pick.espn_player_id, Some("12345".to_string()));
        assert!(pick.eligible_slots.is_empty());
        assert!(pick.roster_slot.is_none());
    }

    // -- ESPN slot ID mapping tests --

    #[test]
    fn position_from_espn_slot_standard_positions() {
        assert_eq!(position_from_espn_slot(ESPN_SLOT_C), Some(Position::Catcher));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_1B), Some(Position::FirstBase));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_2B), Some(Position::SecondBase));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_3B), Some(Position::ThirdBase));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_SS), Some(Position::ShortStop));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_LF), Some(Position::LeftField));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_CF), Some(Position::CenterField));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_RF), Some(Position::RightField));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_DH), Some(Position::DesignatedHitter));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_UTIL), Some(Position::Utility));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_SP), Some(Position::StartingPitcher));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_RP), Some(Position::ReliefPitcher));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_BE), Some(Position::Bench));
        assert_eq!(position_from_espn_slot(ESPN_SLOT_IL), Some(Position::InjuredList));
    }

    #[test]
    fn position_from_espn_slot_combo_slots_return_none() {
        assert_eq!(position_from_espn_slot(ESPN_SLOT_OF), None);
        assert_eq!(position_from_espn_slot(ESPN_SLOT_MI), None);
        assert_eq!(position_from_espn_slot(ESPN_SLOT_CI), None);
        assert_eq!(position_from_espn_slot(ESPN_SLOT_P), None);
    }

    #[test]
    fn position_from_espn_slot_unknown_returns_none() {
        assert_eq!(position_from_espn_slot(99), None);
        assert_eq!(position_from_espn_slot(255), None);
    }

    #[test]
    fn espn_slot_from_position_roundtrip() {
        let positions = [
            Position::Catcher,
            Position::FirstBase,
            Position::SecondBase,
            Position::ThirdBase,
            Position::ShortStop,
            Position::LeftField,
            Position::CenterField,
            Position::RightField,
            Position::DesignatedHitter,
            Position::Utility,
            Position::StartingPitcher,
            Position::ReliefPitcher,
            Position::Bench,
            Position::InjuredList,
        ];
        for pos in positions {
            let slot_id = espn_slot_from_position(pos);
            let roundtripped = position_from_espn_slot(slot_id);
            assert_eq!(roundtripped, Some(pos), "Roundtrip failed for {:?} (slot {})", pos, slot_id);
        }
    }

    #[test]
    fn playing_positions_from_slots_filters_meta_slots() {
        // Mookie Betts: SS, 2B, OF, LF, CF, RF, DH, UTIL, BE, IL
        let slots = vec![
            ESPN_SLOT_SS, ESPN_SLOT_2B, ESPN_SLOT_OF, ESPN_SLOT_LF,
            ESPN_SLOT_CF, ESPN_SLOT_RF, ESPN_SLOT_DH, ESPN_SLOT_UTIL,
            ESPN_SLOT_BE, ESPN_SLOT_IL,
        ];
        let positions = playing_positions_from_slots(&slots);
        // Should include SS, 2B, LF, CF, RF, DH
        // OF combo slot (5) expands to LF, CF, RF — producing duplicates with the individual slots
        assert!(positions.contains(&Position::ShortStop));
        assert!(positions.contains(&Position::SecondBase));
        assert!(positions.contains(&Position::LeftField));
        assert!(positions.contains(&Position::CenterField));
        assert!(positions.contains(&Position::RightField));
        assert!(positions.contains(&Position::DesignatedHitter));
        assert!(!positions.contains(&Position::Utility));
        assert!(!positions.contains(&Position::Bench));
        assert!(!positions.contains(&Position::InjuredList));
        // SS(1) + 2B(1) + OF->LF,CF,RF(3) + LF(1) + CF(1) + RF(1) + DH(1) = 9
        assert_eq!(positions.len(), 9);
    }

    #[test]
    fn playing_positions_from_slots_expands_combo_slots() {
        // Player with only OF combo slot (5) — should expand to LF, CF, RF
        let slots = vec![ESPN_SLOT_OF, ESPN_SLOT_UTIL, ESPN_SLOT_BE, ESPN_SLOT_IL];
        let positions = playing_positions_from_slots(&slots);
        assert!(positions.contains(&Position::LeftField));
        assert!(positions.contains(&Position::CenterField));
        assert!(positions.contains(&Position::RightField));
        assert!(!positions.contains(&Position::Utility));
        assert!(!positions.contains(&Position::Bench));
        assert!(!positions.contains(&Position::InjuredList));
        assert_eq!(positions.len(), 3);
    }

    #[test]
    fn playing_positions_from_slots_empty() {
        let positions = playing_positions_from_slots(&[]);
        assert!(positions.is_empty());
    }

    #[test]
    fn playing_positions_from_slots_pitcher() {
        let slots = vec![ESPN_SLOT_SP, ESPN_SLOT_P, ESPN_SLOT_BE, ESPN_SLOT_IL];
        let positions = playing_positions_from_slots(&slots);
        // SP survives directly; P combo expands to SP+RP; BE and IL are meta (filtered out)
        assert!(positions.contains(&Position::StartingPitcher));
        assert!(positions.contains(&Position::ReliefPitcher));
        // SP(1) + P->SP,RP(2) = 3
        assert_eq!(positions.len(), 3);
    }

    // -- positions_from_espn_slot tests --

    #[test]
    fn positions_from_espn_slot_generic_of() {
        let positions = positions_from_espn_slot(ESPN_SLOT_OF);
        assert_eq!(positions, vec![Position::LeftField, Position::CenterField, Position::RightField]);
    }

    #[test]
    fn positions_from_espn_slot_mi() {
        let positions = positions_from_espn_slot(ESPN_SLOT_MI);
        assert_eq!(positions, vec![Position::SecondBase, Position::ShortStop]);
    }

    #[test]
    fn positions_from_espn_slot_ci() {
        let positions = positions_from_espn_slot(ESPN_SLOT_CI);
        assert_eq!(positions, vec![Position::FirstBase, Position::ThirdBase]);
    }

    #[test]
    fn positions_from_espn_slot_generic_pitcher() {
        let positions = positions_from_espn_slot(ESPN_SLOT_P);
        assert_eq!(positions, vec![Position::StartingPitcher, Position::ReliefPitcher]);
    }

    #[test]
    fn positions_from_espn_slot_regular_slot() {
        // Regular slots should return a single-element vec
        assert_eq!(positions_from_espn_slot(ESPN_SLOT_C), vec![Position::Catcher]);
        assert_eq!(positions_from_espn_slot(ESPN_SLOT_SS), vec![Position::ShortStop]);
        assert_eq!(positions_from_espn_slot(ESPN_SLOT_SP), vec![Position::StartingPitcher]);
        assert_eq!(positions_from_espn_slot(ESPN_SLOT_BE), vec![Position::Bench]);
    }

    #[test]
    fn positions_from_espn_slot_unknown() {
        // Unknown slots should return an empty vec
        assert!(positions_from_espn_slot(99).is_empty());
        assert!(positions_from_espn_slot(255).is_empty());
    }

    // -- is_meta_slot tests --

    #[test]
    fn is_meta_slot_true_for_meta_positions() {
        assert!(Position::Utility.is_meta_slot());
        assert!(Position::Bench.is_meta_slot());
        assert!(Position::InjuredList.is_meta_slot());
    }

    #[test]
    fn is_meta_slot_false_for_playing_positions() {
        assert!(!Position::Catcher.is_meta_slot());
        assert!(!Position::FirstBase.is_meta_slot());
        assert!(!Position::SecondBase.is_meta_slot());
        assert!(!Position::ThirdBase.is_meta_slot());
        assert!(!Position::ShortStop.is_meta_slot());
        assert!(!Position::LeftField.is_meta_slot());
        assert!(!Position::CenterField.is_meta_slot());
        assert!(!Position::RightField.is_meta_slot());
        assert!(!Position::DesignatedHitter.is_meta_slot());
        assert!(!Position::StartingPitcher.is_meta_slot());
        assert!(!Position::ReliefPitcher.is_meta_slot());
    }
}
