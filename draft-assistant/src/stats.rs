// Stats module: generic stat definitions, registry, and helper types.
//
// Provides a data-driven registry of statistical categories that replaces
// hard-coded per-stat logic throughout the valuation engine.

use std::collections::HashMap;

use crate::config::LeagueConfig;

// ---------------------------------------------------------------------------
// Core enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerType {
    Hitter,
    Pitcher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    HigherIsBetter,
    LowerIsBetter,
}

#[derive(Debug, Clone)]
pub enum StatComputation {
    /// Value extracted directly from projections.
    Counting { projection_key: String },
    /// Volume-weighted contribution for z-scoring rate stats.
    RateStat {
        volume_key: String,
        rate_key: String,
        divisor: f64,
    },
}

// ---------------------------------------------------------------------------
// StatDefinition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StatDefinition {
    pub abbrev: String,
    pub display_name: String,
    pub espn_stat_id: Option<u16>,
    pub player_type: PlayerType,
    pub sort_direction: SortDirection,
    pub format_precision: u8,
    pub close_threshold: f64,
    pub matchup_close_threshold: f64,
    pub computation: StatComputation,
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum StatsError {
    #[error("unknown stat category: {abbrev}")]
    UnknownStat { abbrev: String },
    #[error("duplicate stat category: {abbrev}")]
    DuplicateStat { abbrev: String },
}

// ---------------------------------------------------------------------------
// StatRegistry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StatRegistry {
    stats: Vec<StatDefinition>,
    index: HashMap<String, usize>,
    batting_indices: Vec<usize>,
    pitching_indices: Vec<usize>,
}

impl StatRegistry {
    /// Build a registry from a LeagueConfig, looking up each category
    /// abbreviation in the built-in knowledge base.
    pub fn from_league_config(config: &LeagueConfig) -> Result<Self, StatsError> {
        let mut stats = Vec::new();
        let mut index = HashMap::new();
        let mut batting_indices = Vec::new();
        let mut pitching_indices = Vec::new();

        for abbrev in &config.batting_categories.categories {
            if index.contains_key(abbrev.as_str()) {
                return Err(StatsError::DuplicateStat {
                    abbrev: abbrev.clone(),
                });
            }
            let def = lookup_stat_definition(abbrev, PlayerType::Hitter).ok_or_else(|| {
                StatsError::UnknownStat {
                    abbrev: abbrev.clone(),
                }
            })?;
            let idx = stats.len();
            index.insert(abbrev.clone(), idx);
            batting_indices.push(idx);
            stats.push(def);
        }

        for abbrev in &config.pitching_categories.categories {
            if index.contains_key(abbrev.as_str()) {
                return Err(StatsError::DuplicateStat {
                    abbrev: abbrev.clone(),
                });
            }
            let def = lookup_stat_definition(abbrev, PlayerType::Pitcher).ok_or_else(|| {
                StatsError::UnknownStat {
                    abbrev: abbrev.clone(),
                }
            })?;
            let idx = stats.len();
            index.insert(abbrev.clone(), idx);
            pitching_indices.push(idx);
            stats.push(def);
        }

        Ok(Self {
            stats,
            index,
            batting_indices,
            pitching_indices,
        })
    }

    pub fn len(&self) -> usize {
        self.stats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stats.is_empty()
    }

    pub fn get(&self, abbrev: &str) -> Option<&StatDefinition> {
        self.index.get(abbrev).map(|&i| &self.stats[i])
    }

    pub fn index_of(&self, abbrev: &str) -> Option<usize> {
        self.index.get(abbrev).copied()
    }

    pub fn all_stats(&self) -> &[StatDefinition] {
        &self.stats
    }

    pub fn batting_stats(&self) -> impl Iterator<Item = &StatDefinition> {
        self.batting_indices.iter().map(|&i| &self.stats[i])
    }

    pub fn pitching_stats(&self) -> impl Iterator<Item = &StatDefinition> {
        self.pitching_indices.iter().map(|&i| &self.stats[i])
    }

    pub fn batting_stats_indexed(&self) -> impl Iterator<Item = (usize, &StatDefinition)> {
        self.batting_indices.iter().map(|&i| (i, &self.stats[i]))
    }

