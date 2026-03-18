// Projection data loading and normalization.
//
// Reads Razzball-format CSV files: a single combined pitchers CSV with a POS
// column (SP/RP) and an HLD column containing real holds data.

use crate::config::{Config, DataPaths};
use crate::protocol::EspnPlayerProjection;
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
/// The `espn_position` field is populated from the CSV's ESPN column at load
/// time and provides a fallback position. Live ESPN eligible_slots data from
/// the draft extension will override this at runtime when available.
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
    /// Raw ESPN position string from projections CSV (e.g. "SS", "DH", "OF").
    /// Empty if the CSV didn't include an ESPN column.
    pub espn_position: String,
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
#[derive(Debug, Deserialize)]
#[allow(dead_code, non_snake_case)]
struct RawRazzballHitter {
    Name: String,
    #[serde(default)]
    Team: String,
    #[serde(default)]
    ESPN: String,
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

fn load_hitters_from_reader<R: Read>(rdr: R) -> Result<Vec<HitterProjection>, csv::Error> {
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
                    espn_position: raw.ESPN.trim().to_string(),
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

/// Load all projection data using paths from the config and return
/// the combined `AllProjections`.
///
/// Returns `Ok(None)` if no CSV paths are configured (both are `None`).
/// Returns `Err` if only one path is set (must be both or neither)
/// or if the CSV files cannot be loaded.
pub fn load_all(config: &Config) -> Result<Option<AllProjections>, ProjectionError> {
    load_all_from_paths(&config.data_paths)
}

/// Resolve a data file path from the config.
///
/// If the path is absolute, use it as-is. If it is relative:
///
/// - **Debug builds** (`cargo build`/`cargo run`): resolve relative to CWD
///   (dev workflow, files live in the repo checkout).
/// - **Release builds** (`cargo build --release`): resolve relative to the
///   OS app data directory (`~/.local/share/wyncast` on Linux).
fn resolve_data_path(raw: &str) -> std::path::PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() {
        return p.to_path_buf();
    }

    #[cfg(debug_assertions)]
    {
        // Dev: resolve relative to CWD (run from repo root)
        if let Ok(cwd) = std::env::current_dir() {
            let candidate = cwd.join(p);
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // Release (and debug fallback): resolve relative to app data dir
    crate::app_dirs::app_data_dir().join(p)
}

/// Load all projection data from explicit paths. Exposed for testing and flexibility.
///
/// Returns `Ok(None)` if both paths are `None` (no CSV overrides configured).
/// Returns `Err` if only one path is set (must be both or neither),
/// or if the CSV files cannot be loaded.
pub fn load_all_from_paths(paths: &DataPaths) -> Result<Option<AllProjections>, ProjectionError> {
    match (&paths.hitters, &paths.pitchers) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(ProjectionError::Validation(
            "hitters CSV path is set but pitchers CSV path is missing".into(),
        )),
        (None, Some(_)) => Err(ProjectionError::Validation(
            "pitchers CSV path is set but hitters CSV path is missing".into(),
        )),
        (Some(h), Some(p)) => {
            let hitters_path = resolve_data_path(h);
            let pitchers_path = resolve_data_path(p);

            let hitters = load_hitter_projections(&hitters_path)?;
            let pitchers = load_pitcher_projections(&pitchers_path)?;

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

            Ok(Some(AllProjections { hitters, pitchers }))
        }
    }
}

// ---------------------------------------------------------------------------
// ESPN projection conversion
// ---------------------------------------------------------------------------

/// Map an ESPN `defaultPositionId` to a position string.
///
/// ESPN IDs: 1=SP, 2=C, 3=1B, 4=2B, 5=3B, 6=SS, 7=LF, 8=CF, 9=RF, 10=DH, 11=RP.
fn espn_position_name(id: u16) -> &'static str {
    match id {
        1 => "SP",
        2 => "C",
        3 => "1B",
        4 => "2B",
        5 => "3B",
        6 => "SS",
        7 => "LF",
        8 => "CF",
        9 => "RF",
        10 => "DH",
        11 => "RP",
        _ => "UTIL",
    }
}

/// Hitter position IDs: C(2), 1B(3), 2B(4), 3B(5), SS(6), LF(7), CF(8), RF(9), DH(10).
fn is_hitter_position(id: u16) -> bool {
    matches!(id, 2..=10)
}

/// Convert ESPN projection data into our internal `AllProjections` format.
///
/// - Players with batting stats AND a hitter `default_position_id` produce a `HitterProjection`.
/// - Players with pitching stats AND a pitcher `default_position_id` produce a `PitcherProjection`.
/// - Two-way players (both batting and pitching stats) create entries in both lists.
///   The valuation engine's `compute_initial_zscores` already handles two-way player
///   detection by name matching between hitter and pitcher lists.
/// - Players with no projection stats or invalid data are skipped.
pub fn from_espn_projections(espn: &[EspnPlayerProjection]) -> AllProjections {
    let mut hitters = Vec::new();
    let mut pitchers = Vec::new();

    for player in espn {
        // Skip players with empty names
        if player.name.trim().is_empty() {
            continue;
        }

        // Create hitter projection if batting stats are present
        if let Some(ref batting) = player.batting {
            // Validate stats: skip if any counting stat is unreasonable
            if batting.ab == 0 && batting.pa == 0 {
                // No plate appearances — skip hitter entry
            } else if !batting.avg.is_finite() || batting.avg < 0.0 {
                warn!(
                    "Skipping hitter projection for '{}': invalid AVG {}",
                    player.name, batting.avg
                );
            } else {
                let position = if is_hitter_position(player.default_position_id) {
                    espn_position_name(player.default_position_id)
                } else {
                    // Two-way player with pitcher default position — use DH
                    "DH"
                };
                hitters.push(HitterProjection {
                    name: player.name.trim().to_string(),
                    team: player.team.clone(),
                    pa: batting.pa,
                    ab: batting.ab,
                    h: batting.h,
                    hr: batting.hr,
                    r: batting.r,
                    rbi: batting.rbi,
                    bb: batting.bb,
                    sb: batting.sb,
                    avg: batting.avg,
                    espn_position: position.to_string(),
                });
            }
        }

        // Create pitcher projection if pitching stats are present
        if let Some(ref pitching) = player.pitching {
            if pitching.ip <= 0.0 && pitching.g == 0 {
                // No innings or games — skip pitcher entry
            } else if !pitching.era.is_finite() || !pitching.whip.is_finite() {
                warn!(
                    "Skipping pitcher projection for '{}': non-finite ERA/WHIP",
                    player.name
                );
            } else {
                let pitcher_type = if player.default_position_id == 11 {
                    PitcherType::RP
                } else {
                    // SP (1) or two-way player — default to SP
                    PitcherType::SP
                };
                pitchers.push(PitcherProjection {
                    name: player.name.trim().to_string(),
                    team: player.team.clone(),
                    pitcher_type,
                    ip: pitching.ip,
                    k: pitching.k,
                    w: pitching.w,
                    sv: pitching.sv,
                    hd: pitching.hd,
                    era: pitching.era,
                    whip: pitching.whip,
                    g: pitching.g,
                    gs: pitching.gs,
                });
            }
        }
    }

    AllProjections { hitters, pitchers }
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
    fn hitter_negative_stat_skipped() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Valid Player,NYY,600,500,150,30,90,80,70,10,0.300
Negative HR,NYY,600,500,150,-5,90,80,70,10,0.300";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
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

    // -- ESPN position column --

    #[test]
    fn hitter_csv_espn_position_captured() {
        let csv_data = "\
Name,Team,Bats,ESPN,PA,AB,H,HR,R,RBI,BB,SB,AVG
Bobby Witt Jr.,KC,R,SS,652,590,171,27,96,87,49,32,0.289";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters[0].espn_position, "SS");
    }

    #[test]
    fn hitter_csv_missing_espn_column_defaults_empty() {
        let csv_data = "\
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Aaron Judge,NYY,700,600,180,50,120,130,90,5,0.300";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters[0].espn_position, "");
    }

