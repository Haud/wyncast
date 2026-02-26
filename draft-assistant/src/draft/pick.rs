// Individual pick representation and processing.

use serde::{Deserialize, Serialize};
use std::fmt;

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
        };
        assert_eq!(pick.pick_number, 1);
        assert_eq!(pick.price, 45);
        assert_eq!(pick.position, "OF");
        assert_eq!(pick.espn_player_id, Some("12345".to_string()));
    }
}
