// Z-score calculation with volume-weighted rate stats.

use std::collections::HashMap;

use crate::config::{CategoryWeights, Config, PoolConfig};
use crate::draft::pick::Position;
use crate::stats::{self, CategoryValues, StatComputation, StatRegistry};
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
// CategoryWeights → CategoryValues conversion
// ---------------------------------------------------------------------------

/// Convert a `CategoryWeights` HashMap into a registry-indexed `CategoryValues`
/// vector suitable for `weighted_sum()`.
pub fn weights_to_category_values(
    weights: &CategoryWeights,
    registry: &StatRegistry,
) -> CategoryValues {
    let mut cv = CategoryValues::zeros(registry.len());
    for (idx, def) in registry.all_stats().iter().enumerate() {
        cv.set(idx, weights.weight(&def.abbrev));
    }
    cv
}

// ---------------------------------------------------------------------------
// Per-category z-scores (registry-indexed via CategoryValues)
// ---------------------------------------------------------------------------

/// Per-category z-scores for a player, stored as a full-length CategoryValues
/// vector indexed by StatRegistry position. Hitter variants have 0.0 at
/// pitching indices; Pitcher variants have 0.0 at batting indices.
#[derive(Debug, Clone)]
pub enum CategoryZScores {
    Hitter {
        zscores: CategoryValues,
        total: f64,
    },
    Pitcher {
        zscores: CategoryValues,
        total: f64,
    },
    TwoWay {
        zscores: CategoryValues,
        batting_total: f64,
        pitching_total: f64,
        total: f64,
    },
}

impl CategoryZScores {
    /// Get the total weighted z-score.
    pub fn total(&self) -> f64 {
        match self {
            Self::Hitter { total, .. } => *total,
            Self::Pitcher { total, .. } => *total,
            Self::TwoWay { total, .. } => *total,
        }
    }

    /// Get the full-length z-score vector.
    pub fn zscores(&self) -> &CategoryValues {
        match self {
            Self::Hitter { zscores, .. } => zscores,
            Self::Pitcher { zscores, .. } => zscores,
            Self::TwoWay { zscores, .. } => zscores,
        }
    }

    /// Look up a specific category's z-score by abbreviation.
    pub fn get_by_abbrev(&self, registry: &StatRegistry, abbrev: &str) -> Option<f64> {
        let idx = registry.index_of(abbrev)?;
        self.zscores().get(idx)
    }

    /// For TwoWay players, return the batting sub-total. For Hitter, return total.
    /// For Pitcher, return 0.0.
    pub fn batting_total(&self) -> f64 {
        match self {
            Self::Hitter { total, .. } => *total,
            Self::TwoWay { batting_total, .. } => *batting_total,
            Self::Pitcher { .. } => 0.0,
        }
    }

    /// For TwoWay players, return the pitching sub-total. For Pitcher, return total.
    /// For Hitter, return 0.0.
    pub fn pitching_total(&self) -> f64 {
        match self {
            Self::Pitcher { total, .. } => *total,
            Self::TwoWay { pitching_total, .. } => *pitching_total,
            Self::Hitter { .. } => 0.0,
        }
    }

    /// Build a Hitter variant.
    pub fn hitter(zscores: CategoryValues, total: f64) -> Self {
        Self::Hitter { zscores, total }
    }

    /// Build a Pitcher variant.
    pub fn pitcher(zscores: CategoryValues, total: f64) -> Self {
        Self::Pitcher { zscores, total }
    }

    /// Build a TwoWay variant. Computes total as batting_total + pitching_total.
    pub fn two_way(
        zscores: CategoryValues,
        batting_total: f64,
        pitching_total: f64,
    ) -> Self {
        Self::TwoWay {
            zscores,
            batting_total,
            pitching_total,
            total: batting_total + pitching_total,
        }
    }

    /// Build a zeroed-out Hitter variant (for tests/placeholders).
    pub fn zeros_hitter(n: usize) -> Self {
        Self::Hitter {
            zscores: CategoryValues::zeros(n),
            total: 0.0,
        }
    }