    #[test]
    fn hitter_csv_espn_position_trimmed() {
        let csv_data = "\
Name,Team,ESPN,PA,AB,H,HR,R,RBI,BB,SB,AVG
Bobby Witt Jr.,KC, SS ,652,590,171,27,96,87,49,32,0.289";

        let hitters = load_hitters_from_reader(csv_data.as_bytes()).unwrap();
        assert_eq!(hitters[0].espn_position, "SS");
    }

    // -- ESPN projection conversion tests --

    use crate::protocol::{EspnBattingProjection, EspnPitchingProjection, EspnPlayerProjection};

    fn make_espn_hitter(name: &str, pos_id: u16) -> EspnPlayerProjection {
        EspnPlayerProjection {
            espn_id: 1,
            name: name.to_string(),
            team: "NYY".to_string(),
            default_position_id: pos_id,
            eligible_slots: vec![],
            batting: Some(EspnBattingProjection {
                pa: 600,
                ab: 530,
                h: 150,
                hr: 30,
                r: 90,
                rbi: 85,
                bb: 60,
                sb: 10,
                avg: 0.283,
            }),
            pitching: None,
        }
    }

    fn make_espn_pitcher(name: &str, pos_id: u16) -> EspnPlayerProjection {
        EspnPlayerProjection {
            espn_id: 2,
            name: name.to_string(),
            team: "NYY".to_string(),
            default_position_id: pos_id,
            eligible_slots: vec![],
            batting: None,
            pitching: Some(EspnPitchingProjection {
                ip: 180.0,
                k: 200,
                w: 14,
                sv: 0,
                hd: 0,
                era: 3.20,
                whip: 1.10,
                g: 30,
                gs: 30,
            }),
        }
    }

