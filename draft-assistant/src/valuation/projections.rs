// Projection data loading and normalization.
//
// Reads Razzball-format CSV files: a single combined pitchers CSV with a POS
// column (SP/RP) and an HLD column containing real holds data.

use crate::config::{Config, DataPaths};
use crate::draft::pick::Position;
use serde::{Deserialize, Serialize};
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
/// Position eligibility comes from the POS column in the projection CSV.
/// Multi-position players use slash-separated positions (e.g. "2B/SS").
/// During a live draft, ESPN eligible_slots data can augment/override these.
#[derive(Debug, Clone)]
pub struct HitterProjection {
    pub name: String,
    pub team: String,
    pub positions: Vec<Position>,
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
/// via `csv::ReaderBuilder::flexible(true)`.
///
/// The POS column is optional for backward compatibility. When present, it
/// contains slash-separated position strings (e.g. "2B/SS", "OF", "C").
#[derive(Debug, Deserialize)]
#[allow(dead_code, non_snake_case)]
struct RawRazzballHitter {
    Name: String,
    #[serde(default)]
    Team: String,
    #[serde(default)]
    POS: String,
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
    #[serde(default, alias = "HD")]
    HLD: f64,
    ERA: f64,
    WHIP: f64,
    #[serde(alias = "SO")]
    K: f64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a POS string like "2B/SS", "OF", "C" into a Vec<Position>.
///
/// - Splits on '/' to support multi-position strings.
/// - "OF" is expanded to CenterField (generic outfield).
/// - Unrecognized tokens are silently skipped.
/// - Returns an empty Vec if the input is empty.
fn parse_hitter_positions(pos_str: &str) -> Vec<Position> {
    if pos_str.trim().is_empty() {
        return Vec::new();
    }
    let mut positions: Vec<Position> = pos_str
        .split('/')
        .filter_map(|s| Position::from_str_pos(s.trim()))
        .filter(|p| !p.is_meta_slot())
        .collect();
    positions.dedup();
    positions
}

/// Returns true if all given f64 values are finite and non-negative.
fn all_valid_counts(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite() && *v >= 0.0)
}

/// Returns true if all given f64 values are finite (not NaN or Infinity).
fn all_finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

// ---------------------------------------------------------------------------
// Reader-based loaders (private, enable testing without temp files)
// ---------------------------------------------------------------------------

/// Load hitter projections from a reader. Public for testing.
pub fn load_hitter_projections_from_reader<R: Read>(rdr: R) -> Result<Vec<HitterProjection>, csv::Error> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(rdr);
    let mut hitters = Vec::new();
    for result in reader.deserialize::<RawRazzballHitter>() {
        match result {
            Ok(raw) => {
                if !all_valid_counts(&[raw.PA, raw.AB, raw.H, raw.HR, raw.R, raw.RBI, raw.BB, raw.SB]) {
                    warn!("skipping hitter '{}': non-finite or negative counting stat", raw.Name.trim());
                    continue;
                }
                if !raw.AVG.is_finite() {
                    warn!("skipping hitter '{}': non-finite AVG value", raw.Name.trim());
                    continue;
                }
                let positions = parse_hitter_positions(&raw.POS);
                hitters.push(HitterProjection {
                    name: raw.Name.trim().to_string(),
                    team: raw.Team.trim().to_string(),
                    positions,
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
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(rdr);
    let mut pitchers = Vec::new();
    for result in reader.deserialize::<RawRazzballPitcher>() {
        match result {
            Ok(raw) => {
                if !all_valid_counts(&[raw.G, raw.GS, raw.IP, raw.K, raw.W, raw.SV, raw.HLD]) {
                    warn!("skipping pitcher '{}': non-finite or negative counting stat", raw.Name.trim());
                    continue;
                }
                if !all_finite(&[raw.ERA, raw.WHIP]) {
                    warn!("skipping pitcher '{}': non-finite ERA/WHIP value", raw.Name.trim());
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

// ---------------------------------------------------------------------------
// Public path-based loaders
// ---------------------------------------------------------------------------

/// Load hitter projections from a CSV file.
pub fn load_hitter_projections(path: &Path) -> Result<Vec<HitterProjection>, ProjectionError> {
    let file = std::fs::File::open(path).map_err(|e| ProjectionError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    load_hitter_projections_from_reader(file).map_err(|e| ProjectionError::Csv {
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

/// Load all projection data using paths from the config and return
/// the combined `AllProjections`.
pub fn load_all(config: &Config) -> Result<AllProjections, ProjectionError> {
    load_all_from_paths(&config.data_paths)
}

/// Load all projection data from explicit paths. Exposed for testing and flexibility.
pub fn load_all_from_paths(paths: &DataPaths) -> Result<AllProjections, ProjectionError> {
    let hitters = load_hitter_projections(Path::new(&paths.hitters))?;
    let pitchers = load_pitcher_projections(Path::new(&paths.pitchers))?;

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

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
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

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
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

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
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

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
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

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters.len(), 2);
        assert_eq!(hitters[0].name, "Valid Player");
        assert_eq!(hitters[1].name, "Another Valid");
    }

    // -- Empty CSV --

    #[test]
    fn empty_csv_returns_empty_vec() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG";

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
        assert!(hitters.is_empty());
    }

    // -- Name trimming --

    #[test]
    fn hitter_names_trimmed() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
  Aaron Judge  , NYY ,700,600,180,50,120,130,90,5,0.300";

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
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

    // -- Non-finite f64 rejection --

    #[test]
    fn hitter_nan_avg_skipped() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Valid Player,NYY,600,500,150,30,90,80,70,10,0.300
NaN Player,NYY,600,500,150,30,90,80,70,10,NaN";

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
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
    fn hitter_negative_stat_skipped() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Valid Player,NYY,600,500,150,30,90,80,70,10,0.300
Negative HR,NYY,600,500,150,-5,90,80,70,10,0.300";

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters.len(), 1);
        assert_eq!(hitters[0].name, "Valid Player");
    }

    #[test]
    fn pitcher_negative_stat_skipped() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,HLD,ERA,WHIP,K
Valid Pitcher,NYY,SP,32,32,200.0,16,0,0,2.80,1.05,250
Negative K,NYY,SP,32,32,200.0,16,0,0,2.80,1.05,-10";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers.len(), 1);
        assert_eq!(pitchers[0].name, "Valid Pitcher");
    }

    #[test]
    fn pitcher_csv_without_hld_column_defaults_to_zero() {
        let csv_data = "\
Name,Team,POS,G,GS,IP,W,SV,ERA,WHIP,K
Gerrit Cole,NYY,SP,32,32,200.0,16,0,2.80,1.05,250";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(pitchers.len(), 1);
        assert_eq!(pitchers[0].hd, 0);
    }

    // -- POS column parsing for hitters --

    #[test]
    fn hitter_csv_with_pos_column() {
        let csv_data = "\
Name,Team,POS,PA,AB,H,HR,R,RBI,BB,SB,AVG
Aaron Judge,NYY,RF,700,600,180,50,120,130,90,5,0.300
Mookie Betts,LAD,2B/SS,680,590,170,30,110,95,80,15,0.288";

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters.len(), 2);
        assert_eq!(hitters[0].positions, vec![Position::RightField]);
        assert_eq!(
            hitters[1].positions,
            vec![Position::SecondBase, Position::ShortStop]
        );
    }

    #[test]
    fn hitter_csv_without_pos_column_positions_empty() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Aaron Judge,NYY,700,600,180,50,120,130,90,5,0.300";

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters.len(), 1);
        assert!(hitters[0].positions.is_empty());
    }

    #[test]
    fn hitter_csv_of_position_maps_to_center_field() {
        let csv_data = "\
Name,Team,POS,PA,AB,H,HR,R,RBI,BB,SB,AVG
Juan Soto,NYM,OF,700,580,165,35,115,110,110,3,0.284";

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters[0].positions, vec![Position::CenterField]);
    }

    #[test]
    fn hitter_csv_dh_position() {
        let csv_data = "\
Name,Team,POS,PA,AB,H,HR,R,RBI,BB,SB,AVG
Shohei Ohtani,LAD,DH,660,580,170,45,110,100,70,15,0.293";

        let hitters = load_hitter_projections_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters[0].positions, vec![Position::DesignatedHitter]);
    }

    #[test]
    fn parse_hitter_positions_empty_string() {
        assert!(parse_hitter_positions("").is_empty());
        assert!(parse_hitter_positions("   ").is_empty());
    }

    #[test]
    fn parse_hitter_positions_single() {
        assert_eq!(parse_hitter_positions("C"), vec![Position::Catcher]);
        assert_eq!(parse_hitter_positions("SS"), vec![Position::ShortStop]);
        assert_eq!(parse_hitter_positions("1B"), vec![Position::FirstBase]);
    }

    #[test]
    fn parse_hitter_positions_multi() {
        assert_eq!(
            parse_hitter_positions("2B/SS"),
            vec![Position::SecondBase, Position::ShortStop]
        );
        assert_eq!(
            parse_hitter_positions("SS/3B"),
            vec![Position::ShortStop, Position::ThirdBase]
        );
    }

    #[test]
    fn parse_hitter_positions_skips_meta_slots() {
        // UTIL, BE, IL should be filtered out
        assert_eq!(
            parse_hitter_positions("SS/UTIL"),
            vec![Position::ShortStop]
        );
        assert_eq!(
            parse_hitter_positions("BE"),
            Vec::<Position>::new()
        );
    }

    #[test]
    fn parse_hitter_positions_skips_unknown() {
        assert_eq!(
            parse_hitter_positions("SS/XX/3B"),
            vec![Position::ShortStop, Position::ThirdBase]
        );
    }

}
