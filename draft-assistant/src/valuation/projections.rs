// Projection data loading and normalization.
//
// Reads Razzball-format CSV files: a single combined pitchers CSV with a POS
// column (SP/RP) and an HLD column containing real holds data.

use crate::config::{Config, DataPaths};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use tracing::warn;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Whether a pitcher is a starter or reliever (projection-specific).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PitcherType {
    SP,
    RP,
}

/// Projected season stats for a hitter.
///
/// Note: position eligibility is intentionally excluded from projections.
/// It will be sourced from ESPN roster data via a separate overlay.
#[derive(Debug, Clone)]
pub struct HitterProjection {
    pub name: String,
    pub team: String,
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

/// Projected season stats for a pitcher.
#[derive(Debug, Clone)]
pub struct PitcherProjection {
    pub name: String,
    pub team: String,
    pub pitcher_type: PitcherType,
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

/// All projection data loaded and ready for the valuation engine.
#[derive(Debug, Clone)]
pub struct AllProjections {
    pub hitters: Vec<HitterProjection>,
    pub pitchers: Vec<PitcherProjection>,
    pub adp: HashMap<String, f64>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error("failed to read file {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    #[error("CSV error in {path}: {source}")]
    Csv { path: String, source: csv::Error },

    #[error("validation error: {0}")]
    Validation(String),
}

// ---------------------------------------------------------------------------
// Raw CSV serde structs (private) — Razzball format
// ---------------------------------------------------------------------------

/// Razzball hitter CSV row. All counting stats are f64 because Razzball uses
/// fractional projections (e.g. 120.6 HR). Extra columns are silently ignored
/// via `#[serde(flatten)]`.
#[derive(Debug, Deserialize)]
#[allow(dead_code, non_snake_case)]
struct RawRazzballHitter {
    Name: String,
    #[serde(default)]
    Team: String,
    PA: f64,
    AB: f64,
    H: f64,
    HR: f64,
    R: f64,
    RBI: f64,
    BB: f64,
    SB: f64,
    #[serde(alias = "BA")]
    AVG: f64,
    /// Absorb any extra columns Razzball includes.
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

/// Razzball pitcher CSV row (combined SP+RP). The POS column determines
/// pitcher type. HLD is the Razzball column name for holds.
#[derive(Debug, Deserialize)]
#[allow(dead_code, non_snake_case)]
struct RawRazzballPitcher {
    Name: String,
    #[serde(default)]
    Team: String,
    POS: String,
    G: f64,
    #[serde(default)]
    GS: f64,
    IP: f64,
    W: f64,
    SV: f64,
    #[serde(alias = "HD")]
    HLD: f64,
    ERA: f64,
    WHIP: f64,
    #[serde(alias = "SO")]
    K: f64,
    /// Absorb any extra columns Razzball includes.
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code, non_snake_case)]
struct RawAdp {
    Name: String,
    ADP: f64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true if all given f64 values are finite (not NaN or Infinity).
fn all_finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

// ---------------------------------------------------------------------------
// Reader-based loaders (private, enable testing without temp files)
// ---------------------------------------------------------------------------

fn load_hitters_from_reader<R: Read>(rdr: R) -> Result<Vec<HitterProjection>, csv::Error> {
    let mut reader = csv::Reader::from_reader(rdr);
    let mut hitters = Vec::new();
    for result in reader.deserialize::<RawRazzballHitter>() {
        match result {
            Ok(raw) => {
                if !raw.AVG.is_finite() {
                    warn!("skipping hitter '{}': non-finite AVG value", raw.Name.trim());
                    continue;
                }
                hitters.push(HitterProjection {
                    name: raw.Name.trim().to_string(),
                    team: raw.Team.trim().to_string(),
                    pa: raw.PA.round() as u32,
                    ab: raw.AB.round() as u32,
                    h: raw.H.round() as u32,
                    hr: raw.HR.round() as u32,
                    r: raw.R.round() as u32,
                    rbi: raw.RBI.round() as u32,
                    bb: raw.BB.round() as u32,
                    sb: raw.SB.round() as u32,
                    avg: raw.AVG,
                });
            }
            Err(e) => {
                warn!("skipping malformed hitter row: {}", e);
            }
        }
    }
    Ok(hitters)
}

fn load_pitchers_from_reader<R: Read>(rdr: R) -> Result<Vec<PitcherProjection>, csv::Error> {
    let mut reader = csv::Reader::from_reader(rdr);
    let mut pitchers = Vec::new();
    for result in reader.deserialize::<RawRazzballPitcher>() {
        match result {
            Ok(raw) => {
                if !all_finite(&[raw.IP, raw.ERA, raw.WHIP]) {
                    warn!("skipping pitcher '{}': non-finite IP/ERA/WHIP value", raw.Name.trim());
                    continue;
                }
                let pos_str = raw.POS.trim().to_uppercase();
                let pitcher_type = if pos_str == "SP" {
                    PitcherType::SP
                } else if pos_str == "RP" {
                    PitcherType::RP
                } else {
                    warn!("skipping pitcher '{}': unknown POS '{}'", raw.Name.trim(), raw.POS);
                    continue;
                };
                pitchers.push(PitcherProjection {
                    name: raw.Name.trim().to_string(),
                    team: raw.Team.trim().to_string(),
                    pitcher_type,
                    ip: raw.IP,
                    k: raw.K.round() as u32,
                    w: raw.W.round() as u32,
                    sv: raw.SV.round() as u32,
                    hd: raw.HLD.round() as u32,
                    era: raw.ERA,
                    whip: raw.WHIP,
                    g: raw.G.round() as u32,
                    gs: raw.GS.round() as u32,
                });
            }
            Err(e) => {
                warn!("skipping malformed pitcher row: {}", e);
            }
        }
    }
    Ok(pitchers)
}

fn load_adp_from_reader<R: Read>(rdr: R) -> Result<HashMap<String, f64>, csv::Error> {
    let mut reader = csv::Reader::from_reader(rdr);
    let mut map = HashMap::new();
    for result in reader.deserialize::<RawAdp>() {
        match result {
            Ok(raw) => {
                if !raw.ADP.is_finite() {
                    warn!("skipping ADP entry for '{}': non-finite value", raw.Name.trim());
                    continue;
                }
                let name = raw.Name.trim().to_string();
                if map.contains_key(&name) {
                    warn!("duplicate ADP entry for '{}', using latest value", name);
                }
                map.insert(name, raw.ADP);
            }
            Err(e) => {
                warn!("skipping malformed ADP row: {}", e);
            }
        }
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Public path-based loaders
// ---------------------------------------------------------------------------

/// Load hitter projections from a CSV file.
pub fn load_hitter_projections(path: &Path) -> Result<Vec<HitterProjection>, ProjectionError> {
    let file = std::fs::File::open(path).map_err(|e| ProjectionError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    load_hitters_from_reader(file).map_err(|e| ProjectionError::Csv {
        path: path.display().to_string(),
        source: e,
    })
}

/// Load pitcher projections from a combined CSV file (SP+RP with POS column).
pub fn load_pitcher_projections(path: &Path) -> Result<Vec<PitcherProjection>, ProjectionError> {
    let file = std::fs::File::open(path).map_err(|e| ProjectionError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    load_pitchers_from_reader(file).map_err(|e| ProjectionError::Csv {
        path: path.display().to_string(),
        source: e,
    })
}

/// Load ADP data from a CSV file. Returns a map of player name → ADP value.
pub fn load_adp(path: &Path) -> Result<HashMap<String, f64>, ProjectionError> {
    let file = std::fs::File::open(path).map_err(|e| ProjectionError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    load_adp_from_reader(file).map_err(|e| ProjectionError::Csv {
        path: path.display().to_string(),
        source: e,
    })
}

/// Load all projection data using paths from the config and return
/// the combined `AllProjections`.
pub fn load_all(config: &Config) -> Result<AllProjections, ProjectionError> {
    load_all_from_paths(&config.data_paths)
}

/// Load all projection data from explicit paths. Exposed for testing and flexibility.
pub fn load_all_from_paths(paths: &DataPaths) -> Result<AllProjections, ProjectionError> {
    let hitters = load_hitter_projections(Path::new(&paths.hitters))?;
    let pitchers = load_pitcher_projections(Path::new(&paths.pitchers))?;
    let adp = load_adp(Path::new(&paths.adp))?;

    if hitters.is_empty() {
        return Err(ProjectionError::Validation(
            "hitter CSV produced zero valid rows".into(),
        ));
    }
    if pitchers.is_empty() {
        return Err(ProjectionError::Validation(
            "pitcher CSV produced zero valid rows".into(),
        ));
    }

    Ok(AllProjections {
        hitters,
        pitchers,
        adp,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Hitter CSV round-trip --

    #[test]
    fn hitter_csv_roundtrip() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Aaron Judge,NYY,700,600,180,50,120,130,90,5,0.300
Mookie Betts,LAD,680,590,170,30,110,95,80,15,0.288";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters.len(), 2);

        assert_eq!(hitters[0].name, "Aaron Judge");
        assert_eq!(hitters[0].team, "NYY");
        assert_eq!(hitters[0].hr, 50);
        assert_eq!(hitters[0].pa, 700);
        assert_eq!(hitters[0].ab, 600);
        assert_eq!(hitters[0].h, 180);
        assert_eq!(hitters[0].r, 120);
        assert_eq!(hitters[0].rbi, 130);
        assert_eq!(hitters[0].bb, 90);
        assert_eq!(hitters[0].sb, 5);
        assert!((hitters[0].avg - 0.300).abs() < f64::EPSILON);

        assert_eq!(hitters[1].name, "Mookie Betts");
        assert_eq!(hitters[1].team, "LAD");
        assert_eq!(hitters[1].hr, 30);
    }

    // -- Hitter CSV with fractional stats (Razzball format) --

    #[test]
    fn hitter_csv_fractional_stats_rounded() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Aaron Judge,NYY,699.6,600.4,180.3,50.7,120.1,130.9,89.5,5.2,0.300";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters[0].pa, 700);
        assert_eq!(hitters[0].ab, 600);
        assert_eq!(hitters[0].h, 180);
        assert_eq!(hitters[0].hr, 51);
        assert_eq!(hitters[0].r, 120);
        assert_eq!(hitters[0].rbi, 131);
        assert_eq!(hitters[0].bb, 90);
        assert_eq!(hitters[0].sb, 5);
    }

    // -- Hitter CSV with extra Razzball columns ignored --

    #[test]
    fn hitter_csv_extra_columns_ignored() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG,OBP,SLG,OPS
Aaron Judge,NYY,700,600,180,50,120,130,90,5,0.300,0.420,0.650,1.070";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters.len(), 1);
        assert_eq!(hitters[0].name, "Aaron Judge");
        assert_eq!(hitters[0].hr, 50);
    }

    // -- Combined pitcher CSV with POS column --

    #[test]
    fn combined_pitcher_csv_splits_by_pos() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K
Gerrit Cole,NYY,SP,32,32,200.0,16,0,0,2.80,1.05,250
Devin Williams,NYY,RP,60,0,62.0,3,5,25,2.10,0.92,90";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers.len(), 2);

        assert_eq!(pitchers[0].name, "Gerrit Cole");
        assert_eq!(pitchers[0].pitcher_type, PitcherType::SP);
        assert_eq!(pitchers[0].k, 250);
        assert_eq!(pitchers[0].w, 16);
        assert_eq!(pitchers[0].sv, 0);
        assert_eq!(pitchers[0].gs, 32);
        assert!((pitchers[0].ip - 200.0).abs() < f64::EPSILON);
        assert!((pitchers[0].era - 2.80).abs() < f64::EPSILON);
        assert!((pitchers[0].whip - 1.05).abs() < f64::EPSILON);

        assert_eq!(pitchers[1].name, "Devin Williams");
        assert_eq!(pitchers[1].pitcher_type, PitcherType::RP);
        assert_eq!(pitchers[1].k, 90);
        assert_eq!(pitchers[1].hd, 25);
        assert_eq!(pitchers[1].sv, 5);
    }

    // -- HLD column parsed correctly for RP --

    #[test]
    fn hld_column_parsed_for_rp() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K
Clay Holmes,CLE,RP,58,0,60.0,3,2,18,3.20,1.15,65";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers[0].hd, 18);
        assert_eq!(pitchers[0].pitcher_type, PitcherType::RP);
    }

    // -- HD alias for HLD column --

    #[test]
    fn hd_alias_for_hld() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HD,ERA,WHIP,K
Clay Holmes,CLE,RP,58,0,60.0,3,2,18,3.20,1.15,65";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers[0].hd, 18);
    }

    // -- SP gets zero holds --

    #[test]
    fn sp_holds_are_zero() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K
Gerrit Cole,NYY,SP,32,32,200.0,16,0,0,2.80,1.05,250";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers[0].hd, 0);
        assert_eq!(pitchers[0].pitcher_type, PitcherType::SP);
    }

    // -- Extra pitcher columns ignored --

    #[test]
    fn pitcher_csv_extra_columns_ignored() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K,BB,HR,QS
Gerrit Cole,NYY,SP,32,32,200.0,16,0,0,2.80,1.05,250,40,20,22";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers.len(), 1);
        assert_eq!(pitchers[0].name, "Gerrit Cole");
    }

