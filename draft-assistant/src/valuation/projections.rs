// Projection data loading and normalization.

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
// Raw CSV serde structs (private)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code, non_snake_case)]
struct RawHitter {
    Name: String,
    #[serde(default)]
    Team: String,
    PA: u32,
    AB: u32,
    H: u32,
    HR: u32,
    R: u32,
    RBI: u32,
    BB: u32,
    SB: u32,
    #[serde(alias = "BA")]
    AVG: f64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code, non_snake_case)]
struct RawPitcher {
    Name: String,
    #[serde(default)]
    Team: String,
    IP: f64,
    #[serde(alias = "SO")]
    K: u32,
    W: u32,
    SV: u32,
    #[serde(default)]
    HD: u32,
    ERA: f64,
    WHIP: f64,
    G: u32,
    #[serde(default)]
    GS: u32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code, non_snake_case)]
struct RawHoldsOverlay {
    Name: String,
    #[serde(default)]
    Team: String,
    HD: u32,
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

/// Merge holds overlay data into pitcher projections.
///
/// For RPs, holds are resolved in priority order:
/// 1. Holds overlay value (if player has an entry in `holds_map`)
/// 2. Value already present from the RP CSV (if `hd > 0`)
/// 3. Estimated as `max(0, (G - SV - GS) * hold_rate)`
///
/// SPs are never assigned holds.
fn merge_holds(pitchers: &mut [PitcherProjection], holds_map: &HashMap<String, u32>, hold_rate: f64) {
    for p in pitchers.iter_mut() {
        if p.pitcher_type != PitcherType::RP {
            continue;
        }
        if let Some(&hd) = holds_map.get(&p.name) {
            p.hd = hd;
        } else if p.hd == 0 {
            // No overlay match and no value from CSV — estimate.
            let available = (p.g as f64) - (p.sv as f64) - (p.gs as f64);
            p.hd = (available.max(0.0) * hold_rate).round() as u32;
        }
        // else: preserve the non-zero HD value already loaded from the RP CSV.
    }
}

// ---------------------------------------------------------------------------
// Reader-based loaders (private, enable testing without temp files)
// ---------------------------------------------------------------------------

fn load_hitters_from_reader<R: Read>(rdr: R) -> Result<Vec<HitterProjection>, csv::Error> {
    let mut reader = csv::Reader::from_reader(rdr);
    let mut hitters = Vec::new();
    for result in reader.deserialize::<RawHitter>() {
        match result {
            Ok(raw) => {
                if !all_finite(&[raw.AVG]) {
                    warn!("skipping hitter '{}': non-finite AVG value", raw.Name.trim());
                    continue;
                }
                hitters.push(HitterProjection {
                    name: raw.Name.trim().to_string(),
                    team: raw.Team.trim().to_string(),
                    pa: raw.PA,
                    ab: raw.AB,
                    h: raw.H,
                    hr: raw.HR,
                    r: raw.R,
                    rbi: raw.RBI,
                    bb: raw.BB,
                    sb: raw.SB,
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

fn load_pitchers_from_reader<R: Read>(
    rdr: R,
    pitcher_type: PitcherType,
) -> Result<Vec<PitcherProjection>, csv::Error> {
    let mut reader = csv::Reader::from_reader(rdr);
    let mut pitchers = Vec::new();
    for result in reader.deserialize::<RawPitcher>() {
        match result {
            Ok(raw) => {
                if !all_finite(&[raw.IP, raw.ERA, raw.WHIP]) {
                    warn!("skipping pitcher '{}': non-finite IP/ERA/WHIP value", raw.Name.trim());
                    continue;
                }
                pitchers.push(PitcherProjection {
                    name: raw.Name.trim().to_string(),
                    team: raw.Team.trim().to_string(),
                    pitcher_type,
                    ip: raw.IP,
                    k: raw.K,
                    w: raw.W,
                    sv: raw.SV,
                    hd: raw.HD,
                    era: raw.ERA,
                    whip: raw.WHIP,
                    g: raw.G,
                    gs: raw.GS,
                });
            }
            Err(e) => {
                warn!("skipping malformed pitcher row: {}", e);
            }
        }
    }
    Ok(pitchers)
}

fn load_holds_from_reader<R: Read>(rdr: R) -> Result<HashMap<String, u32>, csv::Error> {
    let mut reader = csv::Reader::from_reader(rdr);
    let mut map = HashMap::new();
    for result in reader.deserialize::<RawHoldsOverlay>() {
        match result {
            Ok(raw) => {
                let name = raw.Name.trim().to_string();
                if map.contains_key(&name) {
                    warn!("duplicate holds entry for '{}', using latest value", name);
                }
                map.insert(name, raw.HD);
            }
            Err(e) => {
                warn!("skipping malformed holds row: {}", e);
            }
        }
    }
    Ok(map)
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

/// Load starting pitcher projections from a CSV file.
pub fn load_sp_projections(path: &Path) -> Result<Vec<PitcherProjection>, ProjectionError> {
    let file = std::fs::File::open(path).map_err(|e| ProjectionError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    load_pitchers_from_reader(file, PitcherType::SP).map_err(|e| ProjectionError::Csv {
        path: path.display().to_string(),
        source: e,
    })
}

/// Load relief pitcher projections from a CSV file.
pub fn load_rp_projections(path: &Path) -> Result<Vec<PitcherProjection>, ProjectionError> {
    let file = std::fs::File::open(path).map_err(|e| ProjectionError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    load_pitchers_from_reader(file, PitcherType::RP).map_err(|e| ProjectionError::Csv {
        path: path.display().to_string(),
        source: e,
    })
}

/// Load holds overlay from a CSV file. Returns a map of player name → projected holds.
pub fn load_holds_overlay(path: &Path) -> Result<HashMap<String, u32>, ProjectionError> {
    let file = std::fs::File::open(path).map_err(|e| ProjectionError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    load_holds_from_reader(file).map_err(|e| ProjectionError::Csv {
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

/// Load all projection data using paths from the config, merge holds, and return
/// the combined `AllProjections`.
pub fn load_all(config: &Config) -> Result<AllProjections, ProjectionError> {
    load_all_from_paths(&config.data_paths, config.strategy.holds_estimation.default_hold_rate)
}

/// Load all projection data from explicit paths. Exposed for testing and flexibility.
pub fn load_all_from_paths(
    paths: &DataPaths,
    hold_rate: f64,
) -> Result<AllProjections, ProjectionError> {
    if !(0.0..=1.0).contains(&hold_rate) {
        return Err(ProjectionError::Validation(format!(
            "hold_rate must be between 0.0 and 1.0 inclusive, got {hold_rate}"
        )));
    }

    let hitters = load_hitter_projections(Path::new(&paths.hitters))?;
    let sp = load_sp_projections(Path::new(&paths.pitchers_sp))?;
    let rp = load_rp_projections(Path::new(&paths.pitchers_rp))?;

    // Holds overlay is optional — if the file is missing, fall through to
    // estimation in merge_holds.
    let holds_path = Path::new(&paths.holds_overlay);
    let holds_map = match load_holds_overlay(holds_path) {
        Ok(map) => map,
        Err(ProjectionError::Io { ref source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            warn!("holds overlay file not found at {:?}, using estimation for all RPs", holds_path);
            HashMap::new()
        }
        Err(e) => return Err(e),
    };

    let adp = load_adp(Path::new(&paths.adp))?;

    // Combine SP and RP, warning about duplicates.
    let mut seen_names: HashMap<String, PitcherType> = HashMap::with_capacity(sp.len() + rp.len());
    let mut pitchers: Vec<PitcherProjection> = Vec::with_capacity(sp.len() + rp.len());
    for pitcher in sp.into_iter().chain(rp.into_iter()) {
        if let Some(&prev_type) = seen_names.get(&pitcher.name) {
            warn!(
                "duplicate pitcher '{}' found in both {:?} and {:?} files, keeping both entries",
                pitcher.name, prev_type, pitcher.pitcher_type
            );
        }
        seen_names.insert(pitcher.name.clone(), pitcher.pitcher_type);
        pitchers.push(pitcher);
    }

    merge_holds(&mut pitchers, &holds_map, hold_rate);

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

    // -- Pitcher CSV loading --

    #[test]
    fn pitcher_csv_sp() {
        let csv_data = "\
Name,Team,IP,K,W,SV,ERA,WHIP,G,GS
Gerrit Cole,NYY,200.0,250,16,0,2.80,1.05,32,32";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes(), PitcherType::SP).unwrap();
        assert_eq!(pitchers.len(), 1);
        assert_eq!(pitchers[0].name, "Gerrit Cole");
        assert_eq!(pitchers[0].pitcher_type, PitcherType::SP);
        assert_eq!(pitchers[0].k, 250);
        assert_eq!(pitchers[0].w, 16);
        assert_eq!(pitchers[0].sv, 0);
        assert_eq!(pitchers[0].gs, 32);
        assert!((pitchers[0].ip - 200.0).abs() < f64::EPSILON);
        assert!((pitchers[0].era - 2.80).abs() < f64::EPSILON);
        assert!((pitchers[0].whip - 1.05).abs() < f64::EPSILON);
    }

    // -- Column alias: SO for K --

    #[test]
    fn pitcher_csv_so_alias() {
        let csv_data = "\
Name,Team,IP,SO,W,SV,ERA,WHIP,G,GS
Gerrit Cole,NYY,200.0,250,16,0,2.80,1.05,32,32";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes(), PitcherType::SP).unwrap();
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

    // -- Holds merge --

    #[test]
    fn holds_merge_matched() {
        let mut pitchers = vec![PitcherProjection {
            name: "Devin Williams".into(),
            team: "NYY".into(),
            pitcher_type: PitcherType::RP,
            ip: 65.0,
            k: 80,
            w: 3,
            sv: 5,
            hd: 0,
            era: 2.50,
            whip: 1.00,
            g: 60,
            gs: 0,
        }];
        let mut holds_map = HashMap::new();
        holds_map.insert("Devin Williams".to_string(), 25u32);

        merge_holds(&mut pitchers, &holds_map, 0.25);
        assert_eq!(pitchers[0].hd, 25);
    }

    #[test]
    fn holds_merge_estimated() {
        let mut pitchers = vec![PitcherProjection {
            name: "Some Reliever".into(),
            team: "BOS".into(),
            pitcher_type: PitcherType::RP,
            ip: 60.0,
            k: 60,
            w: 4,
            sv: 10,
            hd: 0,
            era: 3.50,
            whip: 1.20,
            g: 60,
            gs: 0,
        }];
        let holds_map = HashMap::new();

        // estimated = max(0, (60 - 10 - 0) * 0.25) = 12.5 → rounds to 13
        merge_holds(&mut pitchers, &holds_map, 0.25);
        assert_eq!(pitchers[0].hd, 13);
    }

    #[test]
    fn holds_merge_saturating_edge_case() {
        // G < SV + GS → available is negative, should clamp to 0
        let mut pitchers = vec![PitcherProjection {
            name: "Closer".into(),
            team: "LAD".into(),
            pitcher_type: PitcherType::RP,
            ip: 50.0,
            k: 55,
            w: 2,
            sv: 40,
            hd: 0,
            era: 2.00,
            whip: 0.90,
            g: 60,
            gs: 25,
        }];
        let holds_map = HashMap::new();

        // available = 60 - 40 - 25 = -5 → max(0, -5) = 0 → hd = 0
        merge_holds(&mut pitchers, &holds_map, 0.25);
        assert_eq!(pitchers[0].hd, 0);
    }

    #[test]
    fn holds_merge_sp_skipped() {
        let mut pitchers = vec![PitcherProjection {
            name: "SP Guy".into(),
            team: "CHC".into(),
            pitcher_type: PitcherType::SP,
            ip: 180.0,
            k: 200,
            w: 14,
            sv: 0,
            hd: 0,
            era: 3.20,
            whip: 1.10,
            g: 30,
            gs: 30,
        }];
        let mut holds_map = HashMap::new();
        holds_map.insert("SP Guy".to_string(), 10u32);

        merge_holds(&mut pitchers, &holds_map, 0.25);
        // SP should not get holds even if in the overlay
        assert_eq!(pitchers[0].hd, 0);
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

    // -- Holds overlay CSV loading --

    #[test]
    fn holds_overlay_loading() {
        let csv_data = "\
Name,Team,HD
Devin Williams,NYY,25
Clay Holmes,CLE,18";

        let holds = load_holds_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(holds.len(), 2);
        assert_eq!(holds["Devin Williams"], 25);
        assert_eq!(holds["Clay Holmes"], 18);
    }

    // -- Holds merge: preserves non-zero HD from CSV --

    #[test]
    fn holds_merge_preserves_csv_hd() {
        let mut pitchers = vec![PitcherProjection {
            name: "Setup Man".into(),
            team: "SEA".into(),
            pitcher_type: PitcherType::RP,
            ip: 60.0,
            k: 65,
            w: 3,
            sv: 2,
            hd: 20, // non-zero from RP CSV
            era: 3.00,
            whip: 1.10,
            g: 60,
            gs: 0,
        }];
        let holds_map = HashMap::new(); // no overlay entry

        merge_holds(&mut pitchers, &holds_map, 0.25);
        // Should preserve the 20 from the CSV, not estimate
        assert_eq!(pitchers[0].hd, 20);
    }

    // -- Holds merge: overlay overrides even non-zero CSV HD --

    #[test]
    fn holds_merge_overlay_overrides_csv_hd() {
        let mut pitchers = vec![PitcherProjection {
            name: "Setup Man".into(),
            team: "SEA".into(),
            pitcher_type: PitcherType::RP,
            ip: 60.0,
            k: 65,
            w: 3,
            sv: 2,
            hd: 20, // from CSV
            era: 3.00,
            whip: 1.10,
            g: 60,
            gs: 0,
        }];
        let mut holds_map = HashMap::new();
        holds_map.insert("Setup Man".to_string(), 30u32);

        merge_holds(&mut pitchers, &holds_map, 0.25);
        // Overlay takes priority over CSV value
        assert_eq!(pitchers[0].hd, 30);
    }

    // -- Duplicate detection in holds overlay --

    #[test]
    fn holds_overlay_duplicate_uses_latest() {
        let csv_data = "\
Name,Team,HD
Devin Williams,NYY,25
Devin Williams,NYY,30";

        let holds = load_holds_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(holds.len(), 1);
        assert_eq!(holds["Devin Williams"], 30); // last-write-wins
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
Name,Team,IP,K,W,SV,ERA,WHIP,G,GS
  Gerrit Cole  , NYY ,200.0,250,16,0,2.80,1.05,32,32";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes(), PitcherType::SP).unwrap();
        assert_eq!(pitchers[0].name, "Gerrit Cole");
        assert_eq!(pitchers[0].team, "NYY");
    }

    #[test]
    fn holds_overlay_names_trimmed() {
        let csv_data = "\
Name,Team,HD
  Devin Williams  ,NYY,25";

        let holds = load_holds_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(holds["Devin Williams"], 25);
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
Name,Team,IP,K,W,SV,ERA,WHIP,G,GS
Valid Pitcher,NYY,200.0,250,16,0,2.80,1.05,32,32
Inf Pitcher,NYY,200.0,250,16,0,inf,1.05,32,32";

        let pitchers = load_pitchers_from_reader(csv_data.as_bytes(), PitcherType::SP).unwrap();
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
