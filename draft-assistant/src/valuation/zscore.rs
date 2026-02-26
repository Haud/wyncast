// Z-score calculation with volume-weighted rate stats.

use crate::config::{CategoryWeights, Config, PoolConfig};
use crate::draft::pick::Position;
use crate::valuation::projections::{AllProjections, HitterProjection, PitcherProjection, PitcherType};

// ---------------------------------------------------------------------------
// Pool statistics
// ---------------------------------------------------------------------------

/// Mean and standard deviation for a single statistical category across a player pool.
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    pub mean: f64,
    pub stdev: f64,
}

/// Threshold below which standard deviation is treated as zero.
const STDEV_EPSILON: f64 = 1e-9;

/// Compute mean and standard deviation for a slice of values.
///
/// Returns `PoolStats { mean: 0.0, stdev: 0.0 }` for an empty slice.
/// Uses the population standard deviation (N denominator), since the pool
/// represents the full relevant player universe rather than a sample.
pub fn compute_pool_stats(values: &[f64]) -> PoolStats {
    if values.is_empty() {
        return PoolStats {
            mean: 0.0,
            stdev: 0.0,
        };
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    PoolStats {
        mean,
        stdev: variance.sqrt(),
    }
}

/// Compute a z-score given a value and pool stats.
///
/// Returns 0.0 if the standard deviation is approximately zero (guarding
/// against division by zero).
pub fn compute_zscore(value: f64, stats: &PoolStats) -> f64 {
    if stats.stdev < STDEV_EPSILON {
        return 0.0;
    }
    (value - stats.mean) / stats.stdev
}

// ---------------------------------------------------------------------------
// Rate stat contribution functions (volume-weighted)
// ---------------------------------------------------------------------------

/// ERA contribution: `IP * (league_avg_ERA - player_ERA) / 9`
///
/// A pitcher with ERA below league average produces a positive contribution.
pub fn era_contribution(ip: f64, era: f64, league_avg_era: f64) -> f64 {
    ip * (league_avg_era - era) / 9.0
}

/// WHIP contribution: `IP * (league_avg_WHIP - player_WHIP)`
///
/// A pitcher with WHIP below league average produces a positive contribution.
pub fn whip_contribution(ip: f64, whip: f64, league_avg_whip: f64) -> f64 {
    ip * (league_avg_whip - whip)
}

/// AVG contribution: `AB * (player_AVG - league_avg_AVG)`
///
/// A hitter with AVG above league average produces a positive contribution.
pub fn avg_contribution(ab: u32, avg: f64, league_avg_avg: f64) -> f64 {
    (ab as f64) * (avg - league_avg_avg)
}

// ---------------------------------------------------------------------------
// Per-category z-score structs
// ---------------------------------------------------------------------------

/// Per-category z-scores for a hitter.
#[derive(Debug, Clone, Copy)]
pub struct HitterZScores {
    pub r: f64,
    pub hr: f64,
    pub rbi: f64,
    pub bb: f64,
    pub sb: f64,
    pub avg: f64,
    pub total: f64,
}

/// Per-category z-scores for a pitcher.
#[derive(Debug, Clone, Copy)]
pub struct PitcherZScores {
    pub k: f64,
    pub w: f64,
    pub sv: f64,
    pub hd: f64,
    pub era: f64,
    pub whip: f64,
    pub total: f64,
}

/// Enum wrapper for hitter or pitcher z-scores.
#[derive(Debug, Clone, Copy)]
pub enum CategoryZScores {
    Hitter(HitterZScores),
    Pitcher(PitcherZScores),
}

// ---------------------------------------------------------------------------
// Pool stats structs
// ---------------------------------------------------------------------------

/// Aggregated pool statistics for all hitter categories.
#[derive(Debug, Clone)]
pub struct HitterPoolStats {
    pub r: PoolStats,
    pub hr: PoolStats,
    pub rbi: PoolStats,
    pub bb: PoolStats,
    pub sb: PoolStats,
    pub avg_contribution: PoolStats,
}

/// Aggregated pool statistics for all pitcher categories.
#[derive(Debug, Clone)]
pub struct PitcherPoolStats {
    pub k: PoolStats,
    pub w: PoolStats,
    pub sv: PoolStats,
    pub hd: PoolStats,
    pub era_contribution: PoolStats,
    pub whip_contribution: PoolStats,
}

// ---------------------------------------------------------------------------
// Projection data enum (carried through the pipeline)
// ---------------------------------------------------------------------------

/// Raw projection numbers carried forward through the valuation pipeline.
#[derive(Debug, Clone)]
pub enum PlayerProjectionData {
    Hitter {
        pa: u32,
        ab: u32,
        h: u32,
        hr: u32,
        r: u32,
        rbi: u32,
        bb: u32,
        sb: u32,
        avg: f64,
    },
    Pitcher {
        ip: f64,
        k: u32,
        w: u32,
        sv: u32,
        hd: u32,
        era: f64,
        whip: f64,
        g: u32,
        gs: u32,
    },
}

// ---------------------------------------------------------------------------
// Player valuation (main output struct)
// ---------------------------------------------------------------------------

/// A player carried through the full valuation pipeline.
///
/// Fields `vor`, `best_position`, and `dollar_value` are initialized
/// to defaults here and filled by subsequent pipeline stages (Tasks 06/07).
#[derive(Debug, Clone)]
pub struct PlayerValuation {
    pub name: String,
    pub team: String,
    pub positions: Vec<Position>,
    pub is_pitcher: bool,
    pub pitcher_type: Option<PitcherType>,
    pub projection: PlayerProjectionData,
    pub total_zscore: f64,
    pub category_zscores: CategoryZScores,
    pub vor: f64,
    pub best_position: Option<Position>,
    pub dollar_value: f64,
    pub adp: Option<f64>,
}

// ---------------------------------------------------------------------------
// Pool filtering
// ---------------------------------------------------------------------------

/// Filter hitters by minimum PA and return the top N by PA.
pub fn filter_hitter_pool<'a>(hitters: &'a [HitterProjection], pool: &PoolConfig) -> Vec<&'a HitterProjection> {
    let mut qualified: Vec<&HitterProjection> = hitters
        .iter()
        .filter(|h| h.pa >= pool.min_pa as u32)
        .collect();
    // Sort descending by PA to take top N
    qualified.sort_by(|a, b| b.pa.cmp(&a.pa));
    qualified.truncate(pool.hitter_pool_size);
    qualified
}