    pub fn pitching_stats_indexed(&self) -> impl Iterator<Item = (usize, &StatDefinition)> {
        self.pitching_indices.iter().map(|&i| (i, &self.stats[i]))
    }

    pub fn batting_indices(&self) -> &[usize] {
        &self.batting_indices
    }

    pub fn pitching_indices(&self) -> &[usize] {
        &self.pitching_indices
    }

    pub fn batting_count(&self) -> usize {
        self.batting_indices.len()
    }

    pub fn pitching_count(&self) -> usize {
        self.pitching_indices.len()
    }
}

// ---------------------------------------------------------------------------
// CategoryValues
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct CategoryValues {
    values: Vec<f64>,
}

impl CategoryValues {
    pub fn zeros(n: usize) -> Self {
        Self {
            values: vec![0.0; n],
        }
    }

    pub fn uniform(n: usize, value: f64) -> Self {
        Self {
            values: vec![value; n],
        }
    }

    pub fn from_vec(values: Vec<f64>) -> Self {
        Self { values }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn get(&self, idx: usize) -> Option<f64> {
        self.values.get(idx).copied()
    }

    pub fn set(&mut self, idx: usize, val: f64) {
        self.values[idx] = val;
    }

    pub fn as_slice(&self) -> &[f64] {
        &self.values
    }

    pub fn as_mut_slice(&mut self) -> &mut [f64] {
        &mut self.values
    }

    pub fn weighted_sum(&self, weights: &CategoryValues) -> f64 {
        debug_assert_eq!(
            self.values.len(),
            weights.values.len(),
            "CategoryValues length mismatch: {} vs {}",
            self.values.len(),
            weights.values.len()
        );
        self.values
            .iter()
            .zip(weights.values.iter())
            .map(|(v, w)| v * w)
            .sum()
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, f64)> + '_ {
        self.values.iter().copied().enumerate()
    }
}

// ---------------------------------------------------------------------------
// ProjectionData
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ProjectionData {
    data: HashMap<String, f64>,
}

