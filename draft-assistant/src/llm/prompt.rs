// Prompt templates for nomination analysis and nomination planning.
//
// Constructs compact, structured prompts for the Claude API to analyze
// draft nominations and plan future nominations. Each prompt includes
// pre-computed numbers so the LLM focuses on trade-offs and context
// rather than arithmetic.

use crate::config::LeagueConfig;
use crate::draft::pick::Position;
use crate::draft::roster::Roster;
use crate::draft::state::DraftState;
use crate::protocol::NominationInfo;
use crate::stats::{CategoryValues, StatRegistry};
use crate::valuation::auction::InflationTracker;
use crate::valuation::scarcity::ScarcityEntry;
use crate::valuation::zscore::{CategoryZScores, PlayerValuation};

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

/// A drafted player used as a market comparison.
#[derive(Debug, Clone)]
pub struct MarketComp {
    pub player_name: String,
    pub position: String,
    pub predraft_value: f64,
    pub paid_price: u32,
    pub overpay_pct: f64,
}

/// Info about a similar available player for prompt context.
#[derive(Debug, Clone)]
pub struct SimilarPlayerInfo {
    pub name: String,
    pub position: String,
    pub dollar_value: f64,
    pub adjusted_value: f64,
}

/// A candidate player to nominate in order to drive up prices for opponents.
#[derive(Debug, Clone)]
pub struct SellCandidate {
    pub name: String,
    pub position: String,
    pub dollar_value: f64,
    pub reason: String,
}

/// Pre-computed budget constraints passed to the LLM for fiscal discipline.
#[derive(Debug, Clone)]
pub struct BudgetContext {
    /// Budget remaining for the user's team.
    pub budget_remaining: u32,
    /// Number of empty roster slots still to fill.
    pub empty_slots: usize,
    /// Maximum safe bid: budget_remaining - (empty_slots - 1). Every remaining slot costs at least $1.
    pub max_safe_bid: u32,
    /// Average dollars per remaining slot: budget_remaining / empty_slots.
    pub avg_per_slot: f64,
    /// Current draft pick number.
    pub pick_number: usize,
    /// Total picks in the draft.
    pub total_picks: usize,
    /// Pre-computed bid floor from the instant analysis engine (70% of adjusted value).
    pub engine_bid_floor: u32,
    /// Pre-computed bid ceiling from the instant analysis engine (adjusted + scarcity premium).
    pub engine_bid_ceiling: u32,
    /// The engine's verdict label (e.g. "STRONG TARGET", "CONDITIONAL", "PASS").
    pub engine_verdict: String,
}

// ---------------------------------------------------------------------------
// League context helpers
// ---------------------------------------------------------------------------

/// Format scoring type from config value to display string.
fn format_scoring_type(scoring_type: &str) -> String {
    match scoring_type {
        "h2h_most_categories" => "H2H Most Categories".to_string(),
        other => other.replace('_', " "),
    }
}

/// Count draftable roster slots (total minus IL).
fn draftable_roster_size(roster: &std::collections::HashMap<String, usize>) -> usize {
    roster
        .iter()
        .filter(|(k, _)| k.as_str() != "IL")
        .map(|(_, v)| v)
        .sum()
}

/// Format league context line from config for use in LLM prompts.
pub fn format_league_context(league: &LeagueConfig, roster_config: Option<&std::collections::HashMap<String, usize>>) -> String {
    let scoring = format_scoring_type(&league.scoring_type);
    let batting = league.batting_categories.categories.join(", ");
    let pitching = league.pitching_categories.categories.join(", ");
    let roster_info = match roster_config {
        Some(rc) => {
            let roster_size = draftable_roster_size(rc);
            format!("{}-player rosters", roster_size)
        }
        None => "roster pending (waiting for ESPN connection)".to_string(),
    };
    format!(
        "{}-team {}, salary cap ${}, {}.\n\
         Categories: {} (hitting) | {} (pitching)",
        league.num_teams, scoring, league.salary_cap, roster_info, batting, pitching
    )
}

// ---------------------------------------------------------------------------
// System prompt
// ---------------------------------------------------------------------------

/// Return the system prompt for all draft advisory LLM calls.
///
/// When a strategy overview is provided (from the strategy wizard), it is
/// appended so the LLM understands the user's strategic intent.
pub fn system_prompt(league: &LeagueConfig, roster_config: Option<&std::collections::HashMap<String, usize>>, strategy_overview: Option<&str>) -> String {
    let strategy_section = match strategy_overview {
        Some(overview) if !overview.trim().is_empty() => {
            format!(
                "--- MY DRAFT STRATEGY ---\n\
                 {}\n\
                 \n\
                 Your recommendations MUST align with this strategy. Evaluate every player through the lens of my strategic priorities above.",
                overview.trim()
            )
        }
        _ => "No specific draft strategy provided. Give balanced recommendations based on value."
            .to_string(),
    };

    let league_ctx = format_league_context(league, roster_config);
    format!(
        "You are a fantasy baseball auction draft advisor.\n\
         \n\
         {}\n\
         \n\
         {}\n\
         \n\
         For each nominated player, you will provide:\n\
         1. VERDICT: One of BID TO WIN / BID IF CHEAP / DRIVE UP PRICE / PASS\n\
         2. BID RANGE: A minimum (steal price) and maximum (walk-away price)\n\
         3. FIT: How this player fits my specific roster and category needs\n\
         4. STRATEGY: What to think about \u{2014} competing bidders, comparable players available later, draft position implications\n\
         \n\
         Be concise and direct. Use the pre-computed numbers I provide \u{2014} do NOT do arithmetic. Focus on trade-offs and context the numbers don't capture.\n\
         \n\
         BUDGET DISCIPLINE: You must NEVER recommend a maximum bid above the user's max safe bid. \
         Always consider how a bid affects the remaining budget-per-slot average. \
         If a player's adjusted value exceeds the max safe bid, say so explicitly and recommend passing or bidding only to drive up the price for opponents.",
        league_ctx, strategy_section
    )
}

// ---------------------------------------------------------------------------
// Nomination analysis prompt
// ---------------------------------------------------------------------------