/// Filter starting pitchers by minimum IP and return the top N by IP.
pub fn filter_sp_pool<'a>(pitchers: &'a [PitcherProjection], pool: &PoolConfig) -> Vec<&'a PitcherProjection> {
    let mut qualified: Vec<&PitcherProjection> = pitchers
        .iter()
        .filter(|p| p.pitcher_type == PitcherType::SP && p.ip >= pool.min_ip_sp)
        .collect();
    qualified.sort_by(|a, b| b.ip.partial_cmp(&a.ip).unwrap_or(std::cmp::Ordering::Equal));
    qualified.truncate(pool.sp_pool_size);
    qualified
}

/// Filter relief pitchers by minimum games and return the top N by G.
pub fn filter_rp_pool<'a>(pitchers: &'a [PitcherProjection], pool: &PoolConfig) -> Vec<&'a PitcherProjection> {
    let mut qualified: Vec<&PitcherProjection> = pitchers
        .iter()
        .filter(|p| p.pitcher_type == PitcherType::RP && p.g >= pool.min_g_rp as u32)
        .collect();
    qualified.sort_by(|a, b| b.g.cmp(&a.g));
    qualified.truncate(pool.rp_pool_size);
    qualified
}

// ---------------------------------------------------------------------------
// Pool stats computation
// ---------------------------------------------------------------------------

/// Compute league-average rate stats from the hitter pool.
/// Returns (league_avg_avg,).
fn compute_league_avg_hitter(pool: &[&HitterProjection]) -> f64 {
    if pool.is_empty() {
        return 0.0;
    }
    let total_h: u32 = pool.iter().map(|h| h.h).sum();
    let total_ab: u32 = pool.iter().map(|h| h.ab).sum();
    if total_ab == 0 {
        return 0.0;
    }
    total_h as f64 / total_ab as f64
}

/// Compute league-average ERA and WHIP from a pitcher pool.
/// Returns (league_avg_era, league_avg_whip).
fn compute_league_avg_pitcher(pool: &[&PitcherProjection]) -> (f64, f64) {
    if pool.is_empty() {
        return (0.0, 0.0);
    }
    let total_ip: f64 = pool.iter().map(|p| p.ip).sum();
    if total_ip < STDEV_EPSILON {
        return (0.0, 0.0);
    }
    // ERA = (total earned runs) / (total IP) * 9
    // earned runs per pitcher = IP * ERA / 9
    let total_er: f64 = pool.iter().map(|p| p.ip * p.era / 9.0).sum();
    let league_era = total_er / total_ip * 9.0;

    // WHIP = (total walks + hits) / (total IP)
    // walks + hits per pitcher = IP * WHIP
    let total_wh: f64 = pool.iter().map(|p| p.ip * p.whip).sum();
    let league_whip = total_wh / total_ip;

    (league_era, league_whip)
}

/// Compute per-category pool stats for hitters in the given pool.
pub fn compute_hitter_pool_stats(
    pool: &[&HitterProjection],
    league_avg_avg: f64,
) -> HitterPoolStats {
    let r_vals: Vec<f64> = pool.iter().map(|h| h.r as f64).collect();
    let hr_vals: Vec<f64> = pool.iter().map(|h| h.hr as f64).collect();
    let rbi_vals: Vec<f64> = pool.iter().map(|h| h.rbi as f64).collect();
    let bb_vals: Vec<f64> = pool.iter().map(|h| h.bb as f64).collect();
    let sb_vals: Vec<f64> = pool.iter().map(|h| h.sb as f64).collect();
    let avg_contrib_vals: Vec<f64> = pool
        .iter()
        .map(|h| avg_contribution(h.ab, h.avg, league_avg_avg))
        .collect();

    HitterPoolStats {
        r: compute_pool_stats(&r_vals),
        hr: compute_pool_stats(&hr_vals),
        rbi: compute_pool_stats(&rbi_vals),
        bb: compute_pool_stats(&bb_vals),
        sb: compute_pool_stats(&sb_vals),
        avg_contribution: compute_pool_stats(&avg_contrib_vals),
    }
}

/// Compute per-category pool stats for pitchers in a combined SP+RP pool.
pub fn compute_pitcher_pool_stats(
    pool: &[&PitcherProjection],
    league_avg_era: f64,
    league_avg_whip: f64,
) -> PitcherPoolStats {
    let k_vals: Vec<f64> = pool.iter().map(|p| p.k as f64).collect();
    let w_vals: Vec<f64> = pool.iter().map(|p| p.w as f64).collect();
    let sv_vals: Vec<f64> = pool.iter().map(|p| p.sv as f64).collect();
    let hd_vals: Vec<f64> = pool.iter().map(|p| p.hd as f64).collect();
    let era_contrib_vals: Vec<f64> = pool
        .iter()
        .map(|p| era_contribution(p.ip, p.era, league_avg_era))
        .collect();
    let whip_contrib_vals: Vec<f64> = pool
        .iter()
        .map(|p| whip_contribution(p.ip, p.whip, league_avg_whip))
        .collect();

    PitcherPoolStats {
        k: compute_pool_stats(&k_vals),
        w: compute_pool_stats(&w_vals),
        sv: compute_pool_stats(&sv_vals),
        hd: compute_pool_stats(&hd_vals),
        era_contribution: compute_pool_stats(&era_contrib_vals),
        whip_contribution: compute_pool_stats(&whip_contrib_vals),
    }
}