    #[test]
    fn espn_pure_hitter_conversion() {
        let players = vec![make_espn_hitter("Aaron Judge", 9)]; // RF = 9
        let result = from_espn_projections(&players);

        assert_eq!(result.hitters.len(), 1);
        assert_eq!(result.pitchers.len(), 0);
        assert_eq!(result.hitters[0].name, "Aaron Judge");
        assert_eq!(result.hitters[0].espn_position, "RF");
        assert_eq!(result.hitters[0].hr, 30);
        assert_eq!(result.hitters[0].pa, 600);
        assert!((result.hitters[0].avg - 0.283).abs() < f64::EPSILON);
    }

    #[test]
    fn espn_pure_pitcher_sp_conversion() {
        let players = vec![make_espn_pitcher("Gerrit Cole", 1)]; // SP = 1
        let result = from_espn_projections(&players);

        assert_eq!(result.hitters.len(), 0);
        assert_eq!(result.pitchers.len(), 1);
        assert_eq!(result.pitchers[0].name, "Gerrit Cole");
        assert_eq!(result.pitchers[0].pitcher_type, PitcherType::SP);
        assert_eq!(result.pitchers[0].k, 200);
        assert!((result.pitchers[0].era - 3.20).abs() < f64::EPSILON);
    }

    #[test]
    fn espn_pure_pitcher_rp_conversion() {
        let mut rp = make_espn_pitcher("Devin Williams", 11); // RP = 11
        if let Some(ref mut p) = rp.pitching {
            p.ip = 60.0;
            p.sv = 5;
            p.hd = 20;
            p.gs = 0;
        }
        let result = from_espn_projections(&[rp]);

        assert_eq!(result.pitchers.len(), 1);
        assert_eq!(result.pitchers[0].pitcher_type, PitcherType::RP);
        assert_eq!(result.pitchers[0].hd, 20);
        assert_eq!(result.pitchers[0].sv, 5);
    }