    /// Build a zeroed-out Pitcher variant (for tests/placeholders).
    pub fn zeros_pitcher(n: usize) -> Self {
        Self::Pitcher {
            zscores: CategoryValues::zeros(n),
            total: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Projection data (carried through the pipeline)
// ---------------------------------------------------------------------------

/// Raw projection numbers carried forward through the valuation pipeline.
///
/// Keys are lowercase field names matching CSV/ESPN columns:
/// "pa", "ab", "h", "hr", "r", "rbi", "bb", "sb", "avg",
/// "ip", "k", "w", "sv", "hd", "era", "whip", "g", "gs".
///
/// The hitter/pitcher distinction is carried by `PlayerValuation.is_pitcher`
/// and `PlayerValuation.is_two_way` flags. A two-way player's ProjectionData
/// contains both hitting and pitching keys merged together.
#[derive(Debug, Clone)]
pub struct ProjectionData {
    pub values: HashMap<String, f64>,
}

impl ProjectionData {
    /// Look up a projection value by key, returning 0.0 if not present.
    pub fn get(&self, key: &str) -> f64 {
        self.values.get(key).copied().unwrap_or(0.0)
    }

    /// Merge another ProjectionData into this one (for two-way players).
    pub fn merge(&mut self, other: &ProjectionData) {
        for (k, v) in &other.values {
            self.values.insert(k.clone(), *v);
        }
    }
}

impl From<&HitterProjection> for ProjectionData {
    fn from(h: &HitterProjection) -> Self {
        ProjectionData {
            values: HashMap::from([
                ("pa".into(), h.pa as f64),
                ("ab".into(), h.ab as f64),
                ("h".into(), h.h as f64),
                ("hr".into(), h.hr as f64),
                ("r".into(), h.r as f64),
                ("rbi".into(), h.rbi as f64),
                ("bb".into(), h.bb as f64),
                ("sb".into(), h.sb as f64),
                ("avg".into(), h.avg),
            ]),
        }
    }
}

impl From<&PitcherProjection> for ProjectionData {
    fn from(p: &PitcherProjection) -> Self {
        ProjectionData {
            values: HashMap::from([
                ("ip".into(), p.ip),
                ("k".into(), p.k as f64),
                ("w".into(), p.w as f64),
                ("sv".into(), p.sv as f64),
                ("hd".into(), p.hd as f64),
                ("era".into(), p.era),
                ("whip".into(), p.whip),
                ("g".into(), p.g as f64),
                ("gs".into(), p.gs as f64),
            ]),
        }
    }
}

impl From<&ProjectionData> for stats::ProjectionData {
    fn from(proj: &ProjectionData) -> Self {
        let mut data = stats::ProjectionData::new();
        for (k, v) in &proj.values {
            data.insert(k.as_str(), *v);
        }
        data
    }
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
    /// Whether this player has both hitting and pitching projections.
    /// Two-way players have `is_pitcher = false` (they fill a hitter slot)
    /// but contribute to pitching categories as well.
    pub is_two_way: bool,
    pub pitcher_type: Option<PitcherType>,
    pub projection: ProjectionData,
    pub total_zscore: f64,
    pub category_zscores: CategoryZScores,
    pub vor: f64,
    /// Snapshot of VOR from the initial (full-pool) computation.
    /// Used by scarcity calculation so that shrinking the available pool
    /// does not inflate the count of players "above replacement."
    pub initial_vor: f64,
    pub best_position: Option<Position>,
    pub dollar_value: f64,
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
// Generic pool stats and z-score computation (registry-driven)
// ---------------------------------------------------------------------------

/// Compute per-category pool statistics for a group of projections.
///
/// For `Counting` stats: collects raw values via `projection_key` and computes
/// mean/stdev. For `RateStat` stats: computes the volume-weighted league
/// average, then converts each player to a contribution via
/// `rate_stat_contribution`, and computes mean/stdev on the contributions.
///
/// Returns `(pool_stats, league_avgs)` where `pool_stats` is a full-length
/// `Vec<PoolStats>` indexed by registry position (zeroed at unused indices)
/// and `league_avgs` maps rate-stat category indices to their league average.
pub(crate) fn compute_generic_pool_stats(
    pool: &[stats::ProjectionData],
    category_indices: &[usize],
    registry: &StatRegistry,
) -> (Vec<PoolStats>, HashMap<usize, f64>) {
    let zero = PoolStats { mean: 0.0, stdev: 0.0 };
    let mut result = vec![zero; registry.len()];
    let mut league_avgs: HashMap<usize, f64> = HashMap::new();
    let all_stats = registry.all_stats();

    for &cat_idx in category_indices {
        let def = &all_stats[cat_idx];
        match &def.computation {
            StatComputation::Counting { projection_key } => {
                let values: Vec<f64> = pool.iter()
                    .map(|p| p.get_or_zero(projection_key))
                    .collect();
                result[cat_idx] = compute_pool_stats(&values);
            }
            StatComputation::RateStat { volume_key, rate_key, divisor } => {
                let total_volume: f64 = pool.iter()
                    .map(|p| p.get_or_zero(volume_key))
                    .sum();
                let league_avg = if total_volume > STDEV_EPSILON {
                    let weighted_sum: f64 = pool.iter()
                        .map(|p| p.get_or_zero(volume_key) * p.get_or_zero(rate_key))
                        .sum();
                    weighted_sum / total_volume
                } else {
                    0.0
                };
                league_avgs.insert(cat_idx, league_avg);

                let contrib_values: Vec<f64> = pool.iter()
                    .map(|p| stats::rate_stat_contribution(
                        p.get_or_zero(volume_key),
                        p.get_or_zero(rate_key),
                        league_avg,
                        *divisor,
                        def.sort_direction,
                    ))
                    .collect();
                result[cat_idx] = compute_pool_stats(&contrib_values);
            }
        }
    }

    (result, league_avgs)
}

/// Compute z-scores for a single player across a set of categories.
///
/// Writes z-scores into `zscores` at the appropriate registry indices.
/// Returns the weighted total z-score for these categories.
pub(crate) fn compute_player_category_zscores(
    projection: &stats::ProjectionData,
    pool_stats: &[PoolStats],
    league_avgs: &HashMap<usize, f64>,
    category_indices: &[usize],
    registry: &StatRegistry,
    weight_values: &CategoryValues,
    zscores: &mut CategoryValues,
) -> f64 {
    let all_stats = registry.all_stats();
    let mut total = 0.0;

    for &cat_idx in category_indices {
        let def = &all_stats[cat_idx];
        let value = match &def.computation {
            StatComputation::Counting { projection_key } => {
                projection.get_or_zero(projection_key)
            }
            StatComputation::RateStat { volume_key, rate_key, divisor } => {
                let league_avg = league_avgs.get(&cat_idx).copied().unwrap_or(0.0);
                stats::rate_stat_contribution(
                    projection.get_or_zero(volume_key),
                    projection.get_or_zero(rate_key),
                    league_avg,
                    *divisor,
                    def.sort_direction,
                )
            }
        };
        let z = compute_zscore(value, &pool_stats[cat_idx]);
        zscores.set(cat_idx, z);
        total += z * weight_values.get(cat_idx).unwrap_or(0.0);
    }

    total
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
    registry: &StatRegistry,
    weight_values: &CategoryValues,
) -> Vec<PlayerValuation> {
    let pool_cfg = &config.strategy.pool;

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

    // ---- 2+3. Pool stats via generic loop ----
    let hitter_pool_data: Vec<stats::ProjectionData> = hitter_pool
        .iter()
        .map(|h| stats::ProjectionData::from(*h))
        .collect();
    let pitcher_pool_data: Vec<stats::ProjectionData> = pitcher_pool
        .iter()
        .map(|p| stats::ProjectionData::from(*p))
        .collect();

    let (hitter_stats, hitter_league_avgs) = compute_generic_pool_stats(
        &hitter_pool_data, registry.batting_indices(), registry,
    );
    let (pitcher_stats, pitcher_league_avgs) = compute_generic_pool_stats(
        &pitcher_pool_data, registry.pitching_indices(), registry,
    );

    // ---- 4+5. Score all players ----
    let mut valuations: Vec<PlayerValuation> = Vec::with_capacity(
        projections.hitters.len() + projections.pitchers.len(),
    );

    // Build a set of pitcher names for two-way player detection.
    let pitcher_name_set: std::collections::HashSet<&str> = projections
        .pitchers
        .iter()
        .map(|p| p.name.as_str())
        .collect();

    // Track which pitcher names were matched as two-way players so we can
    // skip their standalone pitcher entry later.
    let mut two_way_pitcher_names: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for hitter in &projections.hitters {
        if let Some(matching_pitcher) = pitcher_name_set
            .contains(hitter.name.as_str())
            .then(|| {
                projections
                    .pitchers
                    .iter()
                    .find(|p| p.name == hitter.name)
            })
            .flatten()
        {
            // Two-way player: compute both hitting and pitching z-scores.
            let hitter_proj = stats::ProjectionData::from(hitter);
            let pitcher_proj = stats::ProjectionData::from(matching_pitcher);

            let mut two_way_zscores = CategoryValues::zeros(registry.len());
            let batting_total = compute_player_category_zscores(
                &hitter_proj, &hitter_stats, &hitter_league_avgs,
                registry.batting_indices(), registry, weight_values,
                &mut two_way_zscores,
            );
            let pitching_total = compute_player_category_zscores(
                &pitcher_proj, &pitcher_stats, &pitcher_league_avgs,
                registry.pitching_indices(), registry, weight_values,
                &mut two_way_zscores,
            );
            let combined_total = batting_total + pitching_total;

            // Pitcher position for the positions list (they can fill hitter
            // slots AND contribute pitching stats).
            let pitcher_pos = match matching_pitcher.pitcher_type {
                PitcherType::SP => Position::StartingPitcher,
                PitcherType::RP => Position::ReliefPitcher,
            };

            // Start with pitcher position; add hitter position from CSV if available.
            // Live ESPN eligible_slots will override these at runtime.
            let mut two_way_positions = vec![pitcher_pos];
            if !hitter.espn_position.is_empty() {
                for token in hitter.espn_position.split('/') {
                    let t = token.trim();
                    if t.eq_ignore_ascii_case("OF") {
                        for of_pos in [Position::LeftField, Position::CenterField, Position::RightField] {
                            if !of_pos.is_meta_slot() && !two_way_positions.contains(&of_pos) {
                                two_way_positions.push(of_pos);
                            }
                        }
                    } else if let Some(pos) = Position::from_str_pos(t) {
                        if !pos.is_meta_slot() && !two_way_positions.contains(&pos) {
                            two_way_positions.push(pos);
                        }
                    }
                }
            }

            two_way_pitcher_names.insert(hitter.name.clone());

            valuations.push(PlayerValuation {
                name: hitter.name.clone(),
                team: hitter.team.clone(),
                positions: two_way_positions,
                is_pitcher: false, // Fills a hitter slot for roster purposes
                is_two_way: true,
                pitcher_type: Some(matching_pitcher.pitcher_type),
                projection: {
                    let mut proj = ProjectionData::from(hitter);
                    proj.merge(&ProjectionData::from(matching_pitcher));
                    proj
                },
                total_zscore: combined_total,
                category_zscores: CategoryZScores::two_way(two_way_zscores, batting_total, pitching_total),
                vor: 0.0,
                initial_vor: 0.0,
                best_position: None,
                dollar_value: 0.0,
            });
        } else {
            // Normal hitter (not a two-way player).
            let hitter_proj = stats::ProjectionData::from(hitter);
            let mut zscores = CategoryValues::zeros(registry.len());
            let total = compute_player_category_zscores(
                &hitter_proj, &hitter_stats, &hitter_league_avgs,
                registry.batting_indices(), registry, weight_values,
                &mut zscores,
            );

            // Parse position from CSV projection data as a fallback;
            // may be overridden by live ESPN eligible_slots during draft.
            let positions: Vec<Position> = if !hitter.espn_position.is_empty() {
                let mut pos: Vec<Position> = Vec::new();
                for token in hitter.espn_position.split('/') {
                    let t = token.trim();
                    if t.eq_ignore_ascii_case("OF") {
                        pos.push(Position::LeftField);
                        pos.push(Position::CenterField);
                        pos.push(Position::RightField);
                    } else if let Some(p) = Position::from_str_pos(t) {
                        if !p.is_meta_slot() {
                            pos.push(p);
                        }
                    }
                }
                pos.sort();
                pos.dedup();
                pos
            } else {
                Vec::new()
            };

            valuations.push(PlayerValuation {
                name: hitter.name.clone(),
                team: hitter.team.clone(),
                positions,
                is_pitcher: false,
                is_two_way: false,
                pitcher_type: None,
                projection: ProjectionData::from(hitter),
                total_zscore: total,
                category_zscores: CategoryZScores::hitter(zscores, total),
                vor: 0.0,
                initial_vor: 0.0,
                best_position: None,
                dollar_value: 0.0,
            });
        }
    }

    for pitcher in &projections.pitchers {
        // Skip pitchers that were already merged into a two-way player entry.
        if two_way_pitcher_names.contains(&pitcher.name) {
            continue;
        }

        let pitcher_proj = stats::ProjectionData::from(pitcher);
        let mut zscores = CategoryValues::zeros(registry.len());
        let total = compute_player_category_zscores(
            &pitcher_proj, &pitcher_stats, &pitcher_league_avgs,
            registry.pitching_indices(), registry, weight_values,
            &mut zscores,
        );

        let pos = match pitcher.pitcher_type {
            PitcherType::SP => Position::StartingPitcher,
            PitcherType::RP => Position::ReliefPitcher,
        };
        valuations.push(PlayerValuation {
            name: pitcher.name.clone(),
            team: pitcher.team.clone(),
            positions: vec![pos],
            is_pitcher: true,
            is_two_way: false,
            pitcher_type: Some(pitcher.pitcher_type),
            projection: ProjectionData::from(pitcher),
            total_zscore: total,
            category_zscores: CategoryZScores::pitcher(zscores, total),
            vor: 0.0,
            initial_vor: 0.0,
            best_position: None,
            dollar_value: 0.0,
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
    use crate::stats::StatRegistry;
    use crate::valuation::projections::*;

    fn test_registry(config: &Config) -> StatRegistry {
        StatRegistry::from_league_config(&config.league).unwrap()
    }

    fn test_registry_and_weights(config: &Config) -> (StatRegistry, CategoryValues) {
        let registry = StatRegistry::from_league_config(&config.league).unwrap();
        let weights = weights_to_category_values(&config.strategy.weights, &registry);
        (registry, weights)
    }

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
                roster_limits: RosterLimits {
                    max_sp: 7,
                    max_rp: 7,
                    gs_per_week: 7,
                },
                teams: std::collections::HashMap::new(),
            },
            strategy: StrategyConfig {
                hitting_budget_fraction: 0.65,
                weights: CategoryWeights::from_pairs([
                    ("R", 1.0), ("HR", 1.0), ("RBI", 1.0), ("BB", 1.0),
                    ("SB", 1.0), ("AVG", 1.0), ("K", 1.0), ("W", 1.0),
                    ("SV", 0.7), ("HD", 1.0), ("ERA", 1.0), ("WHIP", 1.0),
                ]),
                strategy_overview: None,
                pool: PoolConfig {
                    min_pa: 200,
                    min_ip_sp: 50.0,
                    min_g_rp: 20,
                    hitter_pool_size: 150,
                    sp_pool_size: 70,
                    rp_pool_size: 80,
                },
                llm: LlmConfig {
                    provider: crate::llm::provider::LlmProvider::Anthropic,
                    model: "test".into(),
                    analysis_max_tokens: 2048,
                    planning_max_tokens: 2048,
                    analysis_trigger: "nomination".into(),
                    prefire_planning: true,
                },
            },
            credentials: CredentialsConfig::default(),
            ws_port: 9001,
            data_paths: DataPaths::default(),
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
            espn_position: String::new(),
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

    // Rate stat contribution tests are in stats::tests (rate_stat_contribution_*)

    // ---- Category weights test ----

    #[test]
    fn category_weights_applied_correctly() {
        let config = test_config();
        let registry = test_registry(&config);

        // Create pool stats as a Vec<PoolStats> indexed by registry position.
        let zero = PoolStats { mean: 0.0, stdev: 0.0 };
        let mut pool_stats = vec![zero; registry.len()];
        pool_stats[registry.index_of("K").unwrap()] = PoolStats { mean: 100.0, stdev: 30.0 };
        pool_stats[registry.index_of("W").unwrap()] = PoolStats { mean: 8.0, stdev: 3.0 };
        pool_stats[registry.index_of("SV").unwrap()] = PoolStats { mean: 10.0, stdev: 10.0 };
        pool_stats[registry.index_of("HD").unwrap()] = PoolStats { mean: 5.0, stdev: 5.0 };
        pool_stats[registry.index_of("ERA").unwrap()] = PoolStats { mean: 0.0, stdev: 10.0 };
        pool_stats[registry.index_of("WHIP").unwrap()] = PoolStats { mean: 0.0, stdev: 10.0 };

        // Create a closer with big SV numbers via ProjectionData
        let mut closer_proj = stats::ProjectionData::new();
        closer_proj.insert("ip", 60.0);
        closer_proj.insert("k", 100.0);
        closer_proj.insert("w", 8.0);
        closer_proj.insert("sv", 40.0);
        closer_proj.insert("hd", 5.0);
        closer_proj.insert("era", 3.00);
        closer_proj.insert("whip", 1.10);

        // League avgs for rate stats (ERA/WHIP pool_stats have mean=0 so
        // the exact league_avg doesn't change the z-score; we still provide
        // plausible values for correctness).
        let mut league_avgs = HashMap::new();
        league_avgs.insert(registry.index_of("ERA").unwrap(), 4.00);
        league_avgs.insert(registry.index_of("WHIP").unwrap(), 1.30);

        let wv_equal = weights_to_category_values(
            &CategoryWeights::from_pairs([
                ("R", 1.0), ("HR", 1.0), ("RBI", 1.0), ("BB", 1.0), ("SB", 1.0), ("AVG", 1.0),
                ("K", 1.0), ("W", 1.0), ("SV", 1.0), ("HD", 1.0), ("ERA", 1.0), ("WHIP", 1.0),
            ]),
            &registry,
        );
        let wv_reduced = weights_to_category_values(
            &CategoryWeights::from_pairs([
                ("R", 1.0), ("HR", 1.0), ("RBI", 1.0), ("BB", 1.0), ("SB", 1.0), ("AVG", 1.0),
                ("K", 1.0), ("W", 1.0), ("SV", 0.7), ("HD", 1.0), ("ERA", 1.0), ("WHIP", 1.0),
            ]),
            &registry,
        );

        let mut zscores_eq = CategoryValues::zeros(registry.len());
        let total_eq = compute_player_category_zscores(
            &closer_proj, &pool_stats, &league_avgs,
            registry.pitching_indices(), &registry, &wv_equal,
            &mut zscores_eq,
        );

        let mut zscores_red = CategoryValues::zeros(registry.len());
        let total_red = compute_player_category_zscores(
            &closer_proj, &pool_stats, &league_avgs,
            registry.pitching_indices(), &registry, &wv_reduced,
            &mut zscores_red,
        );

        // SV z-score for this closer: (40 - 10) / 10 = 3.0
        assert!(approx_eq(zscores_eq.get(registry.index_of("SV").unwrap()).unwrap(), 3.0, 1e-10));
        assert!(approx_eq(zscores_red.get(registry.index_of("SV").unwrap()).unwrap(), 3.0, 1e-10));

        // Total should differ: equal has SV*1.0=3.0, reduced has SV*0.7=2.1
        // Difference = 3.0 - 2.1 = 0.9
        let diff = total_eq - total_red;
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

        let projections = AllProjections {
            hitters,
            pitchers,
        };

        // Config with pools small enough to include all players
        let mut config = test_config();
        config.strategy.pool.min_pa = 200;
        config.strategy.pool.hitter_pool_size = 5;
        config.strategy.pool.min_ip_sp = 50.0;
        config.strategy.pool.sp_pool_size = 5;
        config.strategy.pool.min_g_rp = 20;
        config.strategy.pool.rp_pool_size = 5;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        // Should have all 10 players
        assert_eq!(valuations.len(), 10);

        // First player should have highest z-score
        assert!(valuations[0].total_zscore >= valuations[1].total_zscore);

        // All z-scores should be finite
        for v in &valuations {
            assert!(v.total_zscore.is_finite(), "Player {} has non-finite z-score", v.name);
        }

        let registry = test_registry(&config);

        // Elite hitter should have positive z-scores in counting stats
        let elite = valuations.iter().find(|v| v.name == "Elite").unwrap();
        match &elite.category_zscores {
            CategoryZScores::Hitter { zscores, .. } => {
                assert!(zscores.get(registry.index_of("R").unwrap()).unwrap() > 0.0, "Elite R z-score should be positive");
                assert!(zscores.get(registry.index_of("HR").unwrap()).unwrap() > 0.0, "Elite HR z-score should be positive");
                assert!(zscores.get(registry.index_of("RBI").unwrap()).unwrap() > 0.0, "Elite RBI z-score should be positive");
                assert!(zscores.get(registry.index_of("BB").unwrap()).unwrap() > 0.0, "Elite BB z-score should be positive");
                assert!(zscores.get(registry.index_of("SB").unwrap()).unwrap() > 0.0, "Elite SB z-score should be positive");
            }
            _ => panic!("Elite should be a hitter"),
        }

        // Replacement-level hitter should have negative z-scores
        let replacement = valuations.iter().find(|v| v.name == "Replacement").unwrap();
        match &replacement.category_zscores {
            CategoryZScores::Hitter { zscores, .. } => {
                assert!(zscores.get(registry.index_of("R").unwrap()).unwrap() < 0.0, "Replacement R z-score should be negative");
                assert!(zscores.get(registry.index_of("HR").unwrap()).unwrap() < 0.0, "Replacement HR z-score should be negative");
            }
            _ => panic!("Replacement should be a hitter"),
        }

        // Ace should have positive ERA contribution z-score (below-avg ERA = good)
        let ace = valuations.iter().find(|v| v.name == "Ace").unwrap();
        match &ace.category_zscores {
            CategoryZScores::Pitcher { zscores, .. } => {
                assert!(zscores.get(registry.index_of("ERA").unwrap()).unwrap() > 0.0, "Ace ERA z-score should be positive (good ERA)");
                assert!(zscores.get(registry.index_of("WHIP").unwrap()).unwrap() > 0.0, "Ace WHIP z-score should be positive (good WHIP)");
                assert!(zscores.get(registry.index_of("K").unwrap()).unwrap() > 0.0, "Ace K z-score should be positive");
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
                espn_position: String::new(),
            })
            .collect();

        let pitchers = vec![
            make_sp("SP1", 180.0, 190, 14, 3.30, 1.10),
        ];

        let projections = AllProjections {
            hitters,
            pitchers,
        };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 10;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 10;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

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

    // Volume-weighting and pool stats tests are covered by stats::tests and
    // the generic pool stats function tested via the snapshot/integration tests.

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
        };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 10;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 10;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

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
        };

        let mut config = test_config();
        config.strategy.pool.min_pa = 200;
        config.strategy.pool.hitter_pool_size = 10;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 10;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

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
        };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 10;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 10;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        let hitter = valuations.iter().find(|v| v.name == "TestHitter").unwrap();
        let hp = &hitter.projection;
        assert_eq!(hp.get("pa") as u32, 600);
        assert_eq!(hp.get("ab") as u32, 540);
        assert_eq!(hp.get("hr") as u32, 30);
        assert_eq!(hp.get("r") as u32, 90);
        assert_eq!(hp.get("rbi") as u32, 85);
        assert_eq!(hp.get("bb") as u32, 55);
        assert_eq!(hp.get("sb") as u32, 12);
        assert!(approx_eq(hp.get("avg"), 160.0 / 540.0, 1e-10));

        let pitcher = valuations.iter().find(|v| v.name == "TestPitcher").unwrap();
        let pp = &pitcher.projection;
        assert!(approx_eq(pp.get("ip"), 180.0, 1e-10));
        assert_eq!(pp.get("k") as u32, 190);
        assert_eq!(pp.get("w") as u32, 14);
        assert!(approx_eq(pp.get("era"), 3.30, 1e-10));
        assert!(approx_eq(pp.get("whip"), 1.10, 1e-10));
    }

    // ---- Two-way player detection and valuation tests ----

    #[test]
    fn two_way_player_detected_when_name_matches() {
        // A player appearing in both hitters and pitchers CSVs should be
        // detected as a two-way player.
        let hitters = vec![
            make_hitter("Shohei Ohtani", 600, 540, 162, 40, 100, 95, 55, 15),
            make_hitter("Regular Hitter", 550, 500, 140, 25, 80, 75, 45, 10),
        ];

        let pitchers = vec![
            PitcherProjection {
                name: "Shohei Ohtani".into(),
                team: "LAD".into(),
                pitcher_type: PitcherType::SP,
                ip: 160.0,
                k: 200,
                w: 14,
                sv: 0,
                hd: 0,
                era: 2.80,
                whip: 1.00,
                g: 28,
                gs: 28,
            },
            make_sp("Regular SP", 180.0, 190, 14, 3.30, 1.10),
        ];

        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 150;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 70;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        // Ohtani should appear exactly once (not as separate hitter + pitcher).
        let ohtani_count = valuations.iter().filter(|v| v.name == "Shohei Ohtani").count();
        assert_eq!(ohtani_count, 1, "Two-way player should appear exactly once");

        let ohtani = valuations.iter().find(|v| v.name == "Shohei Ohtani").unwrap();
        assert!(ohtani.is_two_way, "Ohtani should be marked as two-way");
        assert!(!ohtani.is_pitcher, "Two-way player fills a hitter slot (is_pitcher = false)");
        assert_eq!(ohtani.pitcher_type, Some(PitcherType::SP));

        // Should have merged projection data (both hitting and pitching keys).
        let op = &ohtani.projection;
        assert_eq!(op.get("pa") as u32, 600);
        assert_eq!(op.get("hr") as u32, 40);
        assert!(approx_eq(op.get("ip"), 160.0, 1e-10));
        assert_eq!(op.get("k") as u32, 200);

        // Should have TwoWay z-scores.
        match &ohtani.category_zscores {
            CategoryZScores::TwoWay { batting_total, pitching_total, total, .. } => {
                assert!(batting_total.is_finite());
                assert!(pitching_total.is_finite());
                assert!(approx_eq(*total, batting_total + pitching_total, 1e-10));
            }
            other => panic!("Expected TwoWay z-scores, got {:?}", other),
        }

        // The other players should NOT be two-way.
        let regular_hitter = valuations.iter().find(|v| v.name == "Regular Hitter").unwrap();
        assert!(!regular_hitter.is_two_way);

        let regular_sp = valuations.iter().find(|v| v.name == "Regular SP").unwrap();
        assert!(!regular_sp.is_two_way);
        assert!(regular_sp.is_pitcher);
    }

    #[test]
    fn two_way_player_combined_zscore_higher_than_either_side() {
        // A genuinely good two-way player's combined z-score should exceed
        // their hitting-only or pitching-only z-score.
        let hitters = vec![
            make_hitter("TwoWay Star", 600, 540, 162, 35, 95, 90, 55, 12),
            make_hitter("Filler H1", 550, 500, 135, 20, 70, 65, 45, 8),
            make_hitter("Filler H2", 520, 480, 125, 15, 60, 55, 35, 5),
        ];

        let pitchers = vec![
            PitcherProjection {
                name: "TwoWay Star".into(),
                team: "TST".into(),
                pitcher_type: PitcherType::SP,
                ip: 150.0,
                k: 180,
                w: 12,
                sv: 0,
                hd: 0,
                era: 3.00,
                whip: 1.05,
                g: 26,
                gs: 26,
            },
            make_sp("Filler SP1", 180.0, 190, 14, 3.30, 1.10),
            make_sp("Filler SP2", 160.0, 150, 10, 3.80, 1.20),
        ];

        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 150;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 70;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);
        let two_way = valuations.iter().find(|v| v.name == "TwoWay Star").unwrap();

        // Extract hitting and pitching sub-scores.
        let (hitting_total, pitching_total) = match &two_way.category_zscores {
            CategoryZScores::TwoWay { batting_total, pitching_total, .. } => (*batting_total, *pitching_total),
            other => panic!("Expected TwoWay z-scores, got {:?}", other),
        };

        // Combined should be the sum.
        assert!(
            approx_eq(two_way.total_zscore, hitting_total + pitching_total, 1e-10),
            "Combined z-score ({}) should equal hitting ({}) + pitching ({})",
            two_way.total_zscore,
            hitting_total,
            pitching_total,
        );

        // For a good two-way player, the combined should exceed either side alone.
        assert!(
            two_way.total_zscore > hitting_total,
            "Combined ({}) should exceed hitting alone ({})",
            two_way.total_zscore,
            hitting_total,
        );
        assert!(
            two_way.total_zscore > pitching_total,
            "Combined ({}) should exceed pitching alone ({})",
            two_way.total_zscore,
            pitching_total,
        );
    }

    #[test]
    fn two_way_player_bad_pitching_still_reasonable() {
        // A player who appears in both CSVs but has terrible pitching stats.
        // Their combined value might be lower than pure hitting if pitching
        // z-scores are very negative, but the system should handle it gracefully.
        let hitters = vec![
            make_hitter("Bad Pitcher Hitter", 600, 540, 162, 35, 95, 90, 55, 12),
            make_hitter("Filler H1", 550, 500, 135, 20, 70, 65, 45, 8),
            make_hitter("Filler H2", 520, 480, 125, 15, 60, 55, 35, 5),
        ];

        let pitchers = vec![
            PitcherProjection {
                name: "Bad Pitcher Hitter".into(),
                team: "TST".into(),
                pitcher_type: PitcherType::SP,
                ip: 20.0, // Very few innings
                k: 15,
                w: 1,
                sv: 0,
                hd: 0,
                era: 6.50, // Terrible ERA
                whip: 1.80, // Terrible WHIP
                g: 5,
                gs: 5,
            },
            make_sp("Filler SP1", 180.0, 190, 14, 3.30, 1.10),
            make_sp("Filler SP2", 160.0, 150, 10, 3.80, 1.20),
        ];

        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 150;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 70;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);
        let player = valuations.iter().find(|v| v.name == "Bad Pitcher Hitter").unwrap();

        // Should still be detected as two-way.
        assert!(player.is_two_way);

        // Z-score should be finite (not NaN or Infinity).
        assert!(player.total_zscore.is_finite());

        // Pitching z-scores should be negative (bad pitcher).
        match &player.category_zscores {
            CategoryZScores::TwoWay { batting_total, pitching_total, .. } => {
                assert!(
                    *pitching_total < 0.0,
                    "Bad pitcher should have negative pitching z-score, got {}",
                    pitching_total,
                );
                // But hitting should be positive (good hitter).
                assert!(
                    *batting_total > 0.0,
                    "Good hitter should have positive hitting z-score, got {}",
                    batting_total,
                );
            }
            other => panic!("Expected TwoWay z-scores, got {:?}", other),
        }
    }

    #[test]
    fn total_player_count_correct_with_two_way() {
        // With 3 hitters (1 two-way) and 3 pitchers (1 is the two-way match),
        // we should get 3 + 2 = 5 total valuations (not 3 + 3 = 6).
        let hitters = vec![
            make_hitter("TwoWay Player", 600, 540, 162, 35, 95, 90, 55, 12),
            make_hitter("Pure Hitter A", 550, 500, 135, 20, 70, 65, 45, 8),
            make_hitter("Pure Hitter B", 520, 480, 125, 15, 60, 55, 35, 5),
        ];

        let pitchers = vec![
            PitcherProjection {
                name: "TwoWay Player".into(),
                team: "TST".into(),
                pitcher_type: PitcherType::SP,
                ip: 150.0,
                k: 180,
                w: 12,
                sv: 0,
                hd: 0,
                era: 3.00,
                whip: 1.05,
                g: 26,
                gs: 26,
            },
            make_sp("Pure SP", 180.0, 190, 14, 3.30, 1.10),
            make_rp("Pure RP", 60.0, 70, 30, 0, 2.50, 0.95, 55),
        ];

        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 150;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 70;
        config.strategy.pool.min_g_rp = 10;
        config.strategy.pool.rp_pool_size = 80;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);
        assert_eq!(
            valuations.len(),
            5,
            "3 hitters (1 two-way merged) + 2 pure pitchers = 5 total, got {}",
            valuations.len()
        );
    }

