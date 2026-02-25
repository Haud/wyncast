// Individual pick representation and processing.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Baseball positions used for roster slot assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Position {
    C,
    FB,   // 1B
    SB,   // 2B
    TB,   // 3B
    SS,
    LF,
    CF,
    RF,
    SP,
    RP,
    DH,
    UTIL,
    BE,
    IL,
}

impl Position {
    /// Parse a position string into a Position enum.
    ///
    /// Handles ESPN-style abbreviations:
    /// - "1B" -> FB, "2B" -> SB, "3B" -> TB
    /// - "OF" -> CF (generic outfield maps to CF slot)
    /// - "DH" -> DH, "UTIL" -> UTIL, "BE"/"BN" -> BE, "IL"/"DL" -> IL
    pub fn from_str_pos(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "C" => Some(Position::C),
            "1B" => Some(Position::FB),
            "2B" => Some(Position::SB),
            "3B" => Some(Position::TB),
            "SS" => Some(Position::SS),
            "LF" => Some(Position::LF),
            "CF" => Some(Position::CF),
            "RF" => Some(Position::RF),
            "OF" => Some(Position::CF), // Generic outfield -> CF
            "SP" => Some(Position::SP),
            "RP" => Some(Position::RP),
            "DH" => Some(Position::DH),
            "UTIL" => Some(Position::UTIL),
            "BE" | "BN" => Some(Position::BE),
            "IL" | "DL" => Some(Position::IL),
            _ => None,
        }
    }

    /// Return the display string for this position.
    pub fn display_str(&self) -> &'static str {
        match self {
            Position::C => "C",
            Position::FB => "1B",
            Position::SB => "2B",
            Position::TB => "3B",
            Position::SS => "SS",
            Position::LF => "LF",
            Position::CF => "CF",
            Position::RF => "RF",
            Position::SP => "SP",
            Position::RP => "RP",
            Position::DH => "DH",
            Position::UTIL => "UTIL",
            Position::BE => "BE",
            Position::IL => "IL",
        }
    }

    /// Whether this position is a hitting position (not a pitcher).
    pub fn is_hitter(&self) -> bool {
        matches!(
            self,
            Position::C
                | Position::FB
                | Position::SB
                | Position::TB
                | Position::SS
                | Position::LF
                | Position::CF
                | Position::RF
                | Position::DH
                | Position::UTIL
        )
    }

    /// Deterministic ordering index for roster slot display.
    /// Positions are ordered: C, 1B, 2B, 3B, SS, LF, CF, RF, UTIL, DH, SP, RP, BE, IL.
    pub fn sort_order(&self) -> u8 {
        match self {
            Position::C => 0,
            Position::FB => 1,
            Position::SB => 2,
            Position::TB => 3,
            Position::SS => 4,
            Position::LF => 5,
            Position::CF => 6,
            Position::RF => 7,
            Position::UTIL => 8,
            Position::DH => 9,
            Position::SP => 10,
            Position::RP => 11,
            Position::BE => 12,
            Position::IL => 13,
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_str())
    }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_pos_standard_positions() {
        assert_eq!(Position::from_str_pos("C"), Some(Position::C));
        assert_eq!(Position::from_str_pos("SS"), Some(Position::SS));
        assert_eq!(Position::from_str_pos("SP"), Some(Position::SP));
        assert_eq!(Position::from_str_pos("RP"), Some(Position::RP));
        assert_eq!(Position::from_str_pos("LF"), Some(Position::LF));
        assert_eq!(Position::from_str_pos("CF"), Some(Position::CF));
        assert_eq!(Position::from_str_pos("RF"), Some(Position::RF));
    }

    #[test]
    fn from_str_pos_numbered_bases() {
        assert_eq!(Position::from_str_pos("1B"), Some(Position::FB));
        assert_eq!(Position::from_str_pos("2B"), Some(Position::SB));
        assert_eq!(Position::from_str_pos("3B"), Some(Position::TB));
    }

    #[test]
    fn from_str_pos_generic_outfield() {
        assert_eq!(Position::from_str_pos("OF"), Some(Position::CF));
    }

    #[test]
    fn from_str_pos_special_slots() {
        assert_eq!(Position::from_str_pos("UTIL"), Some(Position::UTIL));
        assert_eq!(Position::from_str_pos("DH"), Some(Position::DH));
        assert_eq!(Position::from_str_pos("BE"), Some(Position::BE));
        assert_eq!(Position::from_str_pos("BN"), Some(Position::BE));
        assert_eq!(Position::from_str_pos("IL"), Some(Position::IL));
        assert_eq!(Position::from_str_pos("DL"), Some(Position::IL));
    }

    #[test]
    fn from_str_pos_case_insensitive() {
        assert_eq!(Position::from_str_pos("sp"), Some(Position::SP));
        assert_eq!(Position::from_str_pos("Ss"), Some(Position::SS));
        assert_eq!(Position::from_str_pos("1b"), Some(Position::FB));
        assert_eq!(Position::from_str_pos("util"), Some(Position::UTIL));
    }

    #[test]
    fn from_str_pos_invalid() {
        assert_eq!(Position::from_str_pos("XX"), None);
        assert_eq!(Position::from_str_pos(""), None);
        assert_eq!(Position::from_str_pos("4B"), None);
    }

    #[test]
    fn display_str_roundtrip() {
        // For standard positions, from_str_pos(display_str()) should roundtrip
        let positions = [
            Position::C,
            Position::FB,
            Position::SB,
            Position::TB,
            Position::SS,
            Position::LF,
            Position::CF,
            Position::RF,
            Position::SP,
            Position::RP,
            Position::DH,
            Position::UTIL,
            Position::BE,
            Position::IL,
        ];
        for pos in positions {
            let s = pos.display_str();
            let parsed = Position::from_str_pos(s);
            assert_eq!(parsed, Some(pos), "Roundtrip failed for {}", s);
        }
    }

    #[test]
    fn is_hitter_correct() {
        assert!(Position::C.is_hitter());
        assert!(Position::FB.is_hitter());
        assert!(Position::SB.is_hitter());
        assert!(Position::TB.is_hitter());
        assert!(Position::SS.is_hitter());
        assert!(Position::LF.is_hitter());
        assert!(Position::CF.is_hitter());
        assert!(Position::RF.is_hitter());
        assert!(Position::DH.is_hitter());
        assert!(Position::UTIL.is_hitter());
        assert!(!Position::SP.is_hitter());
        assert!(!Position::RP.is_hitter());
        assert!(!Position::BE.is_hitter());
        assert!(!Position::IL.is_hitter());
    }

    #[test]
    fn display_trait_works() {
        assert_eq!(format!("{}", Position::FB), "1B");
        assert_eq!(format!("{}", Position::SB), "2B");
        assert_eq!(format!("{}", Position::TB), "3B");
        assert_eq!(format!("{}", Position::SP), "SP");
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
        };
        assert_eq!(pick.pick_number, 1);
        assert_eq!(pick.price, 45);
        assert_eq!(pick.position, "OF");
    }
}