    // -- Fractional pitcher stats rounded --

    #[test]
    fn pitcher_csv_fractional_stats_rounded() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K
Test Pitcher,NYY,RP,60.4,0.0,62.3,3.7,5.2,24.6,2.10,0.92,89.5";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers[0].g, 60);
        assert_eq!(pitchers[0].gs, 0);
        assert_eq!(pitchers[0].w, 4);
        assert_eq!(pitchers[0].sv, 5);
        assert_eq!(pitchers[0].hd, 25);
        assert_eq!(pitchers[0].k, 90);
    }

    // -- Unknown POS skipped --

    #[test]
    fn unknown_pos_skipped() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K
Valid SP,NYY,SP,32,32,200.0,16,0,0,2.80,1.05,250
Unknown,NYY,CL,60,0,62.0,3,5,25,2.10,0.92,90
Valid RP,NYY,RP,60,0,62.0,3,5,25,2.10,0.92,90";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers.len(), 2);
        assert_eq!(pitchers[0].name, "Valid SP");
        assert_eq!(pitchers[1].name, "Valid RP");
    }

    // -- Column alias: SO for K --

    #[test]
    fn pitcher_csv_so_alias() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,SO
Gerrit Cole,NYY,SP,32,32,200.0,16,0,0,2.80,1.05,250";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers[0].k, 250);
    }