    // ---- CSV ESPN position populates PlayerValuation.positions ----

    #[test]
    fn hitter_with_espn_position_has_populated_positions() {
        let mut hitter = make_hitter("Bobby Witt Jr.", 652, 590, 171, 27, 96, 87, 49, 32);
        hitter.espn_position = "SS".to_string();

        let hitters = vec![
            hitter,
            make_hitter("Some Other", 600, 540, 150, 25, 80, 75, 50, 10),
        ];
        let pitchers = vec![make_sp("SP1", 180.0, 190, 14, 3.30, 1.10)];
        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 200;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 200;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        let witt = valuations.iter().find(|v| v.name == "Bobby Witt Jr.").unwrap();
        assert_eq!(witt.positions, vec![Position::ShortStop]);

        let other = valuations.iter().find(|v| v.name == "Some Other").unwrap();
        assert!(other.positions.is_empty(), "Hitter without ESPN position should have empty positions");
    }

    #[test]
    fn hitter_with_of_position_expands_to_all_outfield() {
        let mut hitter = make_hitter("Juan Soto", 700, 600, 180, 40, 110, 100, 120, 5);
        hitter.espn_position = "OF".to_string();

        let hitters = vec![hitter];
        let pitchers = vec![make_sp("SP1", 180.0, 190, 14, 3.30, 1.10)];
        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 200;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 200;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        let soto = valuations.iter().find(|v| v.name == "Juan Soto").unwrap();
        // "OF" expands to LF, CF, RF to match ESPN slot behavior
        assert!(soto.positions.contains(&Position::LeftField));
        assert!(soto.positions.contains(&Position::CenterField));
        assert!(soto.positions.contains(&Position::RightField));
        assert_eq!(soto.positions.len(), 3);
    }

