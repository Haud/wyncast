// ESPN projection types (player projections scraped from ESPN Fantasy API).
//
// Placed in wyncast-core so that both wyncast-baseball (which converts them to
// internal projections) and wyncast-tui (which receives them via WebSocket) can
// reference these types without a circular dependency.

use serde::{Deserialize, Serialize};

use crate::stats::ProjectionData;

/// Player projections scraped from ESPN's Fantasy API by the extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EspnProjectionsPayload {
    pub players: Vec<EspnPlayerProjection>,
}

/// A single player's projection data from ESPN.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EspnPlayerProjection {
    /// ESPN's internal player ID.
    pub espn_id: u32,
    pub name: String,
    pub team: String,
    /// ESPN defaultPositionId (1=SP, 2=C, 3=1B, 4=2B, 5=3B, 6=SS, 7=LF, 8=CF, 9=RF, 10=DH, 11=RP).
    pub default_position_id: u16,
    /// ESPN eligible slot IDs for multi-position eligibility.
    #[serde(default)]
    pub eligible_slots: Vec<u16>,
    /// Projected batting stats (None if player is pitcher-only).
    pub batting: Option<EspnBattingProjection>,
    /// Projected pitching stats (None if player is hitter-only).
    pub pitching: Option<EspnPitchingProjection>,
}

/// Projected batting stats from ESPN.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EspnBattingProjection {
    pub pa: u32,
    pub ab: u32,
    pub h: u32,
    pub hr: u32,
    pub r: u32,
    pub rbi: u32,
    pub bb: u32,
    pub sb: u32,
    pub avg: f64,
}

/// Projected pitching stats from ESPN.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EspnPitchingProjection {
    pub ip: f64,
    pub k: u32,
    pub w: u32,
    pub sv: u32,
    pub hd: u32,
    pub era: f64,
    pub whip: f64,
    pub g: u32,
    pub gs: u32,
}

impl From<&EspnBattingProjection> for ProjectionData {
    fn from(proj: &EspnBattingProjection) -> Self {
        let mut data = ProjectionData::new();
        data.insert("pa", f64::from(proj.pa));
        data.insert("ab", f64::from(proj.ab));
        data.insert("h", f64::from(proj.h));
        data.insert("hr", f64::from(proj.hr));
        data.insert("r", f64::from(proj.r));
        data.insert("rbi", f64::from(proj.rbi));
        data.insert("bb", f64::from(proj.bb));
        data.insert("sb", f64::from(proj.sb));
        data.insert("avg", proj.avg);
        data
    }
}

impl From<&EspnPitchingProjection> for ProjectionData {
    fn from(proj: &EspnPitchingProjection) -> Self {
        let mut data = ProjectionData::new();
        data.insert("ip", proj.ip);
        data.insert("k", f64::from(proj.k));
        data.insert("w", f64::from(proj.w));
        data.insert("sv", f64::from(proj.sv));
        data.insert("hd", f64::from(proj.hd));
        data.insert("era", proj.era);
        data.insert("whip", proj.whip);
        data.insert("g", f64::from(proj.g));
        data.insert("gs", f64::from(proj.gs));
        if proj.ip > 0.0 {
            data.insert("k9", f64::from(proj.k) * 9.0 / proj.ip);
        }
        data
    }
}
