// Core draft pick data type (used by persistence layer and protocol).

use serde::{Deserialize, Serialize};

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