impl ProjectionData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: impl Into<String>, value: f64) {
        self.data.insert(key.into(), value);
    }

    pub fn get(&self, key: &str) -> Option<f64> {
        self.data.get(key).copied()
    }

    pub fn get_or_zero(&self, key: &str) -> f64 {
        self.data.get(key).copied().unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------------------
// Rate stat contribution (generic)
// ---------------------------------------------------------------------------

/// Compute the volume-weighted contribution for a rate stat.
///
/// Generalizes the existing `era_contribution`, `whip_contribution`, and
/// `avg_contribution` functions in `zscore.rs`.
pub fn rate_stat_contribution(
    volume: f64,
    player_rate: f64,
    league_avg: f64,
    divisor: f64,
    direction: SortDirection,
) -> f64 {
    debug_assert!(divisor != 0.0, "rate_stat_contribution: divisor must not be zero");
    let diff = match direction {
        SortDirection::HigherIsBetter => player_rate - league_avg,
        SortDirection::LowerIsBetter => league_avg - player_rate,
    };
    volume * diff / divisor
}

// ---------------------------------------------------------------------------
// Stat knowledge base
// ---------------------------------------------------------------------------

/// Look up a stat definition from the built-in knowledge base.
///
/// Returns `None` for unknown abbreviations or player-type mismatches.
pub fn lookup_stat_definition(abbrev: &str, player_type: PlayerType) -> Option<StatDefinition> {
    let def = match (abbrev, player_type) {
        // Batting — counting stats
        ("R", PlayerType::Hitter) => StatDefinition {
            abbrev: "R".into(),
            display_name: "Runs".into(),
            espn_stat_id: Some(20),
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 2.0,
            matchup_close_threshold: 5.0,
            computation: StatComputation::Counting {
                projection_key: "r".into(),
            },
        },
        ("HR", PlayerType::Hitter) => StatDefinition {
            abbrev: "HR".into(),
            display_name: "Home Runs".into(),
            espn_stat_id: Some(5),
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "hr".into(),
            },
        },
        ("RBI", PlayerType::Hitter) => StatDefinition {
            abbrev: "RBI".into(),
            display_name: "Runs Batted In".into(),
            espn_stat_id: Some(21),
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 2.0,
            matchup_close_threshold: 5.0,
            computation: StatComputation::Counting {
                projection_key: "rbi".into(),
            },
        },
        ("BB", PlayerType::Hitter) => StatDefinition {
            abbrev: "BB".into(),
            display_name: "Walks".into(),
            espn_stat_id: Some(10),
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 2.0,
            matchup_close_threshold: 5.0,
            computation: StatComputation::Counting {
                projection_key: "bb".into(),
            },
        },
        ("SB", PlayerType::Hitter) => StatDefinition {
            abbrev: "SB".into(),
            display_name: "Stolen Bases".into(),
            espn_stat_id: Some(23),
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "sb".into(),
            },
        },
        // Batting — rate stat
        ("AVG", PlayerType::Hitter) => StatDefinition {
            abbrev: "AVG".into(),
            display_name: "Batting Average".into(),
            espn_stat_id: Some(2),
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 3,
            close_threshold: 0.005,
            matchup_close_threshold: 0.020,
            computation: StatComputation::RateStat {
                volume_key: "ab".into(),
                rate_key: "avg".into(),
                divisor: 1.0,
            },
        },
        // Pitching — counting stats
        ("K", PlayerType::Pitcher) => StatDefinition {
            abbrev: "K".into(),
            display_name: "Strikeouts".into(),
            espn_stat_id: Some(48),
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 3.0,
            matchup_close_threshold: 10.0,
            computation: StatComputation::Counting {
                projection_key: "k".into(),
            },
        },
        ("W", PlayerType::Pitcher) => StatDefinition {
            abbrev: "W".into(),
            display_name: "Wins".into(),
            espn_stat_id: Some(53),
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 2.0,
            computation: StatComputation::Counting {
                projection_key: "w".into(),
            },
        },
        ("SV", PlayerType::Pitcher) => StatDefinition {
            abbrev: "SV".into(),
            display_name: "Saves".into(),
            espn_stat_id: Some(57),
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 2.0,
            computation: StatComputation::Counting {
                projection_key: "sv".into(),
            },
        },
        ("HD", PlayerType::Pitcher) => StatDefinition {
            abbrev: "HD".into(),
            display_name: "Holds".into(),
            espn_stat_id: Some(60),
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 2.0,
            computation: StatComputation::Counting {
                projection_key: "hd".into(),
            },
        },
        // Pitching — rate stats
        ("ERA", PlayerType::Pitcher) => StatDefinition {
            abbrev: "ERA".into(),
            display_name: "Earned Run Average".into(),
            espn_stat_id: Some(47),
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::LowerIsBetter,
            format_precision: 2,
            close_threshold: 0.10,
            matchup_close_threshold: 1.00,
            computation: StatComputation::RateStat {
                volume_key: "ip".into(),
                rate_key: "era".into(),
                divisor: 9.0,
            },
        },
        ("WHIP", PlayerType::Pitcher) => StatDefinition {
            abbrev: "WHIP".into(),
            display_name: "Walks+Hits per IP".into(),
            espn_stat_id: Some(41),
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::LowerIsBetter,
            format_precision: 2,
            close_threshold: 0.05,
            matchup_close_threshold: 0.20,
            computation: StatComputation::RateStat {
                volume_key: "ip".into(),
                rate_key: "whip".into(),
                divisor: 1.0,
            },
        },
        // Batting — additional counting stats
        ("TB", PlayerType::Hitter) => StatDefinition {
            abbrev: "TB".into(),
            display_name: "Total Bases".into(),
            espn_stat_id: None,
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 3.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "tb".into(),
            },
        },
        ("XBH", PlayerType::Hitter) => StatDefinition {
            abbrev: "XBH".into(),
            display_name: "Extra Base Hits".into(),
            espn_stat_id: None,
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "xbh".into(),
            },
        },
        ("HBP", PlayerType::Hitter) => StatDefinition {
            abbrev: "HBP".into(),
            display_name: "Hit By Pitch".into(),
            espn_stat_id: None,
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "hbp".into(),
            },
        },
        // Batting — counting, lower-is-better
        ("GIDP", PlayerType::Hitter) => StatDefinition {
            abbrev: "GIDP".into(),
            display_name: "Grounded Into Double Play".into(),
            espn_stat_id: None,
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::LowerIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "gidp".into(),
            },
        },
        // Batting — additional rate stats
        ("OBP", PlayerType::Hitter) => StatDefinition {
            abbrev: "OBP".into(),
            display_name: "On-base Percentage".into(),
            espn_stat_id: Some(99),
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 3,
            close_threshold: 0.005,
            matchup_close_threshold: 0.020,
            computation: StatComputation::RateStat {
                volume_key: "pa".into(),
                rate_key: "obp".into(),
                divisor: 1.0,
            },
        },
        ("SLG", PlayerType::Hitter) => StatDefinition {
            abbrev: "SLG".into(),
            display_name: "Slugging Percentage".into(),
            espn_stat_id: Some(102),
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 3,
            close_threshold: 0.010,
            matchup_close_threshold: 0.040,
            computation: StatComputation::RateStat {
                volume_key: "ab".into(),
                rate_key: "slg".into(),
                divisor: 1.0,
            },
        },
        // Batting — derived counting (computed during projection loading)
        ("OPS", PlayerType::Hitter) => StatDefinition {
            abbrev: "OPS".into(),
            display_name: "On-base Plus Slugging".into(),
            espn_stat_id: None,
            player_type: PlayerType::Hitter,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 3,
            close_threshold: 0.010,
            matchup_close_threshold: 0.040,
            computation: StatComputation::Counting {
                projection_key: "ops".into(),
            },
        },
        // Pitching — additional counting stats
        ("QS", PlayerType::Pitcher) => StatDefinition {
            abbrev: "QS".into(),
            display_name: "Quality Starts".into(),
            espn_stat_id: None,
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "qs".into(),
            },
        },
        ("CG", PlayerType::Pitcher) => StatDefinition {
            abbrev: "CG".into(),
            display_name: "Complete Games".into(),
            espn_stat_id: None,
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "cg".into(),
            },
        },
        ("SHO", PlayerType::Pitcher) => StatDefinition {
            abbrev: "SHO".into(),
            display_name: "Shutouts".into(),
            espn_stat_id: None,
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "sho".into(),
            },
        },
        // Pitching — counting, lower-is-better
        ("L", PlayerType::Pitcher) => StatDefinition {
            abbrev: "L".into(),
            display_name: "Losses".into(),
            espn_stat_id: None,
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::LowerIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "l".into(),
            },
        },
        ("BSV", PlayerType::Pitcher) => StatDefinition {
            abbrev: "BSV".into(),
            display_name: "Blown Saves".into(),
            espn_stat_id: None,
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::LowerIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "bsv".into(),
            },
        },
        // Pitching — derived counting (computed during projection loading)
        ("K/9", PlayerType::Pitcher) => StatDefinition {
            abbrev: "K/9".into(),
            display_name: "Strikeouts per 9 IP".into(),
            espn_stat_id: None,
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 2,
            close_threshold: 0.25,
            matchup_close_threshold: 1.00,
            computation: StatComputation::Counting {
                projection_key: "k9".into(),
            },
        },
        ("K/BB", PlayerType::Pitcher) => StatDefinition {
            abbrev: "K/BB".into(),
            display_name: "Strikeout to Walk Ratio".into(),
            espn_stat_id: None,
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 2,
            close_threshold: 0.25,
            matchup_close_threshold: 1.00,
            computation: StatComputation::Counting {
                projection_key: "kbb".into(),
            },
        },
        ("NSV", PlayerType::Pitcher) => StatDefinition {
            abbrev: "NSV".into(),
            display_name: "Net Saves".into(),
            espn_stat_id: None,
            player_type: PlayerType::Pitcher,
            sort_direction: SortDirection::HigherIsBetter,
            format_precision: 0,
            close_threshold: 1.0,
            matchup_close_threshold: 3.0,
            computation: StatComputation::Counting {
                projection_key: "nsv".into(),
            },
        },
        _ => return None,
    };
    Some(def)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_registry() -> StatRegistry {
        StatRegistry::from_league_config(&LeagueConfig::default()).unwrap()
    }

    // ---- StatRegistry tests ----

    #[test]
    fn registry_from_default_config_produces_12_stats() {
        let reg = default_registry();
        assert_eq!(reg.len(), 12);
        assert!(!reg.is_empty());
    }

    #[test]
    fn registry_batting_count_is_6() {
        let reg = default_registry();
        assert_eq!(reg.batting_count(), 6);
        assert_eq!(reg.batting_stats().count(), 6);
    }

    #[test]
    fn registry_pitching_count_is_6() {
        let reg = default_registry();
        assert_eq!(reg.pitching_count(), 6);
        assert_eq!(reg.pitching_stats().count(), 6);
    }

    #[test]
    fn lookup_all_12_by_abbreviation() {
        let reg = default_registry();
        let expected = [
            "R", "HR", "RBI", "BB", "SB", "AVG", "K", "W", "SV", "HD", "ERA", "WHIP",
        ];
        for abbrev in expected {
            assert!(
                reg.get(abbrev).is_some(),
                "expected to find stat '{abbrev}' in registry"
            );
        }
    }

    #[test]
    fn index_of_batting_stats_are_0_through_5() {
        let reg = default_registry();
        let batting = ["R", "HR", "RBI", "BB", "SB", "AVG"];
        for (i, abbrev) in batting.iter().enumerate() {
            assert_eq!(reg.index_of(abbrev), Some(i), "index_of({abbrev})");
        }
    }

    #[test]
    fn index_of_pitching_stats_are_6_through_11() {
        let reg = default_registry();
        let pitching = ["K", "W", "SV", "HD", "ERA", "WHIP"];
        for (i, abbrev) in pitching.iter().enumerate() {
            assert_eq!(reg.index_of(abbrev), Some(i + 6), "index_of({abbrev})");
        }
    }

    #[test]
    fn registry_iter_order_matches_config_order() {
        let reg = default_registry();
        let expected = ["R", "HR", "RBI", "BB", "SB", "AVG", "K", "W", "SV", "HD", "ERA", "WHIP"];
        let actual: Vec<&str> = reg.all_stats().iter().map(|s| s.abbrev.as_str()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn batting_stats_iterator_yields_batting_only() {
        let reg = default_registry();
        for stat in reg.batting_stats() {
            assert_eq!(stat.player_type, PlayerType::Hitter, "{}", stat.abbrev);
        }
    }

    #[test]
    fn pitching_stats_iterator_yields_pitching_only() {
        let reg = default_registry();
        for stat in reg.pitching_stats() {
            assert_eq!(stat.player_type, PlayerType::Pitcher, "{}", stat.abbrev);
        }
    }

    #[test]
    fn unknown_category_returns_error() {
        let mut config = LeagueConfig::default();
        config
            .batting_categories
            .categories
            .push("XYZ".to_string());
        let result = StatRegistry::from_league_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("XYZ"),
            "error should mention the unknown abbreviation"
        );
    }

    #[test]
    fn wrong_player_type_returns_error() {
        let mut config = LeagueConfig::default();
        // ERA is a pitching stat — putting it in batting should fail
        config.batting_categories.categories = vec!["ERA".to_string()];
        config.pitching_categories.categories = vec![];
        let result = StatRegistry::from_league_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn duplicate_category_returns_error() {
        let mut config = LeagueConfig::default();
        config.batting_categories.categories.push("R".to_string());
        let result = StatRegistry::from_league_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("duplicate"),
            "error should mention duplicate: {}",
            err
        );
    }

    // ---- CategoryValues tests ----

    #[test]
    fn category_values_zeros() {
        let cv = CategoryValues::zeros(5);
        assert_eq!(cv.len(), 5);
        for i in 0..5 {
            assert_eq!(cv.get(i), Some(0.0));
        }
    }

    #[test]
    fn category_values_get_set_roundtrip() {
        let mut cv = CategoryValues::zeros(3);
        cv.set(0, 1.5);
        cv.set(1, 2.5);
        cv.set(2, 3.5);
        assert_eq!(cv.get(0), Some(1.5));
        assert_eq!(cv.get(1), Some(2.5));
        assert_eq!(cv.get(2), Some(3.5));
        assert_eq!(cv.get(3), None);
    }

    #[test]
    fn category_values_from_vec() {
        let cv = CategoryValues::from_vec(vec![1.0, 2.0, 3.0]);
        assert_eq!(cv.len(), 3);
        assert_eq!(cv.as_slice(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn category_values_weighted_sum() {
        let values = CategoryValues::from_vec(vec![2.0, 3.0, 4.0]);
        let weights = CategoryValues::from_vec(vec![1.0, 0.5, 0.25]);
        // 2*1 + 3*0.5 + 4*0.25 = 2 + 1.5 + 1 = 4.5
        assert!((values.weighted_sum(&weights) - 4.5).abs() < 1e-10);
    }

    #[test]
    #[should_panic(expected = "length mismatch")]
    fn category_values_weighted_sum_panics_on_mismatch() {
        let a = CategoryValues::zeros(3);
        let b = CategoryValues::zeros(4);
        a.weighted_sum(&b);
    }

    #[test]
    fn category_values_iter() {
        let cv = CategoryValues::from_vec(vec![10.0, 20.0, 30.0]);
        let pairs: Vec<(usize, f64)> = cv.iter().collect();
        assert_eq!(pairs, vec![(0, 10.0), (1, 20.0), (2, 30.0)]);
    }

    // ---- ProjectionData tests ----

    #[test]
    fn projection_data_get_or_zero_default() {
        let pd = ProjectionData::new();
        assert_eq!(pd.get_or_zero("missing"), 0.0);
        assert_eq!(pd.get("missing"), None);
    }

    #[test]
    fn projection_data_insert_and_get() {
        let mut pd = ProjectionData::new();
        pd.insert("hr", 35.0);
        assert_eq!(pd.get("hr"), Some(35.0));
        assert_eq!(pd.get_or_zero("hr"), 35.0);
    }

    // ---- rate_stat_contribution tests ----

    #[test]
    fn rate_stat_contribution_era_matches_legacy() {
        // era_contribution(ip=180, era=3.50, league_avg=4.00) = 180*(4.00-3.50)/9.0 = 10.0
        let result = rate_stat_contribution(180.0, 3.50, 4.00, 9.0, SortDirection::LowerIsBetter);
        assert!((result - 10.0).abs() < 1e-10);
    }

    #[test]
    fn rate_stat_contribution_whip_matches_legacy() {
        // whip_contribution(ip=180, whip=1.10, league_avg=1.30) = 180*(1.30-1.10)/1.0 = 36.0
        let result = rate_stat_contribution(180.0, 1.10, 1.30, 1.0, SortDirection::LowerIsBetter);
        assert!((result - 36.0).abs() < 1e-10);
    }

    #[test]
    fn rate_stat_contribution_avg_matches_legacy() {
        // avg_contribution(ab=500, avg=0.280, league_avg=0.260) = 500*(0.280-0.260)/1.0 = 10.0
        let result =
            rate_stat_contribution(500.0, 0.280, 0.260, 1.0, SortDirection::HigherIsBetter);
        assert!((result - 10.0).abs() < 1e-10);
    }

    #[test]
    fn rate_stat_contribution_higher_below_average_is_negative() {
        // A hitter with AVG below league average has negative contribution
        let result =
            rate_stat_contribution(500.0, 0.240, 0.260, 1.0, SortDirection::HigherIsBetter);
        assert!(result < 0.0);
    }

    #[test]
    fn rate_stat_contribution_lower_above_average_is_negative() {
        // A pitcher with ERA above league average has negative contribution
        let result = rate_stat_contribution(180.0, 4.50, 4.00, 9.0, SortDirection::LowerIsBetter);
        assert!(result < 0.0);
    }

    // ---- lookup_stat_definition tests ----

    #[test]
    fn lookup_known_batting_stat() {
        let def = lookup_stat_definition("HR", PlayerType::Hitter).unwrap();
        assert_eq!(def.abbrev, "HR");
        assert_eq!(def.espn_stat_id, Some(5));
        assert_eq!(def.player_type, PlayerType::Hitter);
        assert_eq!(def.sort_direction, SortDirection::HigherIsBetter);
        assert_eq!(def.format_precision, 0);
    }

    #[test]
    fn lookup_known_pitching_rate_stat() {
        let def = lookup_stat_definition("ERA", PlayerType::Pitcher).unwrap();
        assert_eq!(def.abbrev, "ERA");
        assert_eq!(def.espn_stat_id, Some(47));
        assert_eq!(def.sort_direction, SortDirection::LowerIsBetter);
        assert_eq!(def.format_precision, 2);
        assert!(matches!(
            def.computation,
            StatComputation::RateStat { divisor, .. } if (divisor - 9.0).abs() < 1e-10
        ));
    }

    #[test]
    fn lookup_unknown_stat_returns_none() {
        assert!(lookup_stat_definition("XYZ", PlayerType::Hitter).is_none());
    }

    #[test]
    fn lookup_wrong_player_type_returns_none() {
        // ERA is a pitching stat, not a hitting stat
        assert!(lookup_stat_definition("ERA", PlayerType::Hitter).is_none());
        // R is a hitting stat, not a pitching stat
        assert!(lookup_stat_definition("R", PlayerType::Pitcher).is_none());
    }

    // ---- Expanded knowledge base tests (WI-14) ----

    #[test]
    fn lookup_all_new_hitting_stats() {
        let hitting = [
            ("TB", SortDirection::HigherIsBetter, 0),
            ("XBH", SortDirection::HigherIsBetter, 0),
            ("HBP", SortDirection::HigherIsBetter, 0),
            ("GIDP", SortDirection::LowerIsBetter, 0),
            ("OBP", SortDirection::HigherIsBetter, 3),
            ("SLG", SortDirection::HigherIsBetter, 3),
            ("OPS", SortDirection::HigherIsBetter, 3),
        ];
        for (abbrev, direction, precision) in hitting {
            let def = lookup_stat_definition(abbrev, PlayerType::Hitter)
                .unwrap_or_else(|| panic!("expected to find hitting stat '{abbrev}'"));
            assert_eq!(def.player_type, PlayerType::Hitter, "{abbrev}");
            assert_eq!(def.sort_direction, direction, "{abbrev} direction");
            assert_eq!(def.format_precision, precision, "{abbrev} precision");
        }
    }

    #[test]
    fn lookup_all_new_pitching_stats() {
        let pitching = [
            ("QS", SortDirection::HigherIsBetter, 0),
            ("CG", SortDirection::HigherIsBetter, 0),
            ("SHO", SortDirection::HigherIsBetter, 0),
            ("L", SortDirection::LowerIsBetter, 0),
            ("BSV", SortDirection::LowerIsBetter, 0),
            ("K/9", SortDirection::HigherIsBetter, 2),
            ("K/BB", SortDirection::HigherIsBetter, 2),
            ("NSV", SortDirection::HigherIsBetter, 0),
        ];
        for (abbrev, direction, precision) in pitching {
            let def = lookup_stat_definition(abbrev, PlayerType::Pitcher)
                .unwrap_or_else(|| panic!("expected to find pitching stat '{abbrev}'"));
            assert_eq!(def.player_type, PlayerType::Pitcher, "{abbrev}");
            assert_eq!(def.sort_direction, direction, "{abbrev} direction");
            assert_eq!(def.format_precision, precision, "{abbrev} precision");
        }
    }

    #[test]
    fn new_stats_wrong_player_type_returns_none() {
        // Hitting stats should not resolve as pitching
        for abbrev in ["TB", "XBH", "HBP", "GIDP", "OBP", "SLG", "OPS"] {
            assert!(
                lookup_stat_definition(abbrev, PlayerType::Pitcher).is_none(),
                "{abbrev} should not resolve as Pitcher"
            );
        }
        // Pitching stats should not resolve as hitting
        for abbrev in ["QS", "CG", "SHO", "L", "BSV", "K/9", "K/BB", "NSV"] {
            assert!(
                lookup_stat_definition(abbrev, PlayerType::Hitter).is_none(),
                "{abbrev} should not resolve as Hitter"
            );
        }
    }

    #[test]
    fn registry_with_nonstandard_categories() {
        use crate::config::CategoriesSection;

        let config = LeagueConfig {
            batting_categories: CategoriesSection {
                categories: vec![
                    "OBP".to_string(),
                    "SLG".to_string(),
                    "OPS".to_string(),
                    "HR".to_string(),
                ],
            },
            pitching_categories: CategoriesSection {
                categories: vec![
                    "QS".to_string(),
                    "K/9".to_string(),
                    "ERA".to_string(),
                ],
            },
            ..LeagueConfig::default()
        };
        let reg = StatRegistry::from_league_config(&config).unwrap();
        assert_eq!(reg.len(), 7);
        assert_eq!(reg.batting_count(), 4);
        assert_eq!(reg.pitching_count(), 3);
        assert!(reg.get("OPS").is_some());
        assert!(reg.get("K/9").is_some());
        assert_eq!(reg.index_of("QS"), Some(4));
    }

    #[test]
    fn all_27_known_stats_resolve() {
        let hitting = ["R", "HR", "RBI", "BB", "SB", "AVG", "TB", "XBH", "HBP", "GIDP", "OBP", "SLG", "OPS"];
        let pitching = ["K", "W", "SV", "HD", "ERA", "WHIP", "QS", "CG", "SHO", "L", "BSV", "K/9", "K/BB", "NSV"];

        for abbrev in hitting {
            assert!(
                lookup_stat_definition(abbrev, PlayerType::Hitter).is_some(),
                "hitting stat '{abbrev}' should resolve"
            );
        }
        for abbrev in pitching {
            assert!(
                lookup_stat_definition(abbrev, PlayerType::Pitcher).is_some(),
                "pitching stat '{abbrev}' should resolve"
            );
        }
        // Total: 13 hitting + 14 pitching = 27 known stats
        assert_eq!(hitting.len() + pitching.len(), 27);
    }

    #[test]
    fn obp_and_slg_are_rate_stats() {
        let obp = lookup_stat_definition("OBP", PlayerType::Hitter).unwrap();
        assert!(matches!(
            obp.computation,
            StatComputation::RateStat { ref volume_key, ref rate_key, divisor }
                if volume_key == "pa" && rate_key == "obp" && (divisor - 1.0).abs() < 1e-10
        ));

        let slg = lookup_stat_definition("SLG", PlayerType::Hitter).unwrap();
        assert!(matches!(
            slg.computation,
            StatComputation::RateStat { ref volume_key, ref rate_key, divisor }
                if volume_key == "ab" && rate_key == "slg" && (divisor - 1.0).abs() < 1e-10
        ));
    }

    #[test]
    fn matchup_close_thresholds_are_set() {
        let reg = default_registry();
        for stat in reg.all_stats() {
            assert!(
                stat.matchup_close_threshold > stat.close_threshold,
                "{}: matchup_close_threshold ({}) should be > close_threshold ({})",
                stat.abbrev,
                stat.matchup_close_threshold,
                stat.close_threshold,
            );
        }
    }

    #[test]
    fn matchup_close_threshold_for_rate_stats() {
        let era = lookup_stat_definition("ERA", PlayerType::Pitcher).unwrap();
        assert!(
            (era.matchup_close_threshold - 1.00).abs() < 1e-10,
            "ERA matchup_close_threshold should be 1.00, got {}",
            era.matchup_close_threshold,
        );
        let whip = lookup_stat_definition("WHIP", PlayerType::Pitcher).unwrap();
        assert!(
            (whip.matchup_close_threshold - 0.20).abs() < 1e-10,
            "WHIP matchup_close_threshold should be 0.20, got {}",
            whip.matchup_close_threshold,
        );
    }

    #[test]
    fn espn_stat_ids_for_new_stats() {
        // OBP and SLG have known ESPN IDs; others are None
        assert_eq!(
            lookup_stat_definition("OBP", PlayerType::Hitter).unwrap().espn_stat_id,
            Some(99)
        );
        assert_eq!(
            lookup_stat_definition("SLG", PlayerType::Hitter).unwrap().espn_stat_id,
            Some(102)
        );
        assert_eq!(
            lookup_stat_definition("OPS", PlayerType::Hitter).unwrap().espn_stat_id,
            None
        );
        assert_eq!(
            lookup_stat_definition("K/9", PlayerType::Pitcher).unwrap().espn_stat_id,
            None
        );
    }
}