    #[test]
    fn hitter_with_dh_position_is_not_meta_slot() {
        let mut hitter = make_hitter("Shohei Ohtani", 700, 600, 180, 50, 120, 130, 80, 10);
        hitter.espn_position = "DH".to_string();

        let hitters = vec![hitter];
        let pitchers = vec![make_sp("SP1", 180.0, 190, 14, 3.30, 1.10)];
        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 200;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 200;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        let ohtani = valuations.iter().find(|v| v.name == "Shohei Ohtani").unwrap();
        // DH is not a meta slot, so it should be populated
        assert_eq!(ohtani.positions, vec![Position::DesignatedHitter]);
    }

    #[test]
    fn hitter_with_util_position_filtered_as_meta_slot() {
        let mut hitter = make_hitter("Test UTIL", 600, 540, 150, 25, 80, 75, 50, 10);
        hitter.espn_position = "UTIL".to_string();

        let hitters = vec![hitter];
        let pitchers = vec![make_sp("SP1", 180.0, 190, 14, 3.30, 1.10)];
        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 200;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 200;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        let util_player = valuations.iter().find(|v| v.name == "Test UTIL").unwrap();
        // UTIL is a meta slot, so positions should be empty (filtered out)
        assert!(util_player.positions.is_empty());
    }

    #[test]
    fn hitter_with_multi_position_string() {
        let mut hitter = make_hitter("Wander Franco", 600, 540, 160, 20, 80, 70, 50, 15);
        hitter.espn_position = "1B/3B".to_string();

        let hitters = vec![hitter];
        let pitchers = vec![make_sp("SP1", 180.0, 190, 14, 3.30, 1.10)];
        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 200;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 200;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        let player = valuations.iter().find(|v| v.name == "Wander Franco").unwrap();
        assert!(player.positions.contains(&Position::FirstBase));
        assert!(player.positions.contains(&Position::ThirdBase));
        assert_eq!(player.positions.len(), 2);
    }

