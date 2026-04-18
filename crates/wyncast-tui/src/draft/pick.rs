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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    /// Generic OF roster slot — accepts LF, CF, RF.
    Outfield,
    /// MI combo slot — accepts 2B, SS.
    MiddleInfield,
    /// CI combo slot — accepts 1B, 3B.
    CornerInfield,
    /// Generic P slot — accepts SP, RP.
    GenericPitcher,
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
            Position::Outfield => "OF",
            Position::MiddleInfield => "MI",
            Position::CornerInfield => "CI",
            Position::GenericPitcher => "P",
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
                | Position::Outfield
                | Position::MiddleInfield
                | Position::CornerInfield
        )
    }

    /// Whether this is a meta-slot (not a concrete playing position).
    pub fn is_meta_slot(&self) -> bool {
        matches!(
            self,
            Position::Utility | Position::Bench | Position::InjuredList
        )
    }

    /// Whether this is a combo roster slot (OF, MI, CI, P).
    pub fn is_combo_slot(&self) -> bool {
        matches!(
            self,
            Position::Outfield
                | Position::MiddleInfield
                | Position::CornerInfield
                | Position::GenericPitcher
        )
    }

    /// Concrete positions that a slot accepts.
    ///
    /// For regular positions, returns `vec![self]`.
    /// For combo slots, returns the constituent positions:
    /// - Outfield → [LF, CF, RF]
    /// - MiddleInfield → [2B, SS]
    /// - CornerInfield → [1B, 3B]
    /// - GenericPitcher → [SP, RP]
    pub fn accepted_positions(&self) -> Vec<Position> {
        match self {
            Position::Outfield => vec![
                Position::LeftField,
                Position::CenterField,
                Position::RightField,
            ],
            Position::MiddleInfield => vec![Position::SecondBase, Position::ShortStop],
            Position::CornerInfield => vec![Position::FirstBase, Position::ThirdBase],
            Position::GenericPitcher => vec![Position::StartingPitcher, Position::ReliefPitcher],
            other => vec![*other],
        }
    }

    /// Parse a roster slot string into a Position enum.
    ///
    /// Like `from_str_pos()` but maps combo slot strings to their combo variants:
    /// - "OF" → Outfield (not CenterField)
    /// - "MI" → MiddleInfield
    /// - "CI" → CornerInfield
    /// - "P"  → GenericPitcher
    ///
    /// All other strings delegate to `from_str_pos()`.
    pub fn from_roster_slot_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "OF" => Some(Position::Outfield),
            "MI" => Some(Position::MiddleInfield),
            "CI" => Some(Position::CornerInfield),
            "P" => Some(Position::GenericPitcher),
            other => Self::from_str_pos(other),
        }
    }

    /// Deterministic ordering index for roster slot display.
    pub fn sort_order(&self) -> u8 {
        match self {
            Position::Catcher => 0,
            Position::FirstBase => 1,
            Position::SecondBase => 2,
            Position::ThirdBase => 3,
            Position::CornerInfield => 4,
            Position::ShortStop => 5,
            Position::MiddleInfield => 6,
            Position::LeftField => 7,
            Position::CenterField => 8,
            Position::RightField => 9,
            Position::Outfield => 10,
            Position::DesignatedHitter => 11,
            Position::Utility => 12,
            Position::StartingPitcher => 13,
            Position::ReliefPitcher => 14,
            Position::GenericPitcher => 15,
            Position::Bench => 16,
            Position::InjuredList => 17,
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
/// Returns the combo variant for combo slots (OF, MI, CI, P).
pub fn position_from_espn_slot(slot_id: u16) -> Option<Position> {
    match slot_id {
        ESPN_SLOT_C => Some(Position::Catcher),
        ESPN_SLOT_1B => Some(Position::FirstBase),
        ESPN_SLOT_2B => Some(Position::SecondBase),
        ESPN_SLOT_3B => Some(Position::ThirdBase),
        ESPN_SLOT_SS => Some(Position::ShortStop),
        ESPN_SLOT_OF => Some(Position::Outfield),
        ESPN_SLOT_MI => Some(Position::MiddleInfield),
        ESPN_SLOT_CI => Some(Position::CornerInfield),
        ESPN_SLOT_LF => Some(Position::LeftField),
        ESPN_SLOT_CF => Some(Position::CenterField),
        ESPN_SLOT_RF => Some(Position::RightField),
        ESPN_SLOT_DH => Some(Position::DesignatedHitter),
        ESPN_SLOT_UTIL => Some(Position::Utility),
        ESPN_SLOT_P => Some(Position::GenericPitcher),
        ESPN_SLOT_SP => Some(Position::StartingPitcher),
        ESPN_SLOT_RP => Some(Position::ReliefPitcher),
        ESPN_SLOT_BE => Some(Position::Bench),
        ESPN_SLOT_IL => Some(Position::InjuredList),
        _ => None,
    }
}

/// Expand an ESPN slot ID into all concrete positions it represents.
/// Regular slots return a single position; combo/generic slots expand to multiple.
pub fn positions_from_espn_slot(slot_id: u16) -> Vec<Position> {
    match slot_id {
        ESPN_SLOT_OF => vec![
            Position::LeftField,
            Position::CenterField,
            Position::RightField,
        ],
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
        Position::Outfield => ESPN_SLOT_OF,
        Position::MiddleInfield => ESPN_SLOT_MI,
        Position::CornerInfield => ESPN_SLOT_CI,
        Position::GenericPitcher => ESPN_SLOT_P,
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

/// Map a position string (e.g. "C", "1B", "SP", "BE", "OF", "MI") to the ESPN slot ID.
///
/// This is the Rust equivalent of `espnSlotIdFromPositionStr()` in the
/// extension. Useful for converting draft board `rosterSlot` strings into
/// ESPN slot IDs for roster placement.
///
/// Uses `from_roster_slot_str()` so combo slot strings ("OF", "MI", "CI", "P")
/// map to their proper ESPN slot IDs (5, 6, 7, 13).
pub fn espn_slot_from_position_str(s: &str) -> Option<u16> {
    Position::from_roster_slot_str(s).map(espn_slot_from_position)
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
    /// The ESPN roster slot ID that ESPN actually assigned this player to
    /// when the pick was made (e.g. 12 for UTIL, 14 for SP). When present,
    /// this overrides position-inference logic so two-way players like Ohtani
    /// land in the correct slot (e.g. UTIL instead of SP).
    /// None if not reported by the extension (old data, DOM-only scraping).
    #[serde(default)]
    pub assigned_slot: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_pos_standard_positions() {
        assert_eq!(Position::from_str_pos("C"), Some(Position::Catcher));
        assert_eq!(Position::from_str_pos("SS"), Some(Position::ShortStop));
        assert_eq!(
            Position::from_str_pos("SP"),
            Some(Position::StartingPitcher)
        );
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
        assert_eq!(
            Position::from_str_pos("DH"),
            Some(Position::DesignatedHitter)
        );
        assert_eq!(Position::from_str_pos("BE"), Some(Position::Bench));
        assert_eq!(Position::from_str_pos("BN"), Some(Position::Bench));
        assert_eq!(Position::from_str_pos("IL"), Some(Position::InjuredList));
        assert_eq!(Position::from_str_pos("DL"), Some(Position::InjuredList));
    }

    #[test]
    fn from_str_pos_case_insensitive() {
        assert_eq!(
            Position::from_str_pos("sp"),
            Some(Position::StartingPitcher)
        );
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
        // Non-combo positions roundtrip through from_str_pos
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
    fn display_str_roundtrip_combo_variants() {
        // Combo variants roundtrip through from_roster_slot_str (not from_str_pos)
        let combo_positions = [
            Position::Outfield,
            Position::MiddleInfield,
            Position::CornerInfield,
            Position::GenericPitcher,
        ];
        for pos in combo_positions {
            let s = pos.display_str();
            let parsed = Position::from_roster_slot_str(s);
            assert_eq!(
                parsed,
                Some(pos),
                "Combo roundtrip failed for {}",
                s
            );
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
        assert!(Position::Outfield.is_hitter());
        assert!(Position::MiddleInfield.is_hitter());
        assert!(Position::CornerInfield.is_hitter());
        assert!(!Position::StartingPitcher.is_hitter());
        assert!(!Position::ReliefPitcher.is_hitter());
        assert!(!Position::GenericPitcher.is_hitter());
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
            assigned_slot: None,
        };
        assert_eq!(pick.pick_number, 1);
        assert_eq!(pick.price, 45);
        assert_eq!(pick.position, "OF");
        assert_eq!(pick.espn_player_id, Some("12345".to_string()));
        assert!(pick.eligible_slots.is_empty());
    }

    // -- ESPN slot ID mapping tests --

    #[test]
    fn position_from_espn_slot_standard_positions() {
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_C),
            Some(Position::Catcher)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_1B),
            Some(Position::FirstBase)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_2B),
            Some(Position::SecondBase)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_3B),
            Some(Position::ThirdBase)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_SS),
            Some(Position::ShortStop)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_LF),
            Some(Position::LeftField)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_CF),
            Some(Position::CenterField)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_RF),
            Some(Position::RightField)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_DH),
            Some(Position::DesignatedHitter)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_UTIL),
            Some(Position::Utility)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_SP),
            Some(Position::StartingPitcher)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_RP),
            Some(Position::ReliefPitcher)
        );
        assert_eq!(position_from_espn_slot(ESPN_SLOT_BE), Some(Position::Bench));
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_IL),
            Some(Position::InjuredList)
        );
    }

    #[test]
    fn position_from_espn_slot_combo_slots_return_combo_variants() {
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_OF),
            Some(Position::Outfield)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_MI),
            Some(Position::MiddleInfield)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_CI),
            Some(Position::CornerInfield)
        );
        assert_eq!(
            position_from_espn_slot(ESPN_SLOT_P),
            Some(Position::GenericPitcher)
        );
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
            Position::Outfield,
            Position::MiddleInfield,
            Position::CornerInfield,
            Position::GenericPitcher,
        ];
        for pos in positions {
            let slot_id = espn_slot_from_position(pos);
            let roundtripped = position_from_espn_slot(slot_id);
            assert_eq!(
                roundtripped,
                Some(pos),
                "Roundtrip failed for {:?} (slot {})",
                pos,
                slot_id
            );
        }
    }

    #[test]
    fn playing_positions_from_slots_filters_meta_slots() {
        // Mookie Betts: SS, 2B, OF, LF, CF, RF, DH, UTIL, BE, IL
        let slots = vec![
            ESPN_SLOT_SS,
            ESPN_SLOT_2B,
            ESPN_SLOT_OF,
            ESPN_SLOT_LF,
            ESPN_SLOT_CF,
            ESPN_SLOT_RF,
            ESPN_SLOT_DH,
            ESPN_SLOT_UTIL,
            ESPN_SLOT_BE,
            ESPN_SLOT_IL,
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
        assert_eq!(
            positions,
            vec![
                Position::LeftField,
                Position::CenterField,
                Position::RightField
            ]
        );
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
        assert_eq!(
            positions,
            vec![Position::StartingPitcher, Position::ReliefPitcher]
        );
    }

    #[test]
    fn positions_from_espn_slot_regular_slot() {
        // Regular slots should return a single-element vec
        assert_eq!(
            positions_from_espn_slot(ESPN_SLOT_C),
            vec![Position::Catcher]
        );
        assert_eq!(
            positions_from_espn_slot(ESPN_SLOT_SS),
            vec![Position::ShortStop]
        );
        assert_eq!(
            positions_from_espn_slot(ESPN_SLOT_SP),
            vec![Position::StartingPitcher]
        );
        assert_eq!(
            positions_from_espn_slot(ESPN_SLOT_BE),
            vec![Position::Bench]
        );
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
        assert!(!Position::Outfield.is_meta_slot());
        assert!(!Position::MiddleInfield.is_meta_slot());
        assert!(!Position::CornerInfield.is_meta_slot());
        assert!(!Position::GenericPitcher.is_meta_slot());
    }

    // -- from_roster_slot_str tests --

    #[test]
    fn from_roster_slot_str_combo_slots() {
        assert_eq!(
            Position::from_roster_slot_str("OF"),
            Some(Position::Outfield)
        );
        assert_eq!(
            Position::from_roster_slot_str("MI"),
            Some(Position::MiddleInfield)
        );
        assert_eq!(
            Position::from_roster_slot_str("CI"),
            Some(Position::CornerInfield)
        );
        assert_eq!(
            Position::from_roster_slot_str("P"),
            Some(Position::GenericPitcher)
        );
    }

    #[test]
    fn from_roster_slot_str_case_insensitive() {
        assert_eq!(
            Position::from_roster_slot_str("of"),
            Some(Position::Outfield)
        );
        assert_eq!(
            Position::from_roster_slot_str("mi"),
            Some(Position::MiddleInfield)
        );
        assert_eq!(
            Position::from_roster_slot_str("ci"),
            Some(Position::CornerInfield)
        );
        assert_eq!(
            Position::from_roster_slot_str("p"),
            Some(Position::GenericPitcher)
        );
    }

    #[test]
    fn from_roster_slot_str_delegates_non_combo() {
        assert_eq!(
            Position::from_roster_slot_str("C"),
            Some(Position::Catcher)
        );
        assert_eq!(
            Position::from_roster_slot_str("1B"),
            Some(Position::FirstBase)
        );
        assert_eq!(
            Position::from_roster_slot_str("SP"),
            Some(Position::StartingPitcher)
        );
        assert_eq!(
            Position::from_roster_slot_str("BE"),
            Some(Position::Bench)
        );
        assert_eq!(Position::from_roster_slot_str("XX"), None);
    }

    // -- is_combo_slot tests --

    #[test]
    fn is_combo_slot_true_for_combo_variants() {
        assert!(Position::Outfield.is_combo_slot());
        assert!(Position::MiddleInfield.is_combo_slot());
        assert!(Position::CornerInfield.is_combo_slot());
        assert!(Position::GenericPitcher.is_combo_slot());
    }

    #[test]
    fn is_combo_slot_false_for_regular_positions() {
        assert!(!Position::Catcher.is_combo_slot());
        assert!(!Position::FirstBase.is_combo_slot());
        assert!(!Position::SecondBase.is_combo_slot());
        assert!(!Position::ThirdBase.is_combo_slot());
        assert!(!Position::ShortStop.is_combo_slot());
        assert!(!Position::LeftField.is_combo_slot());
        assert!(!Position::CenterField.is_combo_slot());
        assert!(!Position::RightField.is_combo_slot());
        assert!(!Position::StartingPitcher.is_combo_slot());
        assert!(!Position::ReliefPitcher.is_combo_slot());
        assert!(!Position::DesignatedHitter.is_combo_slot());
        assert!(!Position::Utility.is_combo_slot());
        assert!(!Position::Bench.is_combo_slot());
        assert!(!Position::InjuredList.is_combo_slot());
    }

    // -- accepted_positions tests --

    #[test]
    fn accepted_positions_combo_slots() {
        assert_eq!(
            Position::Outfield.accepted_positions(),
            vec![Position::LeftField, Position::CenterField, Position::RightField]
        );
        assert_eq!(
            Position::MiddleInfield.accepted_positions(),
            vec![Position::SecondBase, Position::ShortStop]
        );
        assert_eq!(
            Position::CornerInfield.accepted_positions(),
            vec![Position::FirstBase, Position::ThirdBase]
        );
        assert_eq!(
            Position::GenericPitcher.accepted_positions(),
            vec![Position::StartingPitcher, Position::ReliefPitcher]
        );
    }

    #[test]
    fn accepted_positions_regular_slots() {
        assert_eq!(
            Position::Catcher.accepted_positions(),
            vec![Position::Catcher]
        );
        assert_eq!(
            Position::FirstBase.accepted_positions(),
            vec![Position::FirstBase]
        );
        assert_eq!(
            Position::StartingPitcher.accepted_positions(),
            vec![Position::StartingPitcher]
        );
        assert_eq!(
            Position::Bench.accepted_positions(),
            vec![Position::Bench]
        );
    }

    // -- espn_slot_from_position_str with combo slots --

    #[test]
    fn espn_slot_from_position_str_combo_slots() {
        assert_eq!(espn_slot_from_position_str("OF"), Some(ESPN_SLOT_OF));
        assert_eq!(espn_slot_from_position_str("MI"), Some(ESPN_SLOT_MI));
        assert_eq!(espn_slot_from_position_str("CI"), Some(ESPN_SLOT_CI));
        assert_eq!(espn_slot_from_position_str("P"), Some(ESPN_SLOT_P));
    }
}