// ---------------------------------------------------------------------------
// Z-score computation for individual players
// ---------------------------------------------------------------------------

fn compute_hitter_zscores(
    hitter: &HitterProjection,
    stats: &HitterPoolStats,
    league_avg_avg: f64,
    weights: &CategoryWeights,
) -> HitterZScores {
    let r = compute_zscore(hitter.r as f64, &stats.r);
    let hr = compute_zscore(hitter.hr as f64, &stats.hr);
    let rbi = compute_zscore(hitter.rbi as f64, &stats.rbi);
    let bb = compute_zscore(hitter.bb as f64, &stats.bb);
    let sb = compute_zscore(hitter.sb as f64, &stats.sb);
    let avg = compute_zscore(
        avg_contribution(hitter.ab, hitter.avg, league_avg_avg),
        &stats.avg_contribution,
    );

    let total = r * weights.R
        + hr * weights.HR
        + rbi * weights.RBI
        + bb * weights.BB
        + sb * weights.SB
        + avg * weights.AVG;

    HitterZScores {
        r,
        hr,
        rbi,
        bb,
        sb,
        avg,
        total,
    }
}

fn compute_pitcher_zscores(
    pitcher: &PitcherProjection,
    stats: &PitcherPoolStats,
    league_avg_era: f64,
    league_avg_whip: f64,
    weights: &CategoryWeights,
) -> PitcherZScores {
    let k = compute_zscore(pitcher.k as f64, &stats.k);
    let w = compute_zscore(pitcher.w as f64, &stats.w);
    let sv = compute_zscore(pitcher.sv as f64, &stats.sv);
    let hd = compute_zscore(pitcher.hd as f64, &stats.hd);
    let era = compute_zscore(
        era_contribution(pitcher.ip, pitcher.era, league_avg_era),
        &stats.era_contribution,
    );
    let whip = compute_zscore(
        whip_contribution(pitcher.ip, pitcher.whip, league_avg_whip),
        &stats.whip_contribution,
    );

    let total = k * weights.K
        + w * weights.W
        + sv * weights.SV
        + hd * weights.HD
        + era * weights.ERA
        + whip * weights.WHIP;

    PitcherZScores {
        k,
        w,
        sv,
        hd,
        era,
        whip,
        total,
    }
}

// ---------------------------------------------------------------------------
// Top-level entry point
// ---------------------------------------------------------------------------