    #[test]
    fn hitter_with_multi_position_and_dh() {
        let mut hitter = make_hitter("Yordan Alvarez", 650, 580, 170, 35, 95, 100, 60, 2);
        hitter.espn_position = "LF/DH".to_string();

        let hitters = vec![hitter];
        let pitchers = vec![make_sp("SP1", 180.0, 190, 14, 3.30, 1.10)];
        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 200;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 200;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        let player = valuations.iter().find(|v| v.name == "Yordan Alvarez").unwrap();
        assert!(player.positions.contains(&Position::LeftField));
        assert!(player.positions.contains(&Position::DesignatedHitter));
        assert_eq!(player.positions.len(), 2);
    }

    #[test]
    fn hitter_with_multi_outfield_positions_deduplicates() {
        let mut hitter = make_hitter("Mike Trout", 600, 540, 160, 35, 100, 90, 70, 8);
        hitter.espn_position = "LF/CF/RF".to_string();

        let hitters = vec![hitter];
        let pitchers = vec![make_sp("SP1", 180.0, 190, 14, 3.30, 1.10)];
        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 100;
        config.strategy.pool.hitter_pool_size = 200;
        config.strategy.pool.min_ip_sp = 10.0;
        config.strategy.pool.sp_pool_size = 200;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        let player = valuations.iter().find(|v| v.name == "Mike Trout").unwrap();
        assert!(player.positions.contains(&Position::LeftField));
        assert!(player.positions.contains(&Position::CenterField));
        assert!(player.positions.contains(&Position::RightField));
        assert_eq!(player.positions.len(), 3);
    }