/// Build a prompt for analyzing a specific player nomination.
///
/// The prompt includes all relevant context: the nominated player's profile,
/// the user's roster state, category needs, positional scarcity, similar
/// available players, and recent market comparisons.
pub fn build_nomination_analysis_prompt(
    player: &PlayerValuation,
    nomination: &NominationInfo,
    my_roster: &Roster,
    category_needs: &CategoryValues,
    scarcity: &[ScarcityEntry],
    available_players: &[PlayerValuation],
    draft_state: &DraftState,
    inflation: &InflationTracker,
    budget: &BudgetContext,
    registry: &StatRegistry,
) -> String {
    let adjusted_value = inflation.adjust(player.dollar_value);
    let positions_str = player
        .positions
        .iter()
        .map(|p| p.display_str())
        .collect::<Vec<_>>()
        .join("/");

    let mut prompt = String::with_capacity(2048);

    // Section 1: NOMINATION header
    prompt.push_str(&format!(
        "## NOMINATION\n\
         Player: {} ({})\n\
         Nominated by: {} | Current bid: ${}\n\
         Pre-draft value: ${:.0} | Adjusted value: ${:.0} | VOR: {:.1}\n\n",
        player.name,
        positions_str,
        nomination.nominated_by,
        nomination.current_bid,
        player.dollar_value,
        adjusted_value,
        player.vor,
    ));

    // Section 2: PLAYER PROFILE
    prompt.push_str("## PLAYER PROFILE\n");
    prompt.push_str(&format_player_profile(player, available_players, registry));
    prompt.push('\n');

    // Section 3: MY ROSTER
    prompt.push_str("## MY ROSTER\n");
    prompt.push_str(&format_roster_for_prompt(my_roster));
    prompt.push_str(&format!(
        "Budget: ${} remaining | {} slots open | Max safe bid: ${} | Avg ${:.0}/slot\n\n",
        budget.budget_remaining,
        budget.empty_slots,
        budget.max_safe_bid,
        budget.avg_per_slot,
    ));

    // Section 4: CATEGORY NEEDS
    prompt.push_str("## CATEGORY NEEDS\n");
    prompt.push_str(&format_category_needs(category_needs, registry));
    prompt.push('\n');

    // Section 5: POSITIONAL SCARCITY
    prompt.push_str("## POSITIONAL SCARCITY (relevant positions)\n");
    for pos in &player.positions {
        if let Some(entry) = scarcity.iter().find(|s| s.position == *pos) {
            prompt.push_str(&format!(
                "  {} : {} ({} above replacement, dropoff {:.1})\n",
                pos.display_str(),
                entry.urgency.label(),
                entry.players_above_replacement,
                entry.dropoff,
            ));
        }
    }
    prompt.push('\n');

    // Section 5b: BUDGET CONSTRAINTS
    prompt.push_str("## BUDGET CONSTRAINTS\n");
    prompt.push_str(&format!(
        "  Draft progress: Pick {} of {}\n",
        budget.pick_number, budget.total_picks,
    ));
    prompt.push_str(&format!(
        "  Engine verdict: {} | Bid floor: ${} | Bid ceiling: ${}\n",
        budget.engine_verdict, budget.engine_bid_floor, budget.engine_bid_ceiling,
    ));
    if budget.engine_bid_ceiling > budget.max_safe_bid {
        prompt.push_str(&format!(
            "  WARNING: Bid ceiling (${}) exceeds max safe bid (${}). Budget-constrained.\n",
            budget.engine_bid_ceiling, budget.max_safe_bid,
        ));
    }
    prompt.push('\n');

    // Section 6: SIMILAR PLAYERS
    let similar = find_similar_players(player, available_players, inflation, 3);
    if !similar.is_empty() {
        prompt.push_str("## SIMILAR AVAILABLE PLAYERS\n");
        for sp in &similar {
            prompt.push_str(&format!(
                "  {} ({}) - Value: ${:.0}, Adj: ${:.0}\n",
                sp.name, sp.position, sp.dollar_value, sp.adjusted_value,
            ));
        }
        prompt.push('\n');
    }

    // Section 7: RECENT MARKET COMPS
    let comps = find_market_comps(draft_state, player, available_players);
    if !comps.is_empty() {
        prompt.push_str("## RECENT MARKET COMPS\n");
        for comp in &comps {
            prompt.push_str(&format!(
                "  {} ({}) - Value: ${:.0}, Paid: ${}, Overpay: {:+.0}%\n",
                comp.player_name,
                comp.position,
                comp.predraft_value,
                comp.paid_price,
                comp.overpay_pct,
            ));
        }
        prompt.push('\n');
    }

    // Section 8: Closing question
    prompt.push_str(
        "## WHAT SHOULD I DO?\n\
         Give me your verdict, bid range, fit assessment, and strategy notes.",
    );

    prompt
}

// ---------------------------------------------------------------------------
// Nomination planning prompt
// ---------------------------------------------------------------------------