/// Compute initial z-scores for all players, returning a `Vec<PlayerValuation>`
/// sorted descending by total z-score.
///
/// Steps:
/// 1. Filter hitter/SP/RP pools according to config thresholds.
/// 2. Compute league averages for rate stats from the filtered pools.
/// 3. Compute per-category pool stats (using volume-weighted contributions
///    for AVG, ERA, WHIP).
/// 4. Score every player (including those below the pool threshold) against
///    the pool stats.
/// 5. Apply category weights and sum to a total z-score.
///
/// Fields `vor`, `best_position`, and `dollar_value` are left at defaults
/// for downstream pipeline stages.
pub fn compute_initial_zscores(
    projections: &AllProjections,
    config: &Config,
) -> Vec<PlayerValuation> {
    let pool_cfg = &config.strategy.pool;
    let weights = &config.strategy.weights;

    // ---- 1. Filter pools ----
    let hitter_pool = filter_hitter_pool(&projections.hitters, pool_cfg);
    let sp_pool = filter_sp_pool(&projections.pitchers, pool_cfg);
    let rp_pool = filter_rp_pool(&projections.pitchers, pool_cfg);

    // Combined pitcher pool for shared stats
    let pitcher_pool: Vec<&PitcherProjection> = sp_pool
        .iter()
        .chain(rp_pool.iter())
        .copied()
        .collect();

    // ---- 2. League averages for rate stats ----
    let league_avg_avg = compute_league_avg_hitter(&hitter_pool);
    let (league_avg_era, league_avg_whip) = compute_league_avg_pitcher(&pitcher_pool);

    // ---- 3. Pool stats ----
    let hitter_stats = compute_hitter_pool_stats(&hitter_pool, league_avg_avg);
    let pitcher_stats = compute_pitcher_pool_stats(&pitcher_pool, league_avg_era, league_avg_whip);

    // ---- 4+5. Score all players ----
    let mut valuations: Vec<PlayerValuation> = Vec::with_capacity(
        projections.hitters.len() + projections.pitchers.len(),
    );

    for hitter in &projections.hitters {
        let zscores = compute_hitter_zscores(hitter, &hitter_stats, league_avg_avg, weights);
        valuations.push(PlayerValuation {
            name: hitter.name.clone(),
            team: hitter.team.clone(),
            positions: Vec::new(), // Populated from ESPN roster data overlay
            is_pitcher: false,
            pitcher_type: None,
            projection: PlayerProjectionData::Hitter {
                pa: hitter.pa,
                ab: hitter.ab,
                h: hitter.h,
                hr: hitter.hr,
                r: hitter.r,
                rbi: hitter.rbi,
                bb: hitter.bb,
                sb: hitter.sb,
                avg: hitter.avg,
            },
            total_zscore: zscores.total,
            category_zscores: CategoryZScores::Hitter(zscores),
            vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
            adp: projections.adp.get(&hitter.name).copied(),
        });
    }

    for pitcher in &projections.pitchers {
        let zscores = compute_pitcher_zscores(
            pitcher,
            &pitcher_stats,
            league_avg_era,
            league_avg_whip,
            weights,
        );
        valuations.push(PlayerValuation {
            name: pitcher.name.clone(),
            team: pitcher.team.clone(),
            positions: Vec::new(),
            is_pitcher: true,
            pitcher_type: Some(pitcher.pitcher_type),
            projection: PlayerProjectionData::Pitcher {
                ip: pitcher.ip,
                k: pitcher.k,
                w: pitcher.w,
                sv: pitcher.sv,
                hd: pitcher.hd,
                era: pitcher.era,
                whip: pitcher.whip,
                g: pitcher.g,
                gs: pitcher.gs,
            },
            total_zscore: zscores.total,
            category_zscores: CategoryZScores::Pitcher(zscores),
            vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
            adp: projections.adp.get(&pitcher.name).copied(),
        });
    }

    // Sort descending by total z-score
    valuations.sort_by(|a, b| {
        b.total_zscore
            .partial_cmp(&a.total_zscore)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    valuations
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::valuation::projections::*;
    use std::collections::HashMap;

    // ---- Helpers ----

    fn approx_eq(a: f64, b: f64, epsilon: f64) -> bool {
        (a - b).abs() < epsilon
    }

    /// Build a minimal Config suitable for testing the z-score engine.
    fn test_config() -> Config {
        Config {
            league: LeagueConfig {
                name: "Test League".into(),
                platform: "espn".into(),
                num_teams: 10,
                scoring_type: "h2h_most_categories".into(),
                salary_cap: 260,
                batting_categories: CategoriesSection {
                    categories: vec![
                        "R".into(),
                        "HR".into(),
                        "RBI".into(),
                        "BB".into(),
                        "SB".into(),
                        "AVG".into(),
                    ],
                },
                pitching_categories: CategoriesSection {
                    categories: vec![
                        "K".into(),
                        "W".into(),
                        "SV".into(),
                        "HD".into(),
                        "ERA".into(),
                        "WHIP".into(),
                    ],
                },
                roster: std::collections::HashMap::new(),
                roster_limits: RosterLimits {
                    max_sp: 7,
                    max_rp: 7,
                    gs_per_week: 7,
                },
                teams: std::collections::HashMap::new(),
                my_team: MyTeam {
                    team_id: "team_1".into(),
                },
            },
            strategy: StrategyConfig {
                hitting_budget_fraction: 0.65,
                weights: CategoryWeights {
                    R: 1.0,
                    HR: 1.0,
                    RBI: 1.0,
                    BB: 1.0,
                    SB: 1.0,
                    AVG: 1.0,
                    K: 1.0,
                    W: 1.0,
                    SV: 0.7,
                    HD: 1.0,
                    ERA: 1.0,
                    WHIP: 1.0,
                },
                pool: PoolConfig {
                    min_pa: 200,
                    min_ip_sp: 50.0,
                    min_g_rp: 20,
                    hitter_pool_size: 150,
                    sp_pool_size: 70,
                    rp_pool_size: 80,
                },
                holds_estimation: HoldsEstimationConfig {
                    default_hold_rate: 0.25,
                },
                llm: LlmConfig {
                    model: "test".into(),
                    analysis_max_tokens: 400,
                    planning_max_tokens: 600,
                    analysis_trigger: "nomination".into(),
                    prefire_planning: true,
                },
            },
            credentials: CredentialsConfig {
                anthropic_api_key: None,
            },
            ws_port: 9001,
            db_path: "test.db".into(),
            data_paths: DataPaths {
                hitters: "data/projections/hitters.csv".into(),
                pitchers_sp: "data/projections/pitchers_sp.csv".into(),
                pitchers_rp: "data/projections/pitchers_rp.csv".into(),
                holds_overlay: "data/projections/holds_overlay.csv".into(),
                adp: "data/adp.csv".into(),
            },
        }
    }

    fn make_hitter(name: &str, pa: u32, ab: u32, h: u32, hr: u32, r: u32, rbi: u32, bb: u32, sb: u32) -> HitterProjection {
        let avg = if ab > 0 { h as f64 / ab as f64 } else { 0.0 };
        HitterProjection {
            name: name.into(),
            team: "TST".into(),
            pa,
            ab,
            h,
            hr,
            r,
            rbi,
            bb,
            sb,
            avg,
        }
    }

    fn make_sp(name: &str, ip: f64, k: u32, w: u32, era: f64, whip: f64) -> PitcherProjection {
        PitcherProjection {
            name: name.into(),
            team: "TST".into(),
            pitcher_type: PitcherType::SP,
            ip,
            k,
            w,
            sv: 0,
            hd: 0,
            era,
            whip,
            g: (ip / 6.0).ceil() as u32,
            gs: (ip / 6.0).ceil() as u32,
        }
    }

    fn make_rp(name: &str, ip: f64, k: u32, sv: u32, hd: u32, era: f64, whip: f64, g: u32) -> PitcherProjection {
        PitcherProjection {
            name: name.into(),
            team: "TST".into(),
            pitcher_type: PitcherType::RP,
            ip,
            k,
            w: 3,
            sv,
            hd,
            era,
            whip,
            g,
            gs: 0,
        }
    }

    // ---- compute_pool_stats tests ----

    #[test]
    fn pool_stats_known_values() {
        // Values: [2, 4, 4, 4, 5, 5, 7, 9]
        // Mean = 40/8 = 5.0
        // Population variance = ((2-5)^2 + (4-5)^2*3 + (5-5)^2*2 + (7-5)^2 + (9-5)^2) / 8
        //   = (9 + 1 + 1 + 1 + 0 + 0 + 4 + 16) / 8 = 32/8 = 4.0
        // Stdev = 2.0
        let values = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let stats = compute_pool_stats(&values);
        assert!(approx_eq(stats.mean, 5.0, 1e-10));
        assert!(approx_eq(stats.stdev, 2.0, 1e-10));
    }

    #[test]
    fn pool_stats_single_value() {
        let values = vec![42.0];
        let stats = compute_pool_stats(&values);
        assert!(approx_eq(stats.mean, 42.0, 1e-10));
        assert!(approx_eq(stats.stdev, 0.0, 1e-10));
    }

    #[test]
    fn pool_stats_empty() {
        let stats = compute_pool_stats(&[]);
        assert!(approx_eq(stats.mean, 0.0, 1e-10));
        assert!(approx_eq(stats.stdev, 0.0, 1e-10));
    }

    // ---- compute_zscore tests ----

    #[test]
    fn zscore_known_inputs() {
        let stats = PoolStats {
            mean: 5.0,
            stdev: 2.0,
        };
        // Value 9 => z = (9-5)/2 = 2.0
        assert!(approx_eq(compute_zscore(9.0, &stats), 2.0, 1e-10));
        // Value 1 => z = (1-5)/2 = -2.0
        assert!(approx_eq(compute_zscore(1.0, &stats), -2.0, 1e-10));
        // Value 5 => z = 0.0
        assert!(approx_eq(compute_zscore(5.0, &stats), 0.0, 1e-10));
    }

    #[test]
    fn zscore_zero_stdev_returns_zero() {
        let stats = PoolStats {
            mean: 42.0,
            stdev: 0.0,
        };
        assert!(approx_eq(compute_zscore(100.0, &stats), 0.0, 1e-10));
    }

    #[test]
    fn zscore_near_zero_stdev_returns_zero() {
        let stats = PoolStats {
            mean: 10.0,
            stdev: 1e-12,
        };
        assert!(approx_eq(compute_zscore(100.0, &stats), 0.0, 1e-10));
    }

    // ---- Rate stat contribution tests ----

    #[test]
    fn era_contribution_below_average_is_positive() {
        // Pitcher with 2.50 ERA vs league avg 4.00
        // contribution = 180 * (4.00 - 2.50) / 9 = 180 * 1.5 / 9 = 30.0
        let contrib = era_contribution(180.0, 2.50, 4.00);
        assert!(approx_eq(contrib, 30.0, 1e-10));
        assert!(contrib > 0.0);
    }

    #[test]
    fn era_contribution_above_average_is_negative() {
        // Pitcher with 5.00 ERA vs league avg 4.00
        // contribution = 180 * (4.00 - 5.00) / 9 = 180 * (-1.0) / 9 = -20.0
        let contrib = era_contribution(180.0, 5.00, 4.00);
        assert!(approx_eq(contrib, -20.0, 1e-10));
        assert!(contrib < 0.0);
    }

    #[test]
    fn whip_contribution_below_average_is_positive() {
        // Pitcher with 1.00 WHIP vs league avg 1.30
        // contribution = 200 * (1.30 - 1.00) = 200 * 0.30 = 60.0
        let contrib = whip_contribution(200.0, 1.00, 1.30);
        assert!(approx_eq(contrib, 60.0, 1e-10));
        assert!(contrib > 0.0);
    }

    #[test]
    fn whip_contribution_above_average_is_negative() {
        let contrib = whip_contribution(200.0, 1.50, 1.30);
        assert!(contrib < 0.0);
    }

    #[test]
    fn avg_contribution_above_average_is_positive() {
        // Hitter with .300 AVG vs league avg .260
        // contribution = 500 * (0.300 - 0.260) = 500 * 0.040 = 20.0
        let contrib = avg_contribution(500, 0.300, 0.260);
        assert!(approx_eq(contrib, 20.0, 1e-10));
        assert!(contrib > 0.0);
    }

    #[test]
    fn avg_contribution_below_average_is_negative() {
        let contrib = avg_contribution(500, 0.220, 0.260);
        assert!(contrib < 0.0);
    }

    // ---- Category weights test ----

    #[test]
    fn category_weights_applied_correctly() {
        // Create a scenario where SV weight (0.7) changes the total
        let stats = PitcherPoolStats {
            k: PoolStats { mean: 100.0, stdev: 30.0 },
            w: PoolStats { mean: 8.0, stdev: 3.0 },
            sv: PoolStats { mean: 10.0, stdev: 10.0 },
            hd: PoolStats { mean: 5.0, stdev: 5.0 },
            era_contribution: PoolStats { mean: 0.0, stdev: 10.0 },
            whip_contribution: PoolStats { mean: 0.0, stdev: 10.0 },
        };

        // Create a closer with big SV numbers
        let closer = PitcherProjection {
            name: "Closer".into(),
            team: "TST".into(),
            pitcher_type: PitcherType::RP,
            ip: 60.0,
            k: 100,
            w: 8,
            sv: 40,
            hd: 5,
            era: 3.00,
            whip: 1.10,
            g: 60,
            gs: 0,
        };

        let weights_equal = CategoryWeights {
            R: 1.0, HR: 1.0, RBI: 1.0, BB: 1.0, SB: 1.0, AVG: 1.0,
            K: 1.0, W: 1.0, SV: 1.0, HD: 1.0, ERA: 1.0, WHIP: 1.0,
        };

        let weights_reduced_sv = CategoryWeights {
            R: 1.0, HR: 1.0, RBI: 1.0, BB: 1.0, SB: 1.0, AVG: 1.0,
            K: 1.0, W: 1.0, SV: 0.7, HD: 1.0, ERA: 1.0, WHIP: 1.0,
        };

        let zscores_equal = compute_pitcher_zscores(
            &closer, &stats, 4.00, 1.30, &weights_equal,
        );
        let zscores_reduced = compute_pitcher_zscores(
            &closer, &stats, 4.00, 1.30, &weights_reduced_sv,
        );

        // SV z-score for this closer: (40 - 10) / 10 = 3.0
        assert!(approx_eq(zscores_equal.sv, 3.0, 1e-10));
        assert!(approx_eq(zscores_reduced.sv, 3.0, 1e-10)); // Raw z-score unchanged

        // Total should differ: equal has SV*1.0=3.0, reduced has SV*0.7=2.1
        // Difference = 3.0 - 2.1 = 0.9
        let diff = zscores_equal.total - zscores_reduced.total;
        assert!(approx_eq(diff, 0.9, 1e-10));
    }

    // ---- Pool filtering tests ----

    #[test]
    fn filter_hitter_pool_respects_min_pa_and_size() {
        let hitters: Vec<HitterProjection> = (0..10)
            .map(|i| {
                let pa = 150 + i * 20; // 150, 170, 190, 210, 230, ...
                make_hitter(&format!("Hitter{}", i), pa, pa - 50, pa / 3, 10 + i, 50 + i * 5, 50 + i * 5, 30 + i, 5 + i)
            })
            .collect();

        let pool_cfg = PoolConfig {
            min_pa: 200,
            min_ip_sp: 50.0,
            min_g_rp: 20,
            hitter_pool_size: 3,
            sp_pool_size: 70,
            rp_pool_size: 80,
        };

        let pool = filter_hitter_pool(&hitters, &pool_cfg);

        // PA values >= 200: 210, 230, 250, 270, 290, 310, 330 (7 qualify)
        // Top 3 by PA: 330, 310, 290
        assert_eq!(pool.len(), 3);
        assert_eq!(pool[0].pa, 330);
        assert_eq!(pool[1].pa, 310);
        assert_eq!(pool[2].pa, 290);
    }

    #[test]
    fn filter_sp_pool_respects_min_ip_and_size() {
        let pitchers: Vec<PitcherProjection> = (0..8)
            .map(|i| {
                let ip = 40.0 + (i as f64) * 15.0; // 40, 55, 70, 85, ...
                make_sp(&format!("SP{}", i), ip, 100 + i * 10, 8 + i, 3.50, 1.20)
            })
            .collect();

        let pool_cfg = PoolConfig {
            min_pa: 200,
            min_ip_sp: 50.0,
            min_g_rp: 20,
            hitter_pool_size: 150,
            sp_pool_size: 3,
            rp_pool_size: 80,
        };

        let pool = filter_sp_pool(&pitchers, &pool_cfg);

        // IP >= 50: 55, 70, 85, 100, 115, 130, 145 (7 qualify)
        // Top 3 by IP: 145, 130, 115
        assert_eq!(pool.len(), 3);
        assert!(approx_eq(pool[0].ip, 145.0, 1e-10));
        assert!(approx_eq(pool[1].ip, 130.0, 1e-10));
        assert!(approx_eq(pool[2].ip, 115.0, 1e-10));
    }

    #[test]
    fn filter_rp_pool_respects_min_g_and_size() {
        let pitchers: Vec<PitcherProjection> = (0..6)
            .map(|i| {
                let g = 15 + i * 5; // 15, 20, 25, 30, 35, 40
                make_rp(&format!("RP{}", i), 50.0, 50, 10, 5, 3.00, 1.10, g)
            })
            .collect();

        let pool_cfg = PoolConfig {
            min_pa: 200,
            min_ip_sp: 50.0,
            min_g_rp: 20,
            hitter_pool_size: 150,
            sp_pool_size: 70,
            rp_pool_size: 2,
        };

        let pool = filter_rp_pool(&pitchers, &pool_cfg);

        // G >= 20: 20, 25, 30, 35, 40 (5 qualify)
        // Top 2 by G: 40, 35
        assert_eq!(pool.len(), 2);
        assert_eq!(pool[0].g, 40);
        assert_eq!(pool[1].g, 35);
    }

    // ---- Synthetic dataset integration test ----

    #[test]
    fn synthetic_dataset_5_hitters_5_pitchers() {
        let hitters = vec![
            make_hitter("Elite", 650, 580, 185, 45, 115, 120, 70, 20),
            make_hitter("Good", 600, 540, 155, 30, 90, 85, 55, 12),
            make_hitter("Average", 550, 500, 130, 20, 70, 65, 45, 8),
            make_hitter("Below", 500, 460, 110, 12, 55, 50, 35, 4),
            make_hitter("Replacement", 450, 410, 95, 8, 40, 35, 30, 2),
        ];

        let pitchers = vec![
            make_sp("Ace", 200.0, 250, 18, 2.50, 0.95),
            make_sp("Solid SP", 180.0, 190, 14, 3.30, 1.10),
            make_sp("Average SP", 160.0, 150, 10, 4.00, 1.25),
            make_rp("Elite Closer", 65.0, 80, 40, 5, 2.00, 0.90, 60),
            make_rp("Setup Man", 70.0, 75, 5, 25, 3.00, 1.10, 65),
        ];

        let adp = HashMap::new();

        let projections = AllProjections {
            hitters,
            pitchers,
            adp,
        };

        // Config with pools small enough to include all players
        let mut config = test_config();
        config.strategy.pool.min_pa = 200;
        config.strategy.pool.hitter_pool_size = 5;
        config.strategy.pool.min_ip_sp = 50.0;
        config.strategy.pool.sp_pool_size = 5;
        config.strategy.pool.min_g_rp = 20;
        config.strategy.pool.rp_pool_size = 5;

        let valuations = compute_initial_zscores(&projections, &config);

        // Should have all 10 players
        assert_eq!(valuations.len(), 10);

        // First player should have highest z-score
        assert!(valuations[0].total_zscore >= valuations[1].total_zscore);

        // All z-scores should be finite
        for v in &valuations {
            assert!(v.total_zscore.is_finite(), "Player {} has non-finite z-score", v.name);
        }

        // Elite hitter should have positive z-scores in counting stats
        let elite = valuations.iter().find(|v| v.name == "Elite").unwrap();
        match elite.category_zscores {
            CategoryZScores::Hitter(ref z) => {
                assert!(z.r > 0.0, "Elite R z-score should be positive");
                assert!(z.hr > 0.0, "Elite HR z-score should be positive");
                assert!(z.rbi > 0.0, "Elite RBI z-score should be positive");
                assert!(z.bb > 0.0, "Elite BB z-score should be positive");
                assert!(z.sb > 0.0, "Elite SB z-score should be positive");
            }
            _ => panic!("Elite should be a hitter"),
        }

        // Replacement-level hitter should have negative z-scores
        let replacement = valuations.iter().find(|v| v.name == "Replacement").unwrap();
        match replacement.category_zscores {
            CategoryZScores::Hitter(ref z) => {
                assert!(z.r < 0.0, "Replacement R z-score should be negative");
                assert!(z.hr < 0.0, "Replacement HR z-score should be negative");
            }
            _ => panic!("Replacement should be a hitter"),
        }

        // Ace should have positive ERA contribution z-score (below-avg ERA = good)
        let ace = valuations.iter().find(|v| v.name == "Ace").unwrap();
        match ace.category_zscores {
            CategoryZScores::Pitcher(ref z) => {
                assert!(z.era > 0.0, "Ace ERA z-score should be positive (good ERA)");
                assert!(z.whip > 0.0, "Ace WHIP z-score should be positive (good WHIP)");
                assert!(z.k > 0.0, "Ace K z-score should be positive");
            }
            _ => panic!("Ace should be a pitcher"),
        }

        // Check that SV weight of 0.7 is applied (closer has high SV z-score)
        let closer = valuations.iter().find(|v| v.name == "Elite Closer").unwrap();
        assert!(closer.is_pitcher);
        assert_eq!(closer.pitcher_type, Some(PitcherType::RP));

        // Verify is_pitcher and pitcher_type set correctly
        assert!(!elite.is_pitcher);
        assert_eq!(elite.pitcher_type, None);
        assert!(ace.is_pitcher);
        assert_eq!(ace.pitcher_type, Some(PitcherType::SP));

        // VOR, best_position, dollar_value should be defaults
        assert!(approx_eq(elite.vor, 0.0, 1e-10));
        assert!(elite.best_position.is_none());
        assert!(approx_eq(elite.dollar_value, 0.0, 1e-10));
    }

    // ---- ADP lookup test ----

    #[test]
    fn adp_lookup_works() {
        let hitters = vec![
            make_hitter("Player A", 600, 540, 155, 30, 90, 85, 55, 12),
            make_hitter("Player B", 550, 500, 130, 20, 70, 65, 45, 8),
        ];

        let pitchers = vec![
            make_sp("Pitcher A", 180.0, 190, 14, 3.30, 1.10),
        ];

        let mut adp = HashMap::new();
        adp.insert("Player A".to_string(), 5.0);
        adp.insert("Pitcher A".to_string(), 15.0);

        let projections = AllProjections { hitters, pitchers, adp };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 10;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 10;

        let valuations = compute_initial_zscores(&projections, &config);

        let a = valuations.iter().find(|v| v.name == "Player A").unwrap();
        assert!(approx_eq(a.adp.unwrap(), 5.0, 1e-10));

        let b = valuations.iter().find(|v| v.name == "Player B").unwrap();
        assert!(b.adp.is_none());

        let pa = valuations.iter().find(|v| v.name == "Pitcher A").unwrap();
        assert!(approx_eq(pa.adp.unwrap(), 15.0, 1e-10));
    }

    // ---- Zero stdev edge case ----

    #[test]
    fn zero_stdev_all_identical_players() {
        // All hitters have identical stats => stdev = 0 => all z-scores = 0
        let hitters: Vec<HitterProjection> = (0..5)
            .map(|i| HitterProjection {
                name: format!("Clone{}", i),
                team: "TST".into(),
                pa: 600,
                ab: 540,
                h: 150,
                hr: 25,
                r: 80,
                rbi: 75,
                bb: 50,
                sb: 10,
                avg: 150.0 / 540.0,
            })
            .collect();

        let pitchers = vec![
            make_sp("SP1", 180.0, 190, 14, 3.30, 1.10),
        ];

        let projections = AllProjections {
            hitters,
            pitchers,
            adp: HashMap::new(),
        };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 10;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 10;

        let valuations = compute_initial_zscores(&projections, &config);

        // All clone hitters should have 0 total z-score (all categories have stdev ≈ 0)
        for v in valuations.iter().filter(|v| v.name.starts_with("Clone")) {
            assert!(
                approx_eq(v.total_zscore, 0.0, 1e-10),
                "Clone hitter {} should have z-score 0, got {}",
                v.name,
                v.total_zscore
            );
        }
    }

    // ---- Volume-weighting correctness test ----

    #[test]
    fn volume_weighting_era_matters() {
        // Two pitchers: same ERA but different IP
        // Higher IP pitcher should have bigger (more positive) ERA contribution
        // when ERA is below league avg
        let league_avg_era = 4.00;
        let era = 3.00;

        let contrib_high_ip = era_contribution(200.0, era, league_avg_era);
        let contrib_low_ip = era_contribution(60.0, era, league_avg_era);

        // Both positive (below avg ERA)
        assert!(contrib_high_ip > 0.0);
        assert!(contrib_low_ip > 0.0);

        // High IP has proportionally more contribution
        assert!(
            approx_eq(contrib_high_ip / contrib_low_ip, 200.0 / 60.0, 1e-10),
            "ERA contribution should scale linearly with IP"
        );
    }

    #[test]
    fn volume_weighting_avg_matters() {
        // Two hitters: same AVG but different AB
        let league_avg = 0.260;
        let avg = 0.300;

        let contrib_high_ab = avg_contribution(600, avg, league_avg);
        let contrib_low_ab = avg_contribution(300, avg, league_avg);

        assert!(contrib_high_ab > 0.0);
        assert!(contrib_low_ab > 0.0);
        assert!(approx_eq(contrib_high_ab / contrib_low_ab, 2.0, 1e-10));
    }

    // ---- Pool stats computation test ----

    #[test]
    fn hitter_pool_stats_calculation() {
        let hitters = vec![
            make_hitter("A", 600, 540, 162, 30, 100, 90, 60, 15),
            make_hitter("B", 580, 520, 140, 20, 80, 70, 50, 10),
            make_hitter("C", 550, 500, 125, 15, 60, 55, 40, 5),
        ];
        let pool: Vec<&HitterProjection> = hitters.iter().collect();

        // League avg = total H / total AB = (162+140+125) / (540+520+500) = 427/1560
        let league_avg = compute_league_avg_hitter(&pool);
        assert!(approx_eq(league_avg, 427.0 / 1560.0, 1e-10));

        let stats = compute_hitter_pool_stats(&pool, league_avg);

        // Mean R = (100+80+60)/3 = 80
        assert!(approx_eq(stats.r.mean, 80.0, 1e-10));

        // Mean HR = (30+20+15)/3 = 65/3 ≈ 21.67
        assert!(approx_eq(stats.hr.mean, 65.0 / 3.0, 1e-10));

        // Stdev R: population stdev of [100, 80, 60]
        // mean=80, var = ((20^2 + 0 + 20^2)/3) = 800/3 ≈ 266.67
        // stdev = sqrt(800/3) ≈ 16.33
        let expected_r_stdev = (800.0_f64 / 3.0).sqrt();
        assert!(approx_eq(stats.r.stdev, expected_r_stdev, 1e-10));
    }

    // ---- Pitcher pool stats: league average ERA/WHIP ----

    #[test]
    fn pitcher_league_averages() {
        let pitchers = vec![
            make_sp("SP1", 200.0, 200, 15, 3.00, 1.10),
            make_sp("SP2", 180.0, 180, 12, 4.00, 1.20),
        ];
        let pool: Vec<&PitcherProjection> = pitchers.iter().collect();

        let (league_era, league_whip) = compute_league_avg_pitcher(&pool);

        // Total IP = 380
        // Total ER = (200 * 3.00/9) + (180 * 4.00/9) = 66.667 + 80.0 = 146.667
        // League ERA = 146.667 / 380 * 9 = 3.4737...
        let expected_era = (200.0 * 3.00 / 9.0 + 180.0 * 4.00 / 9.0) / 380.0 * 9.0;
        assert!(approx_eq(league_era, expected_era, 1e-10));

        // Total WH = (200 * 1.10) + (180 * 1.20) = 220 + 216 = 436
        // League WHIP = 436 / 380 = 1.1473...
        let expected_whip = (200.0 * 1.10 + 180.0 * 1.20) / 380.0;
        assert!(approx_eq(league_whip, expected_whip, 1e-10));
    }

    // ---- Output ordering ----

    #[test]
    fn output_sorted_descending_by_total_zscore() {
        let hitters = vec![
            make_hitter("Bad", 450, 410, 95, 5, 35, 30, 25, 1),
            make_hitter("Great", 650, 580, 190, 50, 120, 130, 80, 25),
            make_hitter("Ok", 550, 500, 135, 20, 70, 65, 45, 8),
        ];

        let pitchers = vec![
            make_sp("SP1", 180.0, 190, 14, 3.30, 1.10),
        ];

        let projections = AllProjections {
            hitters,
            pitchers,
            adp: HashMap::new(),
        };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 10;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 10;

        let valuations = compute_initial_zscores(&projections, &config);

        for w in valuations.windows(2) {
            assert!(
                w[0].total_zscore >= w[1].total_zscore,
                "Not sorted: {} ({}) before {} ({})",
                w[0].name,
                w[0].total_zscore,
                w[1].name,
                w[1].total_zscore
            );
        }
    }

    // ---- Players below pool threshold still get scored ----

    #[test]
    fn below_threshold_players_still_scored() {
        // One hitter above threshold, one below
        let hitters = vec![
            make_hitter("Qualified", 600, 540, 160, 30, 90, 85, 55, 12),
            make_hitter("PartTime", 100, 90, 25, 5, 15, 12, 8, 2),
        ];

        let pitchers = vec![
            make_sp("SP1", 180.0, 190, 14, 3.30, 1.10),
        ];

        let projections = AllProjections {
            hitters,
            pitchers,
            adp: HashMap::new(),
        };

        let mut config = test_config();
        config.strategy.pool.min_pa = 200;
        config.strategy.pool.hitter_pool_size = 10;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 10;

        let valuations = compute_initial_zscores(&projections, &config);

        // Both players should be in output
        assert_eq!(valuations.len(), 3);

        let part_time = valuations.iter().find(|v| v.name == "PartTime").unwrap();
        // Part-time player should have a z-score (likely negative since below pool threshold)
        assert!(part_time.total_zscore.is_finite());
    }

    // ---- Projection data carried through ----

    #[test]
    fn projection_data_preserved() {
        let hitters = vec![
            make_hitter("TestHitter", 600, 540, 160, 30, 90, 85, 55, 12),
        ];

        let pitchers = vec![
            make_sp("TestPitcher", 180.0, 190, 14, 3.30, 1.10),
        ];

        let projections = AllProjections {
            hitters,
            pitchers,
            adp: HashMap::new(),
        };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 10;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 10;

        let valuations = compute_initial_zscores(&projections, &config);

        let hitter = valuations.iter().find(|v| v.name == "TestHitter").unwrap();
        match &hitter.projection {
            PlayerProjectionData::Hitter { pa, ab, hr, r, rbi, bb, sb, avg, .. } => {
                assert_eq!(*pa, 600);
                assert_eq!(*ab, 540);
                assert_eq!(*hr, 30);
                assert_eq!(*r, 90);
                assert_eq!(*rbi, 85);
                assert_eq!(*bb, 55);
                assert_eq!(*sb, 12);
                assert!(approx_eq(*avg, 160.0 / 540.0, 1e-10));
            }
            _ => panic!("Expected Hitter projection"),
        }

        let pitcher = valuations.iter().find(|v| v.name == "TestPitcher").unwrap();
        match &pitcher.projection {
            PlayerProjectionData::Pitcher { ip, k, w, era, whip, .. } => {
                assert!(approx_eq(*ip, 180.0, 1e-10));
                assert_eq!(*k, 190);
                assert_eq!(*w, 14);
                assert!(approx_eq(*era, 3.30, 1e-10));
                assert!(approx_eq(*whip, 1.10, 1e-10));
            }
            _ => panic!("Expected Pitcher projection"),
        }
    }
}
