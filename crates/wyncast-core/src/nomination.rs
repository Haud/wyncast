// Nomination types shared between the baseball domain and the app orchestrator.
//
// Placed in wyncast-core so that wyncast-baseball (prompt construction) and
// wyncast-tui (protocol/UiUpdate) can both reference NominationInfo without
// a circular dependency.

/// Info about the current active nomination during an auction draft.
#[derive(Debug, Clone, PartialEq)]
pub struct NominationInfo {
    pub player_name: String,
    pub position: String,
    pub nominated_by: String,
    pub current_bid: u32,
    pub current_bidder: Option<String>,
    pub time_remaining: Option<u32>,
    pub eligible_slots: Vec<u16>,
}