/// Build a prompt for planning what player to nominate next.
///
/// Includes the user's current roster, category strengths, positional scarcity,
/// opponent budget snapshots, top available targets, and sell candidates.
pub fn build_nomination_planning_prompt(
    my_roster: &Roster,
    category_needs: &CategoryValues,
    scarcity: &[ScarcityEntry],
    available_players: &[PlayerValuation],
    draft_state: &DraftState,
    inflation: &InflationTracker,
    budget: &BudgetContext,
    registry: &StatRegistry,
) -> String {
    let my_team_id = draft_state
        .my_team()
        .map(|t| t.team_id.as_str())
        .unwrap_or("");
    let mut prompt = String::with_capacity(2048);

    // Section 1: Header
    prompt.push_str(&format!(
        "## NOMINATION PLANNING\n\
         Pick {} of {} | My budget: ${} | Inflation rate: {:.2}x | {} open slots | Avg ${:.0}/slot | Max bid: ${}\n\n",
        budget.pick_number,
        budget.total_picks,
        budget.budget_remaining,
        inflation.inflation_rate,
        budget.empty_slots,
        budget.avg_per_slot,
        budget.max_safe_bid,
    ));

    // Section 2: MY ROSTER state
    prompt.push_str("## MY ROSTER\n");
    prompt.push_str(&format_roster_for_prompt(my_roster));
    prompt.push('\n');

    // Section 3: CATEGORY STRENGTHS
    prompt.push_str("## CATEGORY STRENGTHS (need level, higher = more need)\n");
    prompt.push_str(&format_category_needs(category_needs, registry));
    prompt.push('\n');

    // Section 4: POSITIONAL SCARCITY
    prompt.push_str("## POSITIONAL SCARCITY\n");
    for entry in scarcity {
        prompt.push_str(&format!(
            "  {} : {} ({} above replacement)\n",
            entry.position.display_str(),
            entry.urgency.label(),
            entry.players_above_replacement,
        ));
    }
    prompt.push('\n');

    // Section 5: OPPONENT BUDGET SNAPSHOT
    prompt.push_str("## OPPONENT BUDGETS\n");
    for team in &draft_state.teams {
        if team.team_id == my_team_id {
            continue;
        }
        prompt.push_str(&format!(
            "  {} : ${} spent, ${} remaining, {} slots open\n",
            team.team_name,
            team.budget_spent,
            team.budget_remaining,
            team.roster.empty_slots(),
        ));
    }
    prompt.push('\n');

    // Section 6: TOP 10 AVAILABLE PLAYERS I WANT
    let top_targets = find_top_targets(available_players, my_roster, inflation, 10);
    prompt.push_str("## TOP 10 AVAILABLE TARGETS (sorted by adjusted value x roster fit)\n");
    for (i, p) in top_targets.iter().enumerate() {
        let adj = inflation.adjust(p.dollar_value);
        let positions_str = p
            .positions
            .iter()
            .map(|pos| pos.display_str())
            .collect::<Vec<_>>()
            .join("/");
        let fills = if p.positions.iter().any(|pos| my_roster.has_empty_slot(*pos)) {
            " [FILLS SLOT]"
        } else {
            ""
        };
        prompt.push_str(&format!(
            "  {}. {} ({}) - ${:.0} adj, VOR {:.1}{}\n",
            i + 1,
            p.name,
            positions_str,
            adj,
            p.vor,
            fills,
        ));
    }
    prompt.push('\n');

    // Section 7: TOP 5 "NOMINATE TO SELL" CANDIDATES
    let sell_candidates =
        find_nominate_to_sell_candidates(available_players, my_roster, draft_state, 5);
    if !sell_candidates.is_empty() {
        prompt.push_str("## TOP 5 \"NOMINATE TO SELL\" CANDIDATES\n");
        for (i, sc) in sell_candidates.iter().enumerate() {
            prompt.push_str(&format!(
                "  {}. {} ({}) - ${:.0} value - {}\n",
                i + 1,
                sc.name,
                sc.position,
                sc.dollar_value,
                sc.reason,
            ));
        }
        prompt.push('\n');
    }

    // Section 8: Closing question
    prompt.push_str(
        "## WHO SHOULD I NOMINATE AND WHY?\n\
         Give me your top pick to nominate, backup option, and reasoning.",
    );

    prompt
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Format a player's per-category projections, z-scores, and pool rank.
fn format_player_profile(
    player: &PlayerValuation,
    available_players: &[PlayerValuation],
    registry: &StatRegistry,
) -> String {
    let z = |abbrev: &str| -> f64 {
        player.category_zscores.get_by_abbrev(registry, abbrev).unwrap_or(0.0)
    };

    let mut s = String::new();
    let proj = &player.projection;

    if player.is_two_way {
        let h_ranks = compute_hitter_ranks(player, available_players, registry);
        let p_ranks = compute_pitcher_ranks(player, available_players, registry);
        s.push_str("  TWO-WAY PLAYER\n");
        s.push_str(&format!("  --- Hitting (PA: {}) ---\n", proj.get("pa") as u32));
        s.push_str("  Cat   Proj  Z-Score  Rank\n");
        s.push_str(&format!("  R     {:>4}  {:>+6.2}   #{}\n", proj.get("r") as u32, z("R"), h_ranks.0));
        s.push_str(&format!("  HR    {:>4}  {:>+6.2}   #{}\n", proj.get("hr") as u32, z("HR"), h_ranks.1));
        s.push_str(&format!("  RBI   {:>4}  {:>+6.2}   #{}\n", proj.get("rbi") as u32, z("RBI"), h_ranks.2));
        s.push_str(&format!("  BB    {:>4}  {:>+6.2}   #{}\n", proj.get("bb") as u32, z("BB"), h_ranks.3));
        s.push_str(&format!("  SB    {:>4}  {:>+6.2}   #{}\n", proj.get("sb") as u32, z("SB"), h_ranks.4));
        s.push_str(&format!("  AVG   {:.3}  {:>+6.2}   #{}\n", proj.get("avg"), z("AVG"), h_ranks.5));
        s.push_str(&format!("  --- Pitching (IP: {:.0}) ---\n", proj.get("ip")));
        s.push_str("  Cat   Proj  Z-Score  Rank\n");
        s.push_str(&format!("  K     {:>4}  {:>+6.2}   #{}\n", proj.get("k") as u32, z("K"), p_ranks.0));
        s.push_str(&format!("  W     {:>4}  {:>+6.2}   #{}\n", proj.get("w") as u32, z("W"), p_ranks.1));
        s.push_str(&format!("  SV    {:>4}  {:>+6.2}   #{}\n", proj.get("sv") as u32, z("SV"), p_ranks.2));
        s.push_str(&format!("  HD    {:>4}  {:>+6.2}   #{}\n", proj.get("hd") as u32, z("HD"), p_ranks.3));
        s.push_str(&format!("  ERA   {:.2}  {:>+6.2}   #{}\n", proj.get("era"), z("ERA"), p_ranks.4));
        s.push_str(&format!("  WHIP  {:.2}  {:>+6.2}   #{}\n", proj.get("whip"), z("WHIP"), p_ranks.5));
    } else if player.is_pitcher {
        s.push_str(&format!("  IP: {:.0}\n", proj.get("ip")));
        s.push_str("  Cat   Proj  Z-Score  Rank\n");
        let ranks = compute_pitcher_ranks(player, available_players, registry);
        s.push_str(&format!("  K     {:>4}  {:>+6.2}   #{}\n", proj.get("k") as u32, z("K"), ranks.0));
        s.push_str(&format!("  W     {:>4}  {:>+6.2}   #{}\n", proj.get("w") as u32, z("W"), ranks.1));
        s.push_str(&format!("  SV    {:>4}  {:>+6.2}   #{}\n", proj.get("sv") as u32, z("SV"), ranks.2));
        s.push_str(&format!("  HD    {:>4}  {:>+6.2}   #{}\n", proj.get("hd") as u32, z("HD"), ranks.3));
        s.push_str(&format!("  ERA   {:.2}  {:>+6.2}   #{}\n", proj.get("era"), z("ERA"), ranks.4));
        s.push_str(&format!("  WHIP  {:.2}  {:>+6.2}   #{}\n", proj.get("whip"), z("WHIP"), ranks.5));
    } else {
        s.push_str(&format!("  PA: {}\n", proj.get("pa") as u32));
        s.push_str("  Cat   Proj  Z-Score  Rank\n");
        let ranks = compute_hitter_ranks(player, available_players, registry);
        s.push_str(&format!("  R     {:>4}  {:>+6.2}   #{}\n", proj.get("r") as u32, z("R"), ranks.0));
        s.push_str(&format!("  HR    {:>4}  {:>+6.2}   #{}\n", proj.get("hr") as u32, z("HR"), ranks.1));
        s.push_str(&format!("  RBI   {:>4}  {:>+6.2}   #{}\n", proj.get("rbi") as u32, z("RBI"), ranks.2));
        s.push_str(&format!("  BB    {:>4}  {:>+6.2}   #{}\n", proj.get("bb") as u32, z("BB"), ranks.3));
        s.push_str(&format!("  SB    {:>4}  {:>+6.2}   #{}\n", proj.get("sb") as u32, z("SB"), ranks.4));
        s.push_str(&format!("  AVG   {:.3}  {:>+6.2}   #{}\n", proj.get("avg"), z("AVG"), ranks.5));
    }
    s
}

/// Compute per-category ranks for a hitter among available players.
/// Returns (R_rank, HR_rank, RBI_rank, BB_rank, SB_rank, AVG_rank).
fn compute_hitter_ranks(
    player: &PlayerValuation,
    available: &[PlayerValuation],
    registry: &StatRegistry,
) -> (usize, usize, usize, usize, usize, usize) {
    let abbrevs = ["R", "HR", "RBI", "BB", "SB", "AVG"];

    let my_vals: Vec<f64> = abbrevs
        .iter()
        .map(|a| player.category_zscores.get_by_abbrev(registry, a).unwrap_or(0.0))
        .collect();

    let hitter_z: Vec<&CategoryZScores> = available
        .iter()
        .filter(|p| !p.is_pitcher)
        .map(|p| &p.category_zscores)
        .collect();

    let rank = |idx: usize| -> usize {
        let abbrev = abbrevs[idx];
        let my_val = my_vals[idx];
        let better = hitter_z
            .iter()
            .filter(|z| z.get_by_abbrev(registry, abbrev).map(|v| v > my_val).unwrap_or(false))
            .count();
        better + 1
    };

    (rank(0), rank(1), rank(2), rank(3), rank(4), rank(5))
}

/// Compute per-category ranks for a pitcher among available players.
/// Returns (K_rank, W_rank, SV_rank, HD_rank, ERA_rank, WHIP_rank).
fn compute_pitcher_ranks(
    player: &PlayerValuation,
    available: &[PlayerValuation],
    registry: &StatRegistry,
) -> (usize, usize, usize, usize, usize, usize) {
    let abbrevs = ["K", "W", "SV", "HD", "ERA", "WHIP"];

    let my_vals: Vec<f64> = abbrevs
        .iter()
        .map(|a| player.category_zscores.get_by_abbrev(registry, a).unwrap_or(0.0))
        .collect();

    let pitcher_z: Vec<&CategoryZScores> = available
        .iter()
        .filter(|p| p.is_pitcher || p.is_two_way)
        .map(|p| &p.category_zscores)
        .collect();

    let rank = |idx: usize| -> usize {
        let abbrev = abbrevs[idx];
        let my_val = my_vals[idx];
        let better = pitcher_z
            .iter()
            .filter(|z| z.get_by_abbrev(registry, abbrev).map(|v| v > my_val).unwrap_or(false))
            .count();
        better + 1
    };

    (rank(0), rank(1), rank(2), rank(3), rank(4), rank(5))
}

/// Find market comps: recently drafted players at the same position with
/// similar pre-draft values. Computes overpay percentage.
pub fn find_market_comps(
    draft_state: &DraftState,
    player: &PlayerValuation,
    available_players: &[PlayerValuation],
) -> Vec<MarketComp> {
    let recent_picks = &draft_state.picks;
    if recent_picks.is_empty() {
        return Vec::new();
    }

    // Look at the last 20 picks (or fewer if not enough have been made).
    let window_start = recent_picks.len().saturating_sub(20);
    let recent = &recent_picks[window_start..];

    let player_positions: Vec<&str> = player.positions.iter().map(|p| p.display_str()).collect();

    let mut comps = Vec::new();

    for pick in recent {
        // Check if position matches.
        let pick_pos = Position::from_str_pos(&pick.position);
        let position_matches = pick_pos
            .map(|pp| player.positions.contains(&pp))
            .unwrap_or(false);

        if !position_matches {
            continue;
        }

        // Try to find the predraft value for this drafted player.
        // First check the available pool (in case they're still in it for
        // some reason), then estimate from the pick price.
        let predraft_value = available_players
            .iter()
            .find(|p| p.name == pick.player_name)
            .map(|p| p.dollar_value)
            .unwrap_or_else(|| {
                // Estimate: if not in the pool, use the price as a rough proxy
                // (with a small discount since people often overpay).
                pick.price as f64 * 0.85
            });

        if predraft_value < 1.0 {
            continue;
        }

        let overpay_pct = ((pick.price as f64 - predraft_value) / predraft_value) * 100.0;

        comps.push(MarketComp {
            player_name: pick.player_name.clone(),
            position: player_positions.first().unwrap_or(&"?").to_string(),
            predraft_value,
            paid_price: pick.price,
            overpay_pct,
        });
    }

    // Take the most recent 5 comps.
    comps.truncate(5);
    comps
}

/// Find available players similar to the target player (same position, similar value).
pub fn find_similar_players(
    player: &PlayerValuation,
    available_players: &[PlayerValuation],
    inflation: &InflationTracker,
    count: usize,
) -> Vec<SimilarPlayerInfo> {
    if player.dollar_value <= 1.0 {
        return Vec::new();
    }

    let value_threshold = player.dollar_value * 0.35;
    let min_value = player.dollar_value - value_threshold;
    let max_value = player.dollar_value + value_threshold;

    let mut similar: Vec<SimilarPlayerInfo> = available_players
        .iter()
        .filter(|p| {
            p.name != player.name
                && p.dollar_value >= min_value
                && p.dollar_value <= max_value
                && p.dollar_value > 1.0
                && p.positions.iter().any(|pos| player.positions.contains(pos))
        })
        .map(|p| {
            let positions_str = p
                .positions
                .iter()
                .map(|pos| pos.display_str())
                .collect::<Vec<_>>()
                .join("/");
            SimilarPlayerInfo {
                name: p.name.clone(),
                position: positions_str,
                dollar_value: p.dollar_value,
                adjusted_value: inflation.adjust(p.dollar_value),
            }
        })
        .collect();

    // Sort by dollar value descending, take top N.
    similar.sort_by(|a, b| {
        b.dollar_value
            .partial_cmp(&a.dollar_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    similar.truncate(count);
    similar
}

/// Find high-value players at positions the user has already filled, which
/// opponents need. These are good nomination candidates to drain opponent budgets.
pub fn find_nominate_to_sell_candidates(
    available_players: &[PlayerValuation],
    my_roster: &Roster,
    draft_state: &DraftState,
    count: usize,
) -> Vec<SellCandidate> {
    // Positions where my roster is already filled.
    let filled_positions: Vec<Position> = [
        Position::Catcher,
        Position::FirstBase,
        Position::SecondBase,
        Position::ThirdBase,
        Position::ShortStop,
        Position::LeftField,
        Position::CenterField,
        Position::RightField,
        Position::StartingPitcher,
        Position::ReliefPitcher,
    ]
    .iter()
    .filter(|&&pos| !my_roster.has_empty_slot(pos))
    .copied()
    .collect();

    if filled_positions.is_empty() {
        return Vec::new();
    }

    // Count how many opponents need each position (have empty slots).
    let my_team_id = draft_state
        .my_team()
        .map(|t| t.team_id.clone())
        .unwrap_or_default();
    let mut position_demand: std::collections::HashMap<Position, usize> =
        std::collections::HashMap::new();

    for team in &draft_state.teams {
        if team.team_id == my_team_id {
            continue;
        }
        for &pos in &filled_positions {
            if team.roster.has_empty_slot(pos) {
                *position_demand.entry(pos).or_insert(0) += 1;
            }
        }
    }

    let mut candidates: Vec<SellCandidate> = available_players
        .iter()
        .filter(|p| {
            p.dollar_value > 5.0 && p.positions.iter().any(|pos| filled_positions.contains(pos))
        })
        .map(|p| {
            let best_sell_pos = p
                .positions
                .iter()
                .filter(|pos| filled_positions.contains(pos))
                .max_by_key(|pos| position_demand.get(pos).copied().unwrap_or(0))
                .copied()
                .unwrap_or(p.positions[0]);

            let demand = position_demand.get(&best_sell_pos).copied().unwrap_or(0);

            let reason = format!(
                "{} teams need {}; I don't",
                demand,
                best_sell_pos.display_str()
            );

            SellCandidate {
                name: p.name.clone(),
                position: best_sell_pos.display_str().to_string(),
                dollar_value: p.dollar_value,
                reason,
            }
        })
        .collect();

    // Sort by dollar value descending (expensive players drain more budget).
    candidates.sort_by(|a, b| {
        b.dollar_value
            .partial_cmp(&a.dollar_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(count);
    candidates
}

/// Format the user's roster for prompt inclusion.
pub fn format_roster_for_prompt(roster: &Roster) -> String {
    let mut s = String::new();

    for slot in &roster.slots {
        if slot.position == Position::InjuredList {
            continue;
        }
        let status = match &slot.player {
            Some(p) => format!("{} (${}) ", p.name, p.price),
            None => "[EMPTY]".to_string(),
        };
        s.push_str(&format!(
            "  {:>4}: {}\n",
            slot.position.display_str(),
            status
        ));
    }

    s
}

/// Format category needs as a compact table.
pub fn format_category_needs(needs: &CategoryValues, registry: &StatRegistry) -> String {
    let mut s = String::new();
    s.push_str("  Hitting:  ");
    for &idx in registry.batting_indices() {
        let stat = &registry.all_stats()[idx];
        let val = needs.get(idx).unwrap_or(0.0);
        s.push_str(&format!("{}={:.2} ", stat.abbrev, val));
    }
    s.push('\n');
    s.push_str("  Pitching: ");
    for &idx in registry.pitching_indices() {
        let stat = &registry.all_stats()[idx];
        let val = needs.get(idx).unwrap_or(0.0);
        s.push_str(&format!("{}={:.2} ", stat.abbrev, val));
    }
    s.push('\n');
    s
}

/// Find top available players ranked by adjusted value, with a boost for
/// players who fill empty roster slots.
fn find_top_targets<'a>(
    available_players: &'a [PlayerValuation],
    my_roster: &Roster,
    inflation: &InflationTracker,
    count: usize,
) -> Vec<&'a PlayerValuation> {
    let mut scored: Vec<(&PlayerValuation, f64)> = available_players
        .iter()
        .filter(|p| p.dollar_value > 1.0)
        .map(|p| {
            let adj = inflation.adjust(p.dollar_value);
            let fills_slot = p.positions.iter().any(|pos| my_roster.has_empty_slot(*pos));
            let fit_bonus = if fills_slot { adj * 0.20 } else { 0.0 };
            (p, adj + fit_bonus)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    scored.into_iter().take(count).map(|(p, _)| p).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::draft::pick::DraftPick;
    use crate::draft::pick::Position;
    use crate::draft::roster::Roster;
    use crate::draft::state::DraftState;
    use crate::protocol::NominationInfo;
    use crate::valuation::auction::InflationTracker;
    use crate::valuation::projections::PitcherType;
    use crate::valuation::scarcity::compute_scarcity;
    use crate::stats::{CategoryValues, StatRegistry};
    use crate::valuation::zscore::{
        CategoryZScores, PlayerValuation, ProjectionData,
    };
    use std::collections::HashMap;

    fn test_registry() -> StatRegistry {
        StatRegistry::from_league_config(&test_league_config()).expect("valid test config")
    }

    // ---- Test helpers ----

    fn test_roster_config() -> HashMap<String, usize> {
        let mut config = HashMap::new();
        config.insert("C".into(), 1);
        config.insert("1B".into(), 1);
        config.insert("2B".into(), 1);
        config.insert("3B".into(), 1);
        config.insert("SS".into(), 1);
        config.insert("LF".into(), 1);
        config.insert("CF".into(), 1);
        config.insert("RF".into(), 1);
        config.insert("UTIL".into(), 1);
        config.insert("SP".into(), 5);
        config.insert("RP".into(), 6);
        config.insert("BE".into(), 6);
        config.insert("IL".into(), 5);
        config
    }

    fn test_league_config() -> LeagueConfig {
        LeagueConfig {
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
            teams: HashMap::new(),
        }
    }

    fn test_espn_budgets() -> Vec<crate::draft::state::TeamBudgetPayload> {
        (1..=10)
            .map(|i| crate::draft::state::TeamBudgetPayload {
                team_id: format!("{}", i),
                team_name: format!("Team {}", i),
                budget: 260,
            })
            .collect()
    }

    /// Helper: create a DraftState with teams pre-registered from ESPN data.
    fn create_test_draft_state() -> DraftState {
        let mut state = DraftState::new(260, &test_roster_config());
        state.reconcile_budgets(&test_espn_budgets());
        state.set_my_team_by_id("1");
        state
    }

    fn test_budget_context() -> BudgetContext {
        BudgetContext {
            budget_remaining: 260,
            empty_slots: 26,
            max_safe_bid: 235,
            avg_per_slot: 10.0,
            pick_number: 1,
            total_picks: 260,
            engine_bid_floor: 21,
            engine_bid_ceiling: 39,
            engine_verdict: "STRONG TARGET".to_string(),
        }
    }

    fn make_hitter(name: &str, vor: f64, positions: Vec<Position>, dollar: f64) -> PlayerValuation {
        let registry = test_registry();
        let mut zv = CategoryValues::zeros(registry.len());
        zv.set(registry.index_of("R").unwrap(), 1.5);
        zv.set(registry.index_of("HR").unwrap(), 1.2);
        zv.set(registry.index_of("RBI").unwrap(), 0.8);
        zv.set(registry.index_of("BB").unwrap(), 2.0);
        zv.set(registry.index_of("SB").unwrap(), 0.3);
        zv.set(registry.index_of("AVG").unwrap(), 0.5);
        PlayerValuation {
            name: name.into(),
            team: "TST".into(),
            positions: positions.clone(),
            is_pitcher: false,
            is_two_way: false,
            pitcher_type: None,
            projection: ProjectionData {
                values: HashMap::from([
                    ("pa".into(), 600.0), ("ab".into(), 550.0), ("h".into(), 150.0),
                    ("hr".into(), 25.0), ("r".into(), 80.0), ("rbi".into(), 85.0),
                    ("bb".into(), 50.0), ("sb".into(), 10.0), ("avg".into(), 0.273),
                ]),
            },
            total_zscore: vor + 2.0,
            category_zscores: CategoryZScores::hitter(zv, vor + 2.0),
            vor,
            initial_vor: 0.0,
            best_position: positions.first().copied(),
            dollar_value: dollar,
        }
    }

    fn make_pitcher(name: &str, vor: f64, pt: PitcherType, dollar: f64) -> PlayerValuation {
        let pos = match pt {
            PitcherType::SP => Position::StartingPitcher,
            PitcherType::RP => Position::ReliefPitcher,
        };
        let registry = test_registry();
        let mut zv = CategoryValues::zeros(registry.len());
        zv.set(registry.index_of("K").unwrap(), 1.5);
        zv.set(registry.index_of("W").unwrap(), 0.8);
        zv.set(registry.index_of("ERA").unwrap(), 1.2);
        zv.set(registry.index_of("WHIP").unwrap(), 0.9);
        PlayerValuation {
            name: name.into(),
            team: "TST".into(),
            positions: vec![pos],
            is_pitcher: true,
            is_two_way: false,
            pitcher_type: Some(pt),
            projection: ProjectionData {
                values: HashMap::from([
                    ("ip".into(), 180.0), ("k".into(), 200.0), ("w".into(), 14.0),
                    ("sv".into(), 0.0), ("hd".into(), 0.0), ("era".into(), 3.20),
                    ("whip".into(), 1.10), ("g".into(), 30.0), ("gs".into(), 30.0),
                ]),
            },
            total_zscore: vor + 1.0,
            category_zscores: CategoryZScores::pitcher(zv, vor + 1.0),
            vor,
            initial_vor: 0.0,
            best_position: Some(pos),
            dollar_value: dollar,
        }
    }

    // ---- System prompt tests ----

    #[test]
    fn system_prompt_contains_key_elements() {
        let league = test_league_config();
        let sp = system_prompt(&league, None, None);
        assert!(
            sp.contains("10-team H2H Most Categories"),
            "should mention league format"
        );
        assert!(sp.contains("VERDICT"), "should mention verdict");
        assert!(sp.contains("BID RANGE"), "should mention bid range");
        assert!(
            sp.contains("No specific draft strategy provided"),
            "should show fallback when no strategy provided"
        );
        assert!(
            !sp.contains("Stars-and-scrubs"),
            "should not contain hardcoded strategy"
        );
    }

    #[test]
    fn system_prompt_includes_strategy_overview() {
        let league = test_league_config();
        let sp = system_prompt(&league, None, Some("Target elite closers early, punt saves entirely."));
        assert!(
            sp.contains("--- MY DRAFT STRATEGY ---"),
            "should include strategy header"
        );
        assert!(
            sp.contains("Target elite closers early"),
            "should include strategy text"
        );
        assert!(
            sp.contains("Your recommendations MUST align with this strategy"),
            "should include alignment instruction"
        );
        assert!(
            !sp.contains("No specific draft strategy provided"),
            "should not show fallback when strategy is provided"
        );
    }

    #[test]
    fn system_prompt_skips_empty_overview() {
        let league = test_league_config();
        let sp = system_prompt(&league, None, Some("   "));
        assert!(
            !sp.contains("MY DRAFT STRATEGY"),
            "should not include strategy header for whitespace-only overview"
        );
        assert!(
            sp.contains("No specific draft strategy provided"),
            "should show fallback for whitespace-only overview"
        );
    }

    #[test]
    fn system_prompt_uses_league_config_values() {
        let mut league = test_league_config();
        league.num_teams = 12;
        league.salary_cap = 300;
        league.scoring_type = "roto".into();
        league.batting_categories.categories =
            vec!["R".into(), "HR".into(), "OBP".into()];
        league.pitching_categories.categories =
            vec!["K".into(), "ERA".into(), "WHIP".into()];

        let sp = system_prompt(&league, None, None);
        assert!(sp.contains("12-team"), "should reflect num_teams from config");
        assert!(sp.contains("$300"), "should reflect salary_cap from config");
        assert!(
            sp.contains("OBP"),
            "should reflect custom batting categories"
        );
        assert!(
            !sp.contains("BB"),
            "should not contain categories not in config"
        );
        assert!(
            !sp.contains("H2H Most Categories"),
            "should reflect scoring_type from config"
        );
    }

    // ---- Nomination analysis prompt tests ----

    #[test]
    fn nomination_analysis_prompt_contains_sections() {
        let registry = test_registry();
        let player = make_hitter("Mike Trout", 10.0, vec![Position::CenterField], 45.0);
        let nomination = NominationInfo {
            player_name: "Mike Trout".into(),
            position: "CF".into(),
            nominated_by: "Team 5".into(),
            current_bid: 1,
            current_bidder: None,
            time_remaining: Some(30),
            eligible_slots: vec![],
        };
        let roster = Roster::new(&test_roster_config());
        let needs = CategoryValues::uniform(registry.len(), 0.5);
        let available = vec![
            player.clone(),
            make_hitter("Similar CF", 8.0, vec![Position::CenterField], 38.0),
        ];
        let scarcity = compute_scarcity(&available, &test_roster_config());
        let draft_state = create_test_draft_state();
        let inflation = InflationTracker::new();

        let prompt = build_nomination_analysis_prompt(
            &player,
            &nomination,
            &roster,
            &needs,
            &scarcity,
            &available,
            &draft_state,
            &inflation,
            &test_budget_context(),
            &registry,
        );

        assert!(
            prompt.contains("## NOMINATION"),
            "should have NOMINATION section"
        );
        assert!(prompt.contains("Mike Trout"), "should contain player name");
        assert!(prompt.contains("CF"), "should contain position");
        assert!(prompt.contains("Team 5"), "should contain nominator");
        assert!(
            prompt.contains("## PLAYER PROFILE"),
            "should have PLAYER PROFILE section"
        );
        assert!(
            prompt.contains("## MY ROSTER"),
            "should have MY ROSTER section"
        );
        assert!(
            prompt.contains("## CATEGORY NEEDS"),
            "should have CATEGORY NEEDS section"
        );
        assert!(
            prompt.contains("## POSITIONAL SCARCITY"),
            "should have SCARCITY section"
        );
        assert!(
            prompt.contains("WHAT SHOULD I DO"),
            "should have closing question"
        );
    }

    #[test]
    fn nomination_analysis_prompt_includes_values() {
        let registry = test_registry();
        let player = make_hitter("Test Player", 8.0, vec![Position::FirstBase], 30.0);
        let nomination = NominationInfo {
            player_name: "Test Player".into(),
            position: "1B".into(),
            nominated_by: "Team 3".into(),
            current_bid: 5,
            current_bidder: Some("Team 3".into()),
            time_remaining: Some(25),
            eligible_slots: vec![],
        };
        let roster = Roster::new(&test_roster_config());
        let needs = CategoryValues::uniform(registry.len(), 0.5);
        let available = vec![player.clone()];
        let scarcity = compute_scarcity(&available, &test_roster_config());
        let draft_state = create_test_draft_state();
        let inflation = InflationTracker::new();

        let prompt = build_nomination_analysis_prompt(
            &player,
            &nomination,
            &roster,
            &needs,
            &scarcity,
            &available,
            &draft_state,
            &inflation,
            &test_budget_context(),
            &registry,
        );

        assert!(prompt.contains("$30"), "should contain dollar value");
        assert!(prompt.contains("VOR: 8.0"), "should contain VOR");
        assert!(prompt.contains("$5"), "should contain current bid");
    }

    // ---- Nomination planning prompt tests ----

    #[test]
    fn nomination_planning_prompt_contains_sections() {
        let registry = test_registry();
        let roster = Roster::new(&test_roster_config());
        let needs = CategoryValues::uniform(registry.len(), 0.5);
        let available = vec![
            make_hitter("H1", 10.0, vec![Position::FirstBase], 40.0),
            make_hitter("H2", 8.0, vec![Position::SecondBase], 35.0),
            make_pitcher("P1", 7.0, PitcherType::SP, 30.0),
        ];
        let scarcity = compute_scarcity(&available, &test_roster_config());
        let draft_state = create_test_draft_state();
        let inflation = InflationTracker::new();

        let prompt = build_nomination_planning_prompt(
            &roster,
            &needs,
            &scarcity,
            &available,
            &draft_state,
            &inflation,
            &test_budget_context(),
            &registry,
        );

        assert!(
            prompt.contains("## NOMINATION PLANNING"),
            "should have header"
        );
        assert!(
            prompt.contains("## MY ROSTER"),
            "should have roster section"
        );
        assert!(
            prompt.contains("## CATEGORY STRENGTHS"),
            "should have category section"
        );
        assert!(
            prompt.contains("## POSITIONAL SCARCITY"),
            "should have scarcity section"
        );
        assert!(
            prompt.contains("## OPPONENT BUDGETS"),
            "should have opponent section"
        );
        assert!(
            prompt.contains("## TOP 10 AVAILABLE TARGETS"),
            "should have targets section"
        );
        assert!(
            prompt.contains("WHO SHOULD I NOMINATE"),
            "should have closing question"
        );
    }

    #[test]
    fn planning_prompt_shows_opponent_budgets() {
        let registry = test_registry();
        let roster = Roster::new(&test_roster_config());
        let needs = CategoryValues::uniform(registry.len(), 0.5);
        let available = vec![make_hitter("H1", 10.0, vec![Position::FirstBase], 40.0)];
        let scarcity = compute_scarcity(&available, &test_roster_config());
        let mut draft_state = create_test_draft_state();

        // Record a pick so Team 2 has spent money
        draft_state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "2".into(),
            team_name: "Team 2".into(),
            player_name: "Drafted Player".into(),
            position: "SP".into(),
            price: 50,
            espn_player_id: None,
            eligible_slots: vec![],
            assigned_slot: None,
        });

        let inflation = InflationTracker::new();

        let prompt = build_nomination_planning_prompt(
            &roster,
            &needs,
            &scarcity,
            &available,
            &draft_state,
            &inflation,
            &test_budget_context(),
            &registry,
        );

        assert!(prompt.contains("Team 2"), "should list opponent teams");
        assert!(
            prompt.contains("$50 spent"),
            "should show opponent spending"
        );
        assert!(
            prompt.contains("$210 remaining"),
            "should show opponent remaining budget"
        );
    }

    // ---- Market comp tests ----

    #[test]
    fn find_market_comps_empty_draft() {
        let draft_state = create_test_draft_state();
        let player = make_hitter("Test", 5.0, vec![Position::FirstBase], 20.0);
        let available = vec![player.clone()];

        let comps = find_market_comps(&draft_state, &player, &available);
        assert!(comps.is_empty(), "no comps with no picks");
    }

    #[test]
    fn find_market_comps_with_picks() {
        let mut draft_state = create_test_draft_state();

        // Draft a first baseman for $25
        draft_state.record_pick(DraftPick {
            pick_number: 1,
            team_id: "2".into(),
            team_name: "Team 2".into(),
            player_name: "Other 1B".into(),
            position: "1B".into(),
            price: 25,
            espn_player_id: None,
            eligible_slots: vec![],
            assigned_slot: None,
        });

        // Draft an SP (different position)
        draft_state.record_pick(DraftPick {
            pick_number: 2,
            team_id: "3".into(),
            team_name: "Team 3".into(),
            player_name: "Ace SP".into(),
            position: "SP".into(),
            price: 40,
            espn_player_id: None,
            eligible_slots: vec![],
            assigned_slot: None,
        });

        let player = make_hitter("My 1B", 6.0, vec![Position::FirstBase], 22.0);
        let available = vec![player.clone()];

        let comps = find_market_comps(&draft_state, &player, &available);

        // Should find the 1B comp, not the SP
        assert_eq!(comps.len(), 1, "should find exactly one 1B comp");
        assert_eq!(comps[0].player_name, "Other 1B");
    }

    #[test]
    fn find_market_comps_limits_to_5() {
        let mut draft_state = create_test_draft_state();

        for i in 0..10 {
            draft_state.record_pick(DraftPick {
                pick_number: i + 1,
                team_id: format!("team_{}", (i % 9) + 2),
                team_name: format!("Team {}", (i % 9) + 2),
                player_name: format!("1B Player {}", i),
                position: "1B".into(),
                price: 20 + i,
                espn_player_id: None,
                eligible_slots: vec![],
                assigned_slot: None,
            });
        }

        let player = make_hitter("Target 1B", 5.0, vec![Position::FirstBase], 20.0);
        let available = vec![player.clone()];

        let comps = find_market_comps(&draft_state, &player, &available);
        assert!(
            comps.len() <= 5,
            "should limit to 5 comps, got {}",
            comps.len()
        );
    }

    // ---- Similar player tests ----

    #[test]
    fn find_similar_players_same_position() {
        let target = make_hitter("Target", 5.0, vec![Position::FirstBase], 20.0);
        let available = vec![
            target.clone(),
            make_hitter("Similar1", 4.5, vec![Position::FirstBase], 18.0),
            make_hitter("Similar2", 5.5, vec![Position::FirstBase], 22.0),
            make_hitter("TooFar", 1.0, vec![Position::FirstBase], 5.0),
            make_hitter("WrongPos", 5.0, vec![Position::Catcher], 20.0),
        ];
        let inflation = InflationTracker::new();

        let similar = find_similar_players(&target, &available, &inflation, 3);

        // Should find Similar1 and Similar2 but not TooFar or WrongPos
        assert_eq!(similar.len(), 2);
        let names: Vec<&str> = similar.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Similar1"));
        assert!(names.contains(&"Similar2"));
    }

    #[test]
    fn find_similar_players_excludes_self() {
        let target = make_hitter("Target", 5.0, vec![Position::FirstBase], 20.0);
        let available = vec![target.clone()];
        let inflation = InflationTracker::new();

        let similar = find_similar_players(&target, &available, &inflation, 3);
        assert!(similar.is_empty(), "should not include self");
    }

    #[test]
    fn find_similar_players_respects_count() {
        let target = make_hitter("Target", 5.0, vec![Position::FirstBase], 20.0);
        let available = vec![
            target.clone(),
            make_hitter("S1", 5.0, vec![Position::FirstBase], 19.0),
            make_hitter("S2", 5.0, vec![Position::FirstBase], 18.0),
            make_hitter("S3", 5.0, vec![Position::FirstBase], 17.0),
            make_hitter("S4", 5.0, vec![Position::FirstBase], 16.0),
        ];
        let inflation = InflationTracker::new();

        let similar = find_similar_players(&target, &available, &inflation, 2);
        assert_eq!(similar.len(), 2, "should respect count limit");
    }

    #[test]
    fn find_similar_players_empty_for_low_value() {
        let target = make_hitter("Cheap", 0.5, vec![Position::FirstBase], 1.0);
        let available = vec![
            target.clone(),
            make_hitter("Other", 0.5, vec![Position::FirstBase], 1.0),
        ];
        let inflation = InflationTracker::new();

        let similar = find_similar_players(&target, &available, &inflation, 3);
        assert!(similar.is_empty(), "should return empty for $1 player");
    }

    // ---- Nominate-to-sell tests ----

    #[test]
    fn nominate_to_sell_finds_candidates() {
        let mut roster = Roster::new(&test_roster_config());
        // Fill the CF slot
        roster.add_player("My CF", "CF", 30, None);

        let available = vec![
            make_hitter("Good CF", 8.0, vec![Position::CenterField], 35.0),
            make_hitter("Cheap CF", 2.0, vec![Position::CenterField], 5.0),
        ];

        let draft_state = create_test_draft_state();

        let candidates = find_nominate_to_sell_candidates(&available, &roster, &draft_state, 5);

        // Should find the expensive CF since we already have CF filled
        assert!(!candidates.is_empty(), "should find sell candidates");
        assert_eq!(candidates[0].name, "Good CF");
        assert!(
            candidates[0].reason.contains("CF"),
            "reason should mention position"
        );
    }

    #[test]
    fn nominate_to_sell_excludes_cheap_players() {
        let mut roster = Roster::new(&test_roster_config());
        roster.add_player("My C", "C", 10, None);

        let available = vec![make_hitter("Cheap C", 0.5, vec![Position::Catcher], 3.0)];

        let draft_state = create_test_draft_state();

        let candidates = find_nominate_to_sell_candidates(&available, &roster, &draft_state, 5);
        assert!(
            candidates.is_empty(),
            "should not nominate cheap players to sell"
        );
    }

    #[test]
    fn nominate_to_sell_empty_when_no_filled_positions() {
        let roster = Roster::new(&test_roster_config()); // All empty

        let available = vec![make_hitter(
            "Good 1B",
            10.0,
            vec![Position::FirstBase],
            40.0,
        )];

        let draft_state = create_test_draft_state();

        let candidates = find_nominate_to_sell_candidates(&available, &roster, &draft_state, 5);
        assert!(
            candidates.is_empty(),
            "should not sell when no positions filled"
        );
    }

    // ---- Roster formatting tests ----

    #[test]
    fn format_roster_shows_empty_and_filled() {
        let mut roster = Roster::new(&test_roster_config());
        roster.add_player("Mike Trout", "CF", 45, None);
        roster.add_player("Corbin Burnes", "SP", 35, None);

        let formatted = format_roster_for_prompt(&roster);

        assert!(formatted.contains("[EMPTY]"), "should show empty slots");
        assert!(
            formatted.contains("Mike Trout"),
            "should show filled player"
        );
        assert!(formatted.contains("$45"), "should show player price");
        assert!(
            formatted.contains("Corbin Burnes"),
            "should show second player"
        );
        // "UTIL" contains substring "IL", so check for the specific IL slot format
        assert!(!formatted.contains("  IL:"), "should exclude IL slots");
    }

    #[test]
    fn format_roster_excludes_il() {
        let roster = Roster::new(&test_roster_config());
        let formatted = format_roster_for_prompt(&roster);

        // Count lines - should not include IL slots
        let lines: Vec<&str> = formatted.lines().filter(|l| !l.is_empty()).collect();
        // 26 draftable slots (excluding 5 IL)
        assert_eq!(lines.len(), 26, "should have 26 lines (excluding IL)");
    }

    // ---- Category needs formatting tests ----

    #[test]
    fn format_category_needs_includes_all_categories() {
        let registry = test_registry();
        // Registry order: R, HR, RBI, BB, SB, AVG, K, W, SV, HD, ERA, WHIP
        let needs = CategoryValues::from_vec(vec![
            0.8, 0.5, 0.3, 1.0, 0.2, 0.4, 0.6, 0.7, 0.1, 0.9, 0.5, 0.6,
        ]);

        let formatted = format_category_needs(&needs, &registry);

        assert!(formatted.contains("R=0.80"), "should contain R value");
        assert!(formatted.contains("HR=0.50"), "should contain HR value");
        assert!(formatted.contains("BB=1.00"), "should contain BB value");
        assert!(formatted.contains("K=0.60"), "should contain K value");
        assert!(formatted.contains("HD=0.90"), "should contain HD value");
        assert!(formatted.contains("Hitting:"), "should label hitting row");
        assert!(formatted.contains("Pitching:"), "should label pitching row");
    }

    // ---- Player profile formatting tests ----

    #[test]
    fn player_profile_hitter_shows_all_categories() {
        let player = make_hitter("Test Hitter", 5.0, vec![Position::FirstBase], 20.0);
        let available = vec![player.clone()];

        let profile = format_player_profile(&player, &available, &test_registry());

        assert!(profile.contains("PA: 600"), "should show PA");
        assert!(profile.contains("R"), "should show R category");
        assert!(profile.contains("HR"), "should show HR category");
        assert!(profile.contains("BB"), "should show BB category");
        assert!(profile.contains("AVG"), "should show AVG category");
        assert!(profile.contains("Z-Score"), "should show z-score header");
        assert!(profile.contains("Rank"), "should show rank header");
    }

    #[test]
    fn player_profile_pitcher_shows_all_categories() {
        let player = make_pitcher("Test Pitcher", 5.0, PitcherType::SP, 20.0);
        let available = vec![player.clone()];

        let profile = format_player_profile(&player, &available, &test_registry());

        assert!(profile.contains("IP: 180"), "should show IP");
        assert!(profile.contains("K"), "should show K category");
        assert!(profile.contains("W"), "should show W category");
        assert!(profile.contains("ERA"), "should show ERA category");
        assert!(profile.contains("WHIP"), "should show WHIP category");
    }

    // ---- Budget constraints tests ----

    #[test]
    fn nomination_analysis_prompt_contains_budget_constraints() {
        let registry = test_registry();
        let player = make_hitter("Test Player", 8.0, vec![Position::FirstBase], 30.0);
        let nomination = NominationInfo {
            player_name: "Test Player".into(),
            position: "1B".into(),
            nominated_by: "Team 3".into(),
            current_bid: 5,
            current_bidder: Some("Team 3".into()),
            time_remaining: Some(25),
            eligible_slots: vec![],
        };
        let roster = Roster::new(&test_roster_config());
        let needs = CategoryValues::uniform(registry.len(), 0.5);
        let available = vec![player.clone()];
        let scarcity = compute_scarcity(&available, &test_roster_config());
        let draft_state = create_test_draft_state();
        let inflation = InflationTracker::new();
        let budget = test_budget_context();

        let prompt = build_nomination_analysis_prompt(
            &player,
            &nomination,
            &roster,
            &needs,
            &scarcity,
            &available,
            &draft_state,
            &inflation,
            &budget,
            &registry,
        );

        assert!(prompt.contains("## BUDGET CONSTRAINTS"), "should have budget constraints section");
        assert!(prompt.contains("Max safe bid: $235"), "should contain max safe bid");
        assert!(prompt.contains("Avg $10/slot"), "should contain avg per slot");
        assert!(prompt.contains("Engine verdict: STRONG TARGET"), "should contain engine verdict");
        assert!(prompt.contains("Bid floor: $21"), "should contain bid floor");
        assert!(prompt.contains("Bid ceiling: $39"), "should contain bid ceiling");
        assert!(prompt.contains("Pick 1 of 260"), "should contain draft progress");
    }
}