    // ---- Snapshot test: captures exact numerical output for regression ----

    /// Golden values captured from the bespoke implementation before the
    /// registry-driven rewrite. Any drift beyond 1e-12 indicates a bug.
    #[test]
    fn snapshot_exact_zscores() {
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

        let projections = AllProjections { hitters, pitchers };

        let mut config = test_config();
        config.strategy.pool.min_pa = 200;
        config.strategy.pool.hitter_pool_size = 5;
        config.strategy.pool.min_ip_sp = 50.0;
        config.strategy.pool.sp_pool_size = 5;
        config.strategy.pool.min_g_rp = 20;
        config.strategy.pool.rp_pool_size = 5;

        let (registry, weight_values) = test_registry_and_weights(&config);
        let valuations = compute_initial_zscores(&projections, &config, &registry, &weight_values);

        // Golden total z-scores (descending order)
        let expected: &[(&str, f64)] = &[
            ("Elite",         9.84290718552074750e0),
            ("Ace",           4.54079930434119561e0),
            ("Good",          3.12476270097996878e0),
            ("Elite Closer",  5.35738541186727923e-1),
            ("Solid SP",     -4.01675142375689131e-1),
            ("Setup Man",    -4.96648650567435235e-1),
            ("Average",      -1.24367912250344737e0),
            ("Average SP",   -4.17821405258479928e0),
            ("Below",        -4.79915242226655536e0),
            ("Replacement",  -6.92483834173071333e0),
        ];

        assert_eq!(valuations.len(), expected.len());
        for (v, &(name, exp_total)) in valuations.iter().zip(expected.iter()) {
            assert_eq!(v.name, name, "ordering mismatch");
            assert!(
                (v.total_zscore - exp_total).abs() < 1e-12,
                "{}: expected {}, got {}",
                name, exp_total, v.total_zscore,
            );
        }
    }
}