    // -- Column alias: BA for AVG --

    #[test]
    fn hitter_csv_ba_alias() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,BA
Aaron Judge,NYY,700,600,180,50,120,130,90,5,0.300";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters.len(), 1);
        assert!((hitters[0].avg - 0.300).abs() < f64::EPSILON);
    }

    // -- Malformed rows skipped --

    #[test]
    fn malformed_hitter_rows_skipped() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Valid Player,NYY,600,500,150,30,90,80,70,10,0.300
Bad Row,NYY,not_a_number,500,150,30,90,80,70,10,0.300
Another Valid,BOS,550,480,140,25,80,75,60,5,0.292";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters.len(), 2);
        assert_eq!(hitters[0].name, "Valid Player");
        assert_eq!(hitters[1].name, "Another Valid");
    }

    // -- Empty CSV --

    #[test]
    fn empty_csv_returns_empty_vec() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert!(hitters.is_empty());
    }

    // -- ADP loading --

    #[test]
    fn adp_loading() {
        let csv_data = "\
Name,ADP
Aaron Judge,3.5
Mookie Betts,7.2
Shohei Ohtani,1.1";

        let adp = load_adp_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(adp.len(), 3);
        assert!((adp["Aaron Judge"] - 3.5).abs() < f64::EPSILON);
        assert!((adp["Mookie Betts"] - 7.2).abs() < f64::EPSILON);
        assert!((adp["Shohei Ohtani"] - 1.1).abs() < f64::EPSILON);
    }

    // -- Duplicate detection in ADP --

    #[test]
    fn adp_duplicate_uses_latest() {
        let csv_data = "\
Name,ADP
Aaron Judge,3.5
Aaron Judge,5.0";

        let adp = load_adp_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(adp.len(), 1);
        assert!((adp["Aaron Judge"] - 5.0).abs() < f64::EPSILON);
    }

    // -- Name trimming --

    #[test]
    fn hitter_names_trimmed() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
  Aaron Judge  , NYY ,700,600,180,50,120,130,90,5,0.300";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters[0].name, "Aaron Judge");
        assert_eq!(hitters[0].team, "NYY");
    }

    #[test]
    fn pitcher_names_trimmed() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K
  Gerrit Cole  , NYY ,SP,32,32,200.0,16,0,0,2.80,1.05,250";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers[0].name, "Gerrit Cole");
        assert_eq!(pitchers[0].team, "NYY");
    }

    #[test]
    fn adp_names_trimmed() {
        let csv_data = "\
Name,ADP
  Aaron Judge  ,3.5";

        let adp = load_adp_from_reader(csv_data.as_bytes()).unwrap();
        assert!((adp["Aaron Judge"] - 3.5).abs() < f64::EPSILON);
    }

    // -- Non-finite f64 rejection --

    #[test]
    fn hitter_nan_avg_skipped() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Valid Player,NYY,600,500,150,30,90,80,70,10,0.300
NaN Player,NYY,600,500,150,30,90,80,70,10,NaN";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters.len(), 1);
        assert_eq!(hitters[0].name, "Valid Player");
    }

    #[test]
    fn pitcher_inf_era_skipped() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K
Valid Pitcher,NYY,SP,32,32,200.0,16,0,0,2.80,1.05,250
Inf Pitcher,NYY,SP,32,32,200.0,16,0,0,inf,1.05,250";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers.len(), 1);
        assert_eq!(pitchers[0].name, "Valid Pitcher");
    }

    #[test]
    fn adp_nan_skipped() {
        let csv_data = "\
Name,ADP
Aaron Judge,3.5
Bad Player,NaN";

        let adp = load_adp_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(adp.len(), 1);
        assert!(adp.contains_key("Aaron Judge"));
    }
}
