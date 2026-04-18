// Shared strategy configuration types used by both app logic and TUI rendering.

use wyncast_core::config;

/// Default category list matching the league's configured category order.
///
/// Hitting categories first (R, HR, RBI, BB, SB, AVG),
/// then pitching categories (K, W, SV, HD, ERA, WHIP).
pub fn default_categories() -> Vec<String> {
    ["R", "HR", "RBI", "BB", "SB", "AVG", "K", "W", "SV", "HD", "ERA", "WHIP"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Build a category list from a league config (batting + pitching categories).
pub fn categories_from_league(league: &config::LeagueConfig) -> Vec<String> {
    league
        .batting_categories
        .categories
        .iter()
        .chain(league.pitching_categories.categories.iter())
        .cloned()
        .collect()
}

/// Number of columns in the category weight grid.
pub const WEIGHT_COLS: usize = 3;

/// Category weight multipliers for all 12 league categories.
///
/// Provides indexed access via `get(idx)` and `set(idx, val)` using the
/// `CATEGORIES` const array ordering.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryWeights {
    categories: Vec<String>,
    weights: Vec<f32>,
}

impl Default for CategoryWeights {
    fn default() -> Self {
        Self::new(default_categories())
    }
}

impl CategoryWeights {
    /// Create a new `CategoryWeights` with the given category names.
    /// All weights default to 1.0, except "SV" which defaults to 0.7.
    pub fn new(categories: Vec<String>) -> Self {
        let weights = categories
            .iter()
            .map(|cat| if cat == "SV" { 0.7 } else { 1.0 })
            .collect();
        CategoryWeights { categories, weights }
    }

    /// The category name list.
    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    /// Number of categories.
    pub fn len(&self) -> usize {
        self.categories.len()
    }

    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
    }

    /// Get the weight value at the given index.
    pub fn get(&self, idx: usize) -> f32 {
        self.weights.get(idx).copied().unwrap_or(1.0)
    }

    /// Set the weight value at the given index. No-op for out-of-bounds.
    pub fn set(&mut self, idx: usize, val: f32) {
        if let Some(w) = self.weights.get_mut(idx) {
            *w = val;
        }
    }

    /// Look up a weight by category name.
    pub fn get_by_name(&self, name: &str) -> Option<f32> {
        self.categories
            .iter()
            .position(|c| c == name)
            .map(|idx| self.weights[idx])
    }

    /// Return (category_name, weight_as_f64) pairs for serialization.
    pub fn to_pairs(&self) -> Vec<(&str, f64)> {
        self.categories
            .iter()
            .zip(self.weights.iter())
            .map(|(cat, &w)| (cat.as_str(), w as f64))
            .collect()
    }

    /// Convert to the config-compatible `CategoryWeights` (HashMap-based).
    pub fn to_config_weights(&self) -> config::CategoryWeights {
        config::CategoryWeights::from_pairs(
            self.categories.iter().zip(self.weights.iter())
                .map(|(cat, &val)| (cat.clone(), val as f64))
        )
    }

    /// Create from the config-compatible `CategoryWeights` with a given category list.
    pub fn from_config_weights(
        w: &config::CategoryWeights,
        categories: Vec<String>,
    ) -> Self {
        let weights = categories
            .iter()
            .map(|cat| w.weight(cat) as f32)
            .collect();
        CategoryWeights { categories, weights }
    }

    /// Create from a slice of values using default categories.
    pub fn from_values(values: &[f32]) -> Self {
        let categories = default_categories();
        assert_eq!(
            categories.len(),
            values.len(),
            "values length must match default categories count"
        );
        CategoryWeights {
            categories,
            weights: values.to_vec(),
        }
    }
}

/// Mask an API key for display, showing only the first 7 and last 4 characters.
pub fn mask_api_key(key: &str) -> String {
    let char_count = key.chars().count();
    if char_count < 8 {
        return String::new();
    }
    let prefix: String = key.chars().take(7).collect();
    let suffix: String = key.chars().skip(char_count - 4).collect();
    let dots = "\u{2022}".repeat(5);
    format!("{}{}{}", prefix, dots, suffix)
}
