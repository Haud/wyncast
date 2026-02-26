// Valuation engine: z-scores, VOR, auction dollar conversion.

pub mod auction;
pub mod projections;
pub mod scarcity;
pub mod vor;
pub mod zscore;

use crate::config::Config;
use projections::AllProjections;
use zscore::PlayerValuation;

// ---------------------------------------------------------------------------
// Full valuation pipeline
// ---------------------------------------------------------------------------

/// Run the complete initial valuation pipeline:
///
/// 1. **Z-scores** — compute per-category z-scores for every player, producing
///    a `Vec<PlayerValuation>` sorted by total z-score.
/// 2. **VOR** — adjust z-scores by positional replacement level, sort by VOR.
/// 3. **Auction dollars** — convert VOR into dollar values using the league's
///    salary cap, sort by dollar value descending.
///
/// The returned list is sorted by descending dollar value, ready for display
/// or further processing (inflation tracking, scarcity adjustments, etc.).
pub fn compute_initial(
    projections: &AllProjections,
    config: &Config,
) -> anyhow::Result<Vec<PlayerValuation>> {
    // Step 1: Z-scores
    let mut players = zscore::compute_initial_zscores(projections, config);

    // Step 2: VOR adjustment
    vor::apply_vor(&mut players, &config.league);

    // Step 3: Auction dollar conversion
    auction::apply_auction_values(&mut players, &config.league, &config.strategy);

    Ok(players)
}