    #[test]
    fn espn_two_way_player_creates_both() {
        let two_way = EspnPlayerProjection {
            espn_id: 100,
            name: "Shohei Ohtani".to_string(),
            team: "LAD".to_string(),
            default_position_id: 10, // DH
            eligible_slots: vec![],
            batting: Some(EspnBattingProjection {
                pa: 650,
                ab: 570,
                h: 170,
                hr: 44,
                r: 100,
                rbi: 95,
                bb: 70,
                sb: 20,
                avg: 0.298,
            }),
            pitching: Some(EspnPitchingProjection {
                ip: 160.0,
                k: 220,
                w: 15,
                sv: 0,
                hd: 0,
                era: 2.80,
                whip: 1.00,
                g: 28,
                gs: 28,
            }),
        };

        let result = from_espn_projections(&[two_way]);
        assert_eq!(result.hitters.len(), 1);
        assert_eq!(result.pitchers.len(), 1);
        assert_eq!(result.hitters[0].name, "Shohei Ohtani");
        assert_eq!(result.hitters[0].espn_position, "DH");
        assert_eq!(result.pitchers[0].name, "Shohei Ohtani");
        assert_eq!(result.pitchers[0].pitcher_type, PitcherType::SP);
    }

    #[test]
    fn espn_no_stats_skipped() {
        let no_stats = EspnPlayerProjection {
            espn_id: 999,
            name: "No Stats Player".to_string(),
            team: "FA".to_string(),
            default_position_id: 6, // SS
            eligible_slots: vec![],
            batting: None,
            pitching: None,
        };
        let result = from_espn_projections(&[no_stats]);
        assert_eq!(result.hitters.len(), 0);
        assert_eq!(result.pitchers.len(), 0);
    }

    #[test]
    fn espn_nan_avg_skipped() {
        let mut bad_hitter = make_espn_hitter("NaN Player", 6);
        if let Some(ref mut b) = bad_hitter.batting {
            b.avg = f64::NAN;
        }
        let result = from_espn_projections(&[bad_hitter]);
        assert_eq!(result.hitters.len(), 0);
    }

    #[test]
    fn espn_inf_era_skipped() {
        let mut bad_pitcher = make_espn_pitcher("Inf ERA", 1);
        if let Some(ref mut p) = bad_pitcher.pitching {
            p.era = f64::INFINITY;
        }
        let result = from_espn_projections(&[bad_pitcher]);
        assert_eq!(result.pitchers.len(), 0);
    }

    #[test]
    fn espn_empty_name_skipped() {
        let empty_name = EspnPlayerProjection {
            espn_id: 1,
            name: "  ".to_string(),
            team: "NYY".to_string(),
            default_position_id: 6,
            eligible_slots: vec![],
            batting: Some(EspnBattingProjection {
                pa: 600, ab: 530, h: 150, hr: 30, r: 90, rbi: 85, bb: 60, sb: 10, avg: 0.283,
            }),
            pitching: None,
        };
        let result = from_espn_projections(&[empty_name]);
        assert_eq!(result.hitters.len(), 0);
    }

    #[test]
    fn espn_zero_pa_hitter_skipped() {
        let mut zero_pa = make_espn_hitter("Zero PA", 6);
        if let Some(ref mut b) = zero_pa.batting {
            b.pa = 0;
            b.ab = 0;
        }
        let result = from_espn_projections(&[zero_pa]);
        assert_eq!(result.hitters.len(), 0);
    }

    #[test]
    fn espn_pitcher_with_hitter_default_pos_uses_sp() {
        // A two-way player with SP default but only pitching stats
        let pitcher_hitter_pos = EspnPlayerProjection {
            espn_id: 50,
            name: "Pitcher With DH Pos".to_string(),
            team: "LAA".to_string(),
            default_position_id: 10, // DH
            eligible_slots: vec![],
            batting: None,
            pitching: Some(EspnPitchingProjection {
                ip: 150.0, k: 180, w: 12, sv: 0, hd: 0, era: 3.50, whip: 1.20, g: 28, gs: 28,
            }),
        };
        let result = from_espn_projections(&[pitcher_hitter_pos]);
        // Only pitching stats → only pitcher entry, default to SP for non-RP
        assert_eq!(result.hitters.len(), 0);
        assert_eq!(result.pitchers.len(), 1);
        assert_eq!(result.pitchers[0].pitcher_type, PitcherType::SP);
    }

}
