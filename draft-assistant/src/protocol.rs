// Message protocol types for WebSocket communication and internal async channels.

use serde::{Deserialize, Serialize};

use crate::draft::pick::DraftPick;
use crate::draft::roster::RosterSlot;
use crate::llm::provider::LlmProvider;
use crate::matchup::MatchupSnapshot;
use crate::onboarding::OnboardingStep;
use crate::valuation::scarcity::ScarcityEntry;
use crate::stats::ProjectionData;
use crate::valuation::zscore::PlayerValuation;

// ---------------------------------------------------------------------------
// Extension -> Backend messages (JSON over WebSocket)
// ---------------------------------------------------------------------------

/// Messages received from the Firefox extension over WebSocket.
/// Serialized/deserialized as internally-tagged JSON using the `type` field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ExtensionMessage {
    /// Sent once when the extension first connects.
    #[serde(rename = "EXTENSION_CONNECTED")]
    ExtensionConnected { payload: ExtensionConnectedPayload },

    /// Periodic draft-state snapshot pushed by the extension.
    #[serde(rename = "STATE_UPDATE")]
    StateUpdate {
        timestamp: u64,
        payload: StateUpdatePayload,
    },

    /// Full state snapshot sent on initial connect or reconnect.
    ///
    /// When the extension connects (or reconnects) to an in-progress draft,
    /// it sends this message with the complete current draft state (all picks,
    /// rosters, budgets) before resuming incremental diffs. The backend resets
    /// its in-memory draft state and rebuilds from this snapshot, preventing
    /// corrupted state that would result from applying diffs against a blank slate.
    #[serde(rename = "FULL_STATE_SYNC")]
    FullStateSync {
        timestamp: u64,
        payload: StateUpdatePayload,
    },

    /// Keep-alive heartbeat from the extension.
    #[serde(rename = "EXTENSION_HEARTBEAT")]
    ExtensionHeartbeat { payload: HeartbeatPayload },

    /// Player projections scraped from ESPN's Fantasy API by the extension.
    #[serde(rename = "PLAYER_PROJECTIONS")]
    PlayerProjections {
        timestamp: u64,
        payload: EspnProjectionsPayload,
    },

    /// Matchup state snapshot from the ESPN matchup page.
    #[serde(rename = "MATCHUP_STATE")]
    MatchupState {
        timestamp: u64,
        payload: MatchupStatePayload,
    },
}

// ---------------------------------------------------------------------------
// Payload structs (camelCase JSON <-> snake_case Rust)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionConnectedPayload {
    pub platform: String,
    pub extension_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StateUpdatePayload {
    #[serde(default)]
    pub picks: Vec<PickData>,
    #[serde(default)]
    pub current_nomination: Option<NominationData>,
    pub my_team_id: Option<String>,
    #[serde(default)]
    pub teams: Vec<TeamBudgetData>,
    /// Current pick number from the ESPN clock label (e.g. "PK 128 OF 260").
    #[serde(default)]
    pub pick_count: Option<u32>,
    /// Total number of picks from the ESPN clock label.
    #[serde(default)]
    pub total_picks: Option<u32>,
    /// Unique draft identifier scraped from the ESPN page (e.g. league ID
    /// from the URL, or a team-name fingerprint). Used to detect when a new
    /// draft has started across reconnects.
    #[serde(default)]
    pub draft_id: Option<String>,
    pub source: Option<String>,

    // --- New fields for complete draft state synchronization ---

    /// Complete draft board grid data (all teams × all roster slots).
    /// Always fully rendered in the ESPN DOM, never virtualized.
    /// Sent on both STATE_UPDATE and FULL_STATE_SYNC.
    #[serde(default)]
    pub draft_board: Option<DraftBoardData>,

    /// Chronological pick history from the pick-history-tables section.
    /// All rounds fully rendered. Only sent on FULL_STATE_SYNC (expensive).
    #[serde(default)]
    pub pick_history: Option<Vec<PickHistoryEntry>>,

    /// Team name to ESPN numeric team ID mapping from the roster dropdown.
    /// Sent on both STATE_UPDATE and FULL_STATE_SYNC.
    #[serde(default)]
    pub team_id_mapping: Option<Vec<TeamIdMapping>>,

}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PickData {
    pub pick_number: u32,
    pub team_id: String,
    pub team_name: String,
    pub player_id: String,
    pub player_name: String,
    pub position: String,
    pub price: u32,
    #[serde(default)]
    pub eligible_slots: Vec<u16>,
    /// The ESPN roster slot ID that ESPN assigned this player to when
    /// the pick was made. Sent by the extension when it can determine
    /// the actual placement slot. None / absent if unknown.
    #[serde(default)]
    pub assigned_slot: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NominationData {
    pub player_id: String,
    pub player_name: String,
    pub position: String,
    pub nominated_by: String,
    pub current_bid: u32,
    pub current_bidder: Option<String>,
    pub time_remaining: Option<u32>,
    #[serde(default)]
    pub eligible_slots: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamBudgetData {
    /// ESPN team ID extracted from the pick train (e.g. "1", "2").
    /// Optional for backward compatibility with older extension messages.
    #[serde(default)]
    pub team_id: Option<String>,
    pub team_name: String,
    pub budget: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatPayload {
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Matchup state payload (from ESPN matchup page)
// ---------------------------------------------------------------------------

/// Matchup state scraped from the ESPN matchup page by the extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchupStatePayload {
    pub matchup_period: u8,
    pub start_date: String,
    pub end_date: String,
    pub selected_day: String,
    pub my_team: MatchupTeamPayload,
    pub opp_team: MatchupTeamPayload,
    pub categories: Vec<MatchupCategoryPayload>,
    pub batting: MatchupSectionPayload,
    pub pitching: MatchupSectionPayload,
}

/// A team's info within the matchup WebSocket message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchupTeamPayload {
    pub name: String,
    /// Season record as "W-L-T" string (e.g. "0-0-0").
    pub record: String,
    /// Category score within this matchup as "W-L-T" string (e.g. "2-3-7").
    pub matchup_score: String,
}

/// A single category's values from the matchup WebSocket message.
///
/// `my_value` and `opp_value` are `Option<f64>` because ESPN renders `"--"`
/// for rate stats (AVG/ERA/WHIP) with a zero denominator, and the scoreboard
/// can be partially rendered while the page is still loading.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchupCategoryPayload {
    pub stat_id: u16,
    pub abbrev: String,
    pub my_value: Option<f64>,
    pub opp_value: Option<f64>,
    pub lower_is_better: bool,
}

/// Batting or pitching section of the matchup WebSocket message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchupSectionPayload {
    pub headers: Vec<String>,
    pub players: Vec<MatchupPlayerPayload>,
    #[serde(default)]
    pub totals: Option<Vec<Option<f64>>>,
}

/// A single player row in the matchup batting/pitching section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchupPlayerPayload {
    pub slot: String,
    pub name: String,
    pub team: String,
    pub positions: Vec<String>,
    pub opponent: Option<String>,
    pub status: Option<String>,
    pub stats: Vec<Option<f64>>,
}

// ---------------------------------------------------------------------------
// ESPN projection types (player projections from ESPN Fantasy API)
// ---------------------------------------------------------------------------

/// Player projections scraped from ESPN's Fantasy API by the extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EspnProjectionsPayload {
    pub players: Vec<EspnPlayerProjection>,
}

/// A single player's projection data from ESPN.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EspnPlayerProjection {
    /// ESPN's internal player ID.
    pub espn_id: u32,
    pub name: String,
    pub team: String,
    /// ESPN defaultPositionId (1=SP, 2=C, 3=1B, 4=2B, 5=3B, 6=SS, 7=LF, 8=CF, 9=RF, 10=DH, 11=RP).
    pub default_position_id: u16,
    /// ESPN eligible slot IDs for multi-position eligibility.
    #[serde(default)]
    pub eligible_slots: Vec<u16>,
    /// Projected batting stats (None if player is pitcher-only).
    pub batting: Option<EspnBattingProjection>,
    /// Projected pitching stats (None if player is hitter-only).
    pub pitching: Option<EspnPitchingProjection>,
}

/// Projected batting stats from ESPN.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EspnBattingProjection {
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

/// Projected pitching stats from ESPN.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EspnPitchingProjection {
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

impl From<&EspnBattingProjection> for ProjectionData {
    fn from(proj: &EspnBattingProjection) -> Self {
        let mut data = ProjectionData::new();
        data.insert("pa", f64::from(proj.pa));
        data.insert("ab", f64::from(proj.ab));
        data.insert("h", f64::from(proj.h));
        data.insert("hr", f64::from(proj.hr));
        data.insert("r", f64::from(proj.r));
        data.insert("rbi", f64::from(proj.rbi));
        data.insert("bb", f64::from(proj.bb));
        data.insert("sb", f64::from(proj.sb));
        data.insert("avg", proj.avg);
        data
    }
}

impl From<&EspnPitchingProjection> for ProjectionData {
    fn from(proj: &EspnPitchingProjection) -> Self {
        let mut data = ProjectionData::new();
        data.insert("ip", proj.ip);
        data.insert("k", f64::from(proj.k));
        data.insert("w", f64::from(proj.w));
        data.insert("sv", f64::from(proj.sv));
        data.insert("hd", f64::from(proj.hd));
        data.insert("era", proj.era);
        data.insert("whip", proj.whip);
        data.insert("g", f64::from(proj.g));
        data.insert("gs", f64::from(proj.gs));
        if proj.ip > 0.0 {
            data.insert("k9", f64::from(proj.k) * 9.0 / proj.ip);
        }
        data
    }
}

// ---------------------------------------------------------------------------
// Draft board grid types (complete team × roster slot data)
// ---------------------------------------------------------------------------

/// Complete draft board grid data scraped from `div.draftBoardGrid`.
///
/// Contains all teams and their roster slots (filled and empty). Always
/// fully rendered in the ESPN DOM, making it the most reliable source for
/// roster state when resuming a draft mid-way.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DraftBoardData {
    pub teams: Vec<DraftBoardTeam>,
    pub on_the_clock_team: Option<String>,
}

/// A single team's data from the draft board grid header + cells.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DraftBoardTeam {
    #[serde(default)]
    pub team_id: String,
    pub team_name: String,
    pub column: u16,
    pub is_my_team: bool,
    pub is_on_the_clock: bool,
    pub slots: Vec<DraftBoardSlot>,
}

/// A single roster slot from the draft board grid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DraftBoardSlot {
    pub row: u16,
    pub roster_slot: String,
    pub filled: bool,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub pro_team: Option<String>,
    #[serde(default)]
    pub natural_position: Option<String>,
    #[serde(default)]
    pub price: Option<u32>,
}

// ---------------------------------------------------------------------------
// Pick history types (chronological pick order from round tables)
// ---------------------------------------------------------------------------

/// A single entry from the pick history tables.
///
/// The pick history section contains all rounds fully rendered, giving
/// complete chronological draft order with player IDs and eligible positions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PickHistoryEntry {
    pub pick_number: u32,
    pub round: u16,
    pub player_name: String,
    #[serde(default)]
    pub espn_player_id: String,
    #[serde(default)]
    pub eligible_positions: Vec<String>,
    #[serde(default)]
    pub team_id: String,
    pub team_name: String,
    pub price: u32,
    #[serde(default)]
    pub is_my_pick: bool,
}

// ---------------------------------------------------------------------------
// Team ID mapping (roster dropdown)
// ---------------------------------------------------------------------------

/// Maps a team name to its ESPN numeric team ID from the roster dropdown.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamIdMapping {
    pub team_name: String,
    pub espn_team_id: String,
}

// ---------------------------------------------------------------------------
// Internal connection events (not serialized to/from JSON)
// ---------------------------------------------------------------------------

/// Events generated by the WebSocket server for connection lifecycle.
#[derive(Debug, Clone, PartialEq)]
pub enum InternalEvent {
    /// A new extension client connected from the given address.
    Connected { addr: String },
    /// The extension client disconnected.
    Disconnected,
}

// ---------------------------------------------------------------------------
// App mode and settings
// ---------------------------------------------------------------------------

/// The current mode of the application UI.
///
/// Determines which screen the TUI renders and which input handlers are active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    /// First-run onboarding wizard (LLM setup, strategy config).
    Onboarding(OnboardingStep),
    /// Main draft dashboard (the default operational mode).
    Draft,
    /// Weekly head-to-head matchup view.
    Matchup,
    /// Settings screen (accessible from draft mode).
    Settings(SettingsSection),
}

/// Which section of the settings screen is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    /// LLM provider / model / API key configuration.
    LlmConfig,
    /// Strategy tuning (budget split, punt categories, etc.).
    StrategyConfig,
}

/// Actions the user can take during onboarding.
///
/// Sent from the TUI to the app orchestrator via `UserCommand::OnboardingAction`.
#[derive(Debug, Clone, PartialEq)]
pub enum OnboardingAction {
    /// Select an LLM provider.
    SetProvider(LlmProvider),
    /// Select a model (by model ID string).
    SetModel(String),
    /// Enter an API key.
    SetApiKey(String),
    /// Request an API connection test.
    TestConnection,
    /// Test the API connection with explicit provider, model, and key.
    /// Unlike `TestConnection`, this does NOT read from or mutate app state.
    /// Used by the settings cascade flow where the user hasn't saved yet.
    TestConnectionWith {
        provider: LlmProvider,
        model_id: String,
        api_key: String,
    },
    /// Save all LLM settings (provider, model, API key) in a single batch.
    /// Used by the settings page to defer persistence until the user presses 's'.
    SaveLlmConfig {
        provider: LlmProvider,
        model_id: String,
        api_key: Option<String>,
    },
    /// Save the strategy configuration with the given budget, weights, and optional overview.
    SaveStrategyConfig {
        hitting_budget_pct: u8,
        category_weights: crate::tui::onboarding::strategy_setup::CategoryWeights,
        strategy_overview: Option<String>,
    },
    /// Request LLM-assisted strategy configuration from a natural language description.
    ConfigureStrategyWithLlm(String),
    /// Navigate back to the previous onboarding step.
    GoBack,
    /// Advance to the next onboarding step.
    GoNext,
    /// Skip onboarding entirely and go straight to draft mode.
    Skip,
}

/// Updates pushed from the app orchestrator to the TUI during onboarding.
#[derive(Debug, Clone, PartialEq)]
pub enum OnboardingUpdate {
    /// Result of an API connection test.
    ConnectionTestResult {
        success: bool,
        message: String,
    },
    /// Sync onboarding state back to the TUI (e.g. on GoBack to LlmSetup).
    /// Carries the provider and model so the TUI can rebuild `LlmSetupState`.
    /// Optionally carries a masked API key string for the Settings screen
    /// placeholder (e.g. `sk-ant-*****6789`).
    ProgressSync {
        provider: Option<LlmProvider>,
        model: Option<String>,
        /// Masked API key for display in Settings. `None` means no key exists
        /// or the sync is from onboarding (where the user types the key fresh).
        api_key_mask: Option<String>,
    },
    /// A streamed token from the strategy LLM generation.
    StrategyLlmToken(String),
    /// Strategy LLM generation completed successfully with parsed config.
    StrategyLlmComplete {
        hitting_budget_pct: u8,
        category_weights: crate::tui::onboarding::strategy_setup::CategoryWeights,
        strategy_overview: String,
    },
    /// Strategy LLM generation failed.
    StrategyLlmError(String),
}

// ---------------------------------------------------------------------------
// Internal app messages (for mpsc channels, no serde needed)
// ---------------------------------------------------------------------------

/// Events produced by the LLM streaming client.
///
/// Each event carries a `generation` counter that identifies which LLM task
/// produced it. The app orchestrator increments the generation each time it
/// spawns a new LLM task, and discards events whose generation doesn't match
/// the current one. This prevents stale tokens from a cancelled task being
/// attributed to a newer analysis.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmEvent {
    /// A single token of streamed output.
    Token { text: String, generation: u64 },
    /// The LLM response is complete.
    Complete {
        full_text: String,
        input_tokens: u32,
        output_tokens: u32,
        /// The stop reason from the API (e.g. "end_turn" or "max_tokens").
        stop_reason: Option<String>,
        generation: u64,
    },
    /// An error occurred during LLM interaction.
    Error { message: String, generation: u64 },
}

impl LlmEvent {
    /// Extract the request ID (generation) from any event variant.
    pub fn request_id(&self) -> u64 {
        match self {
            LlmEvent::Token { generation, .. } => *generation,
            LlmEvent::Complete { generation, .. } => *generation,
            LlmEvent::Error { generation, .. } => *generation,
        }
    }
}

/// Commands sent from the TUI to the app orchestrator.
#[derive(Debug, Clone, PartialEq)]
pub enum UserCommand {
    /// Request a full keyframe (FULL_STATE_SYNC) from the extension.
    /// Sends a `REQUEST_KEYFRAME` message over the WebSocket so the
    /// extension responds with a complete state snapshot.
    RequestKeyframe,
    ManualPick {
        player_name: String,
        team_idx: usize,
        price: u32,
    },
    SwitchTab(TabId),
    Scroll {
        widget: WidgetId,
        direction: ScrollDirection,
    },
    /// User action during the onboarding wizard.
    OnboardingAction(OnboardingAction),
    /// Open the settings screen from draft mode.
    OpenSettings,
    /// Exit the settings screen and return to draft mode.
    ExitSettings,
    /// Save all dirty settings and then exit the settings screen.
    ///
    /// Carries optional save payloads for both LLM and Strategy tabs so
    /// the orchestrator can persist whichever (or both) have unsaved changes
    /// before transitioning back to draft mode.
    SaveAndExitSettings {
        /// LLM config to save, if any. (provider, model_id, api_key)
        llm: Option<(LlmProvider, String, Option<String>)>,
        /// Strategy config to save, if any. (budget_pct, weights, overview)
        strategy: Option<(
            u8,
            crate::tui::onboarding::strategy_setup::CategoryWeights,
            Option<String>,
        )>,
    },
    /// Switch which settings tab is active.
    SwitchSettingsTab(SettingsSection),
    Quit,
}

/// Generic LLM stream update, routed by request ID.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmStreamUpdate {
    /// A new token of streamed output.
    Token(String),
    /// Streaming is complete with the final text.
    Complete(String),
    /// An error occurred during streaming.
    Error(String),
}

/// Updates pushed from the app orchestrator to the TUI render loop.
#[derive(Debug, Clone)]
pub enum UiUpdate {
    /// Full state snapshot for a complete redraw.
    StateSnapshot(Box<AppSnapshot>),
    /// Generic LLM stream update, routed by request ID.
    LlmUpdate { request_id: u64, update: LlmStreamUpdate },
    /// Extension connection status changed.
    ConnectionStatus(ConnectionStatus),
    /// A new nomination is active. Carries the analysis request ID if one was started.
    NominationUpdate { info: Box<NominationInfo>, analysis_request_id: Option<u64> },
    /// Bid updated on the current nomination (same player, new bid amount).
    /// Unlike NominationUpdate, this does NOT clear accumulated LLM text.
    BidUpdate(Box<NominationInfo>),
    /// The current nomination was cleared (pick completed).
    NominationCleared,
    /// A new nomination plan stream is starting. Carries the plan request ID.
    PlanStarted { request_id: u64 },
    /// An update for the onboarding wizard (e.g. connection test result).
    OnboardingUpdate(OnboardingUpdate),
    /// The app mode has changed (e.g. onboarding -> draft).
    ModeChanged(AppMode),
    /// Full matchup state snapshot for the matchup screen.
    MatchupSnapshot(Box<MatchupSnapshot>),
}

/// WebSocket connection status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
}

/// LLM streaming status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmStatus {
    Idle,
    Streaming,
    Complete,
    Error,
}

/// Tab identifiers for the TUI layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabId {
    Analysis,
    Available,
    DraftLog,
    Teams,
}

/// Features that a tab may support.
///
/// Used with `TabId::supports()` to gate behavior by capability rather than
/// by checking specific tab variants. This keeps guard-check intent
/// self-documenting and centralizes per-tab capability declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabFeature {
    /// Text filter input (the `/` key to search/filter content).
    Filter,
    /// Position-based filter cycling (the `p` key).
    PositionFilter,
}

impl TabId {
    /// Returns whether this tab supports the given feature.
    pub fn supports(self, feature: TabFeature) -> bool {
        match feature {
            // Filter and PositionFilter are intentionally separate variants even though
            // they currently resolve to the same set of tabs. This allows future tabs to
            // support text filtering without position cycling (or vice versa).
            TabFeature::Filter => matches!(self, TabId::Available),
            TabFeature::PositionFilter => matches!(self, TabId::Available),
        }
    }
}

/// Widget identifiers for scroll targeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetId {
    MainPanel,
    Roster,
    Scarcity,
}

/// Scroll direction commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    PageUp,
    PageDown,
}

// ---------------------------------------------------------------------------
// Placeholder structs (filled in by later tasks)
// ---------------------------------------------------------------------------

/// Snapshot of the full application state, sent to the TUI for rendering.
///
/// Carries all recalculated data after picks are processed so the TUI
/// can update its ViewState in one shot.
#[derive(Debug, Clone)]
pub struct AppSnapshot {
    pub app_mode: AppMode,
    pub pick_count: usize,
    pub total_picks: usize,
    pub active_tab: Option<TabId>,
    /// Remaining player pool with updated valuations.
    pub available_players: Vec<PlayerValuation>,
    /// Recomputed positional scarcity indices.
    pub positional_scarcity: Vec<ScarcityEntry>,
    /// Chronological list of completed draft picks.
    pub draft_log: Vec<DraftPick>,
    /// User's roster slots (position + optional player).
    pub my_roster: Vec<RosterSlot>,
    /// Budget fields for the user's team.
    pub budget_spent: u32,
    pub budget_remaining: u32,
    pub salary_cap: u32,
    /// Current league-wide inflation rate.
    pub inflation_rate: f64,
    /// Maximum bid the user can make right now.
    pub max_bid: u32,
    /// Average dollars remaining per empty roster slot.
    pub avg_per_slot: f64,
    /// Hitting dollars spent by user's team.
    pub hitting_spent: u32,
    /// Hitting budget target (salary_cap * hitting_budget_fraction).
    pub hitting_target: u32,
    /// Pitching dollars spent by user's team.
    pub pitching_spent: u32,
    /// Pitching budget target (salary_cap * (1 - hitting_budget_fraction)).
    pub pitching_target: u32,
    /// Per-team summaries (name, budget, slots filled/total).
    pub team_snapshots: Vec<TeamSnapshot>,
    /// Whether the LLM client is configured (has a valid API key).
    /// Used by the status bar to show a "No LLM configured" hint.
    pub llm_configured: bool,
}

/// Lightweight summary of a team's draft state for the snapshot.
#[derive(Debug, Clone)]
pub struct TeamSnapshot {
    pub name: String,
    pub budget_remaining: u32,
    pub slots_filled: usize,
    pub total_slots: usize,
}

/// Info about the current active nomination.
#[derive(Debug, Clone, PartialEq)]
pub struct NominationInfo {
    pub player_name: String,
    pub position: String,
    pub nominated_by: String,
    pub current_bid: u32,
    pub current_bidder: Option<String>,
    pub time_remaining: Option<u32>,
    pub eligible_slots: Vec<u16>,
}

/// Instant analysis result for a nominated player.
#[derive(Debug, Clone, PartialEq)]
pub struct InstantAnalysis {
    pub player_name: String,
    pub dollar_value: f64,
    pub adjusted_value: f64,
    pub verdict: InstantVerdict,
}

/// Quick verdict for a nomination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantVerdict {
    StrongTarget,
    ConditionalTarget,
    Pass,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- TabFeature capability API --

    #[test]
    fn available_supports_filter() {
        assert!(TabId::Available.supports(TabFeature::Filter));
    }

    #[test]
    fn available_supports_position_filter() {
        assert!(TabId::Available.supports(TabFeature::PositionFilter));
    }

    #[test]
    fn non_available_tabs_do_not_support_filter() {
        for tab in [TabId::Analysis, TabId::DraftLog, TabId::Teams] {
            assert!(
                !tab.supports(TabFeature::Filter),
                "{:?} should not support Filter",
                tab
            );
            assert!(
                !tab.supports(TabFeature::PositionFilter),
                "{:?} should not support PositionFilter",
                tab
            );
        }
    }

    // -- JSON round-trip for all ExtensionMessage variants --

    #[test]
    fn round_trip_extension_connected() {
        let msg = ExtensionMessage::ExtensionConnected {
            payload: ExtensionConnectedPayload {
                platform: "firefox".to_string(),
                extension_version: "1.0.0".to_string(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ExtensionMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn round_trip_state_update() {
        let msg = ExtensionMessage::StateUpdate {
            timestamp: 1700000000,
            payload: StateUpdatePayload {
                picks: vec![PickData {
                    pick_number: 1,
                    team_id: "team_3".to_string(),
                    team_name: "Vorticists".to_string(),
                    player_id: "12345".to_string(),
                    player_name: "Shohei Ohtani".to_string(),
                    position: "DH".to_string(),
                    price: 62,
                    eligible_slots: vec![11, 12, 16, 17],
                    assigned_slot: None,
                }],
                current_nomination: Some(NominationData {
                    player_id: "67890".to_string(),
                    player_name: "Aaron Judge".to_string(),
                    position: "OF".to_string(),
                    nominated_by: "Team Alpha".to_string(),
                    current_bid: 55,
                    current_bidder: Some("Team Beta".to_string()),
                    time_remaining: Some(15),
                    eligible_slots: vec![5, 8, 9, 10, 11, 12, 16, 17],
                }),
                my_team_id: Some("team_7".to_string()),
                teams: vec![TeamBudgetData {
                    team_id: Some("3".to_string()),
                    team_name: "Vorticists".to_string(),
                    budget: 198,
                }],
                pick_count: None,
                total_picks: None,
                draft_id: Some("espn_12345_2026".to_string()),
                source: Some("dom_scraper".to_string()),
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ExtensionMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn round_trip_heartbeat() {
        let msg = ExtensionMessage::ExtensionHeartbeat {
            payload: HeartbeatPayload {
                timestamp: 1700000001,
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ExtensionMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
    }

    // -- Deserialize from hand-written JSON (camelCase -> snake_case) --

    #[test]
    fn deserialize_extension_connected_camel_case() {
        let json = r#"{
            "type": "EXTENSION_CONNECTED",
            "payload": {
                "platform": "firefox",
                "extensionVersion": "0.2.1"
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::ExtensionConnected { payload } => {
                assert_eq!(payload.platform, "firefox");
                assert_eq!(payload.extension_version, "0.2.1");
            }
            _ => panic!("expected ExtensionConnected variant"),
        }
    }

    #[test]
    fn deserialize_heartbeat_camel_case() {
        let json = r#"{
            "type": "EXTENSION_HEARTBEAT",
            "payload": {
                "timestamp": 9999
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::ExtensionHeartbeat { payload } => {
                assert_eq!(payload.timestamp, 9999);
            }
            _ => panic!("expected ExtensionHeartbeat variant"),
        }
    }

    #[test]
    fn deserialize_state_update_with_picks_and_nomination() {
        let json = r#"{
            "type": "STATE_UPDATE",
            "timestamp": 1700000005,
            "payload": {
                "picks": [
                    {
                        "pickNumber": 1,
                        "teamId": "team_3",
                        "teamName": "Vorticists",
                        "playerId": "12345",
                        "playerName": "Shohei Ohtani",
                        "position": "DH",
                        "price": 62
                    },
                    {
                        "pickNumber": 2,
                        "teamId": "team_5",
                        "teamName": "Sluggers",
                        "playerId": "54321",
                        "playerName": "Mookie Betts",
                        "position": "SS",
                        "price": 48
                    }
                ],
                "currentNomination": {
                    "playerId": "67890",
                    "playerName": "Aaron Judge",
                    "position": "OF",
                    "nominatedBy": "Team Alpha",
                    "currentBid": 55,
                    "currentBidder": "Team Beta",
                    "timeRemaining": 15
                },
                "myTeamId": "team_7",
                "source": "dom_scraper"
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::StateUpdate { timestamp, payload } => {
                assert_eq!(timestamp, 1700000005);
                assert_eq!(payload.picks.len(), 2);
                assert_eq!(payload.picks[0].pick_number, 1);
                assert_eq!(payload.picks[0].player_name, "Shohei Ohtani");
                assert_eq!(payload.picks[1].pick_number, 2);
                assert_eq!(payload.picks[1].player_name, "Mookie Betts");
                let nom = payload.current_nomination.unwrap();
                assert_eq!(nom.player_name, "Aaron Judge");
                assert_eq!(nom.current_bid, 55);
                assert_eq!(nom.time_remaining, Some(15));
                assert_eq!(payload.my_team_id, Some("team_7".to_string()));
                assert_eq!(payload.source, Some("dom_scraper".to_string()));
            }
            _ => panic!("expected StateUpdate variant"),
        }
    }

    #[test]
    fn deserialize_state_update_no_nomination() {
        let json = r#"{
            "type": "STATE_UPDATE",
            "timestamp": 1700000010,
            "payload": {
                "picks": [],
                "currentNomination": null,
                "myTeamId": "team_1",
                "source": "react_state"
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::StateUpdate { payload, .. } => {
                assert!(payload.picks.is_empty());
                assert!(payload.current_nomination.is_none());
            }
            _ => panic!("expected StateUpdate variant"),
        }
    }

    #[test]
    fn deserialize_state_update_omitted_nomination() {
        // With #[serde(default)], omitting currentNomination entirely should work
        let json = r#"{
            "type": "STATE_UPDATE",
            "timestamp": 1700000010,
            "payload": {
                "picks": [],
                "myTeamId": "team_1",
                "source": "react_state"
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::StateUpdate { payload, .. } => {
                assert!(payload.current_nomination.is_none());
            }
            _ => panic!("expected StateUpdate variant"),
        }
    }

    // -- Malformed JSON returns error (does not panic) --

    #[test]
    fn malformed_json_returns_error() {
        let bad_json = r#"{ this is not valid json }"#;
        let result = serde_json::from_str::<ExtensionMessage>(bad_json);
        assert!(result.is_err());
    }

    #[test]
    fn missing_type_field_returns_error() {
        let json = r#"{ "payload": { "timestamp": 123 } }"#;
        let result = serde_json::from_str::<ExtensionMessage>(json);
        assert!(result.is_err());
    }

    #[test]
    fn unknown_type_variant_returns_error() {
        let json = r#"{ "type": "UNKNOWN_TYPE", "payload": {} }"#;
        let result = serde_json::from_str::<ExtensionMessage>(json);
        assert!(result.is_err());
    }

    #[test]
    fn missing_required_payload_field_returns_error() {
        // Missing extensionVersion in EXTENSION_CONNECTED payload
        let json = r#"{
            "type": "EXTENSION_CONNECTED",
            "payload": {
                "platform": "firefox"
            }
        }"#;
        let result = serde_json::from_str::<ExtensionMessage>(json);
        assert!(result.is_err());
    }

    // -- camelCase serialization check --

    #[test]
    fn serialized_json_uses_camel_case() {
        let msg = ExtensionMessage::StateUpdate {
            timestamp: 100,
            payload: StateUpdatePayload {
                picks: vec![PickData {
                    pick_number: 1,
                    team_id: "team_2".to_string(),
                    team_name: "Test".to_string(),
                    player_id: "p3".to_string(),
                    player_name: "Player".to_string(),
                    position: "C".to_string(),
                    price: 10,
                    eligible_slots: vec![],
                    assigned_slot: None,
                }],
                current_nomination: None,
                my_team_id: Some("team_5".to_string()),
                teams: vec![],
                pick_count: None,
                total_picks: None,
                draft_id: Some("espn_42_2026".to_string()),
                source: Some("test".to_string()),
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        // Verify camelCase keys are present
        assert!(json.contains("pickNumber"));
        assert!(json.contains("teamId"));
        assert!(json.contains("teamName"));
        assert!(json.contains("playerId"));
        assert!(json.contains("playerName"));
        assert!(json.contains("currentNomination"));
        assert!(json.contains("myTeamId"));
        assert!(json.contains("eligibleSlots"));
        assert!(json.contains("draftId"));
        // Verify snake_case keys are NOT present
        assert!(!json.contains("pick_number"));
        assert!(!json.contains("player_name"));
        assert!(!json.contains("eligible_slots"));
        assert!(!json.contains("draft_id"));
    }

    // -- AppSnapshot construction --

    #[test]
    fn app_snapshot_construction() {
        let snap = AppSnapshot {
            app_mode: AppMode::Draft,
            pick_count: 0,
            total_picks: 0,
            active_tab: None,
            available_players: vec![],
            positional_scarcity: vec![],
            draft_log: vec![],
            my_roster: vec![],
            budget_spent: 0,
            budget_remaining: 260,
            salary_cap: 260,
            inflation_rate: 1.0,
            max_bid: 0,
            avg_per_slot: 0.0,
            hitting_spent: 0,
            hitting_target: 0,
            pitching_spent: 0,
            pitching_target: 0,
            team_snapshots: vec![],
            llm_configured: true,
        };
        assert_eq!(snap.app_mode, AppMode::Draft);
        assert_eq!(snap.pick_count, 0);
        assert_eq!(snap.total_picks, 0);
        assert_eq!(snap.active_tab, None);
        assert!(snap.available_players.is_empty());
        assert!(snap.team_snapshots.is_empty());
    }

    // -- eligible_slots backward compatibility --

    #[test]
    fn eligible_slots_defaults_to_empty_when_omitted() {
        // JSON without eligibleSlots fields should still deserialize
        // thanks to #[serde(default)]
        let json = r#"{
            "type": "STATE_UPDATE",
            "timestamp": 1700000000,
            "payload": {
                "picks": [
                    {
                        "pickNumber": 1,
                        "teamId": "team_1",
                        "teamName": "Team 1",
                        "playerId": "p1",
                        "playerName": "Player One",
                        "position": "SP",
                        "price": 30
                    }
                ],
                "currentNomination": {
                    "playerId": "p2",
                    "playerName": "Player Two",
                    "position": "1B",
                    "nominatedBy": "Team 2",
                    "currentBid": 5,
                    "currentBidder": null,
                    "timeRemaining": 30
                },
                "myTeamId": "team_1",
                "source": "test"
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::StateUpdate { payload, .. } => {
                assert!(payload.picks[0].eligible_slots.is_empty());
                let nom = payload.current_nomination.unwrap();
                assert!(nom.eligible_slots.is_empty());
            }
            _ => panic!("expected StateUpdate variant"),
        }
    }

    // -- draftId backward compatibility --

    #[test]
    fn draft_id_defaults_to_none_when_omitted() {
        // JSON without draftId should still deserialize thanks to #[serde(default)]
        let json = r#"{
            "type": "STATE_UPDATE",
            "timestamp": 1700000000,
            "payload": {
                "picks": [],
                "currentNomination": null,
                "myTeamId": "team_1",
                "source": "test"
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::StateUpdate { payload, .. } => {
                assert!(payload.draft_id.is_none());
            }
            _ => panic!("expected StateUpdate variant"),
        }
    }

    #[test]
    fn draft_id_deserialized_from_camel_case() {
        let json = r#"{
            "type": "STATE_UPDATE",
            "timestamp": 1700000000,
            "payload": {
                "picks": [],
                "currentNomination": null,
                "myTeamId": "team_1",
                "draftId": "espn_12345_2026",
                "source": "test"
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::StateUpdate { payload, .. } => {
                assert_eq!(payload.draft_id, Some("espn_12345_2026".to_string()));
            }
            _ => panic!("expected StateUpdate variant"),
        }
    }

    #[test]
    fn draft_id_null_deserialized_as_none() {
        let json = r#"{
            "type": "STATE_UPDATE",
            "timestamp": 1700000000,
            "payload": {
                "picks": [],
                "currentNomination": null,
                "myTeamId": "team_1",
                "draftId": null,
                "source": "test"
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::StateUpdate { payload, .. } => {
                assert!(payload.draft_id.is_none());
            }
            _ => panic!("expected StateUpdate variant"),
        }
    }

    #[test]
    fn eligible_slots_round_trip_with_values() {
        let pick_data = PickData {
            pick_number: 1,
            team_id: "team_1".into(),
            team_name: "Team 1".into(),
            player_id: "p1".into(),
            player_name: "Mookie Betts".into(),
            position: "SS".into(),
            price: 40,
            eligible_slots: vec![4, 2, 5, 8, 9, 10, 11, 12, 16, 17],
            assigned_slot: None,
        };
        let json = serde_json::to_string(&pick_data).unwrap();
        let parsed: PickData = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.eligible_slots,
            vec![4, 2, 5, 8, 9, 10, 11, 12, 16, 17]
        );
    }

    // -- FULL_STATE_SYNC variant --

    #[test]
    fn round_trip_full_state_sync() {
        let msg = ExtensionMessage::FullStateSync {
            timestamp: 1700000100,
            payload: StateUpdatePayload {
                picks: vec![
                    PickData {
                        pick_number: 1,
                        team_id: "team_1".to_string(),
                        team_name: "Vorticists".to_string(),
                        player_id: "11111".to_string(),
                        player_name: "Mike Trout".to_string(),
                        position: "CF".to_string(),
                        price: 50,
                        eligible_slots: vec![],
                        assigned_slot: None,
                    },
                    PickData {
                        pick_number: 2,
                        team_id: "team_2".to_string(),
                        team_name: "Sluggers".to_string(),
                        player_id: "22222".to_string(),
                        player_name: "Shohei Ohtani".to_string(),
                        position: "SP".to_string(),
                        price: 65,
                        eligible_slots: vec![11, 12, 16, 17],
                        assigned_slot: None,
                    },
                ],
                current_nomination: None,
                my_team_id: Some("team_1".to_string()),
                teams: vec![
                    TeamBudgetData {
                        team_id: Some("1".to_string()),
                        team_name: "Vorticists".to_string(),
                        budget: 210,
                    },
                    TeamBudgetData {
                        team_id: Some("2".to_string()),
                        team_name: "Sluggers".to_string(),
                        budget: 195,
                    },
                ],
                pick_count: Some(2),
                total_picks: Some(260),
                draft_id: Some("espn_12345_2026".to_string()),
                source: Some("dom_scrape".to_string()),
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ExtensionMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
        // Verify the type field is serialized as FULL_STATE_SYNC
        assert!(json.contains("\"FULL_STATE_SYNC\""));
    }

    #[test]
    fn deserialize_full_state_sync_camel_case() {
        let json = r#"{
            "type": "FULL_STATE_SYNC",
            "timestamp": 1700000200,
            "payload": {
                "picks": [
                    {
                        "pickNumber": 1,
                        "teamId": "team_3",
                        "teamName": "Vorticists",
                        "playerId": "99999",
                        "playerName": "Aaron Judge",
                        "position": "OF",
                        "price": 55
                    }
                ],
                "currentNomination": null,
                "myTeamId": "team_3",
                "teams": [],
                "pickCount": 1,
                "totalPicks": 260,
                "draftId": "espn_42_2026",
                "source": "dom_scrape"
            }
        }"#;
        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::FullStateSync { timestamp, payload } => {
                assert_eq!(timestamp, 1700000200);
                assert_eq!(payload.picks.len(), 1);
                assert_eq!(payload.picks[0].player_name, "Aaron Judge");
                assert_eq!(payload.picks[0].price, 55);
                assert_eq!(payload.my_team_id, Some("team_3".to_string()));
                assert_eq!(payload.pick_count, Some(1));
                assert_eq!(payload.draft_id, Some("espn_42_2026".to_string()));
            }
            _ => panic!("expected FullStateSync variant"),
        }
    }

    #[test]
    fn full_state_sync_is_distinct_from_state_update() {
        // Ensure FULL_STATE_SYNC and STATE_UPDATE do not deserialize interchangeably
        let full_sync_json = r#"{
            "type": "FULL_STATE_SYNC",
            "timestamp": 123,
            "payload": { "picks": [], "myTeamId": null, "source": "test" }
        }"#;
        let state_update_json = r#"{
            "type": "STATE_UPDATE",
            "timestamp": 123,
            "payload": { "picks": [], "myTeamId": null, "source": "test" }
        }"#;

        let full_sync: ExtensionMessage = serde_json::from_str(full_sync_json).unwrap();
        let state_update: ExtensionMessage = serde_json::from_str(state_update_json).unwrap();

        assert!(matches!(full_sync, ExtensionMessage::FullStateSync { .. }));
        assert!(matches!(state_update, ExtensionMessage::StateUpdate { .. }));
    }

    // -- AppMode / SettingsSection / OnboardingAction --

    #[test]
    fn app_mode_equality() {
        use crate::onboarding::OnboardingStep;

        assert_eq!(AppMode::Draft, AppMode::Draft);
        assert_eq!(
            AppMode::Onboarding(OnboardingStep::LlmSetup),
            AppMode::Onboarding(OnboardingStep::LlmSetup)
        );
        assert_eq!(
            AppMode::Settings(SettingsSection::LlmConfig),
            AppMode::Settings(SettingsSection::LlmConfig)
        );
        assert_ne!(AppMode::Draft, AppMode::Onboarding(OnboardingStep::LlmSetup));
        assert_ne!(
            AppMode::Settings(SettingsSection::LlmConfig),
            AppMode::Settings(SettingsSection::StrategyConfig)
        );
    }

    #[test]
    fn settings_section_equality() {
        assert_eq!(SettingsSection::LlmConfig, SettingsSection::LlmConfig);
        assert_eq!(SettingsSection::StrategyConfig, SettingsSection::StrategyConfig);
        assert_ne!(SettingsSection::LlmConfig, SettingsSection::StrategyConfig);
    }

    #[test]
    fn onboarding_action_variants_constructable() {
        use crate::llm::provider::LlmProvider;

        // Ensure all OnboardingAction variants can be constructed
        let _set_provider = OnboardingAction::SetProvider(LlmProvider::Anthropic);
        let _set_model = OnboardingAction::SetModel("claude-sonnet-4-6".to_string());
        let _set_key = OnboardingAction::SetApiKey("sk-test".to_string());
        let _test_conn = OnboardingAction::TestConnection;
        let _save_strategy = OnboardingAction::SaveStrategyConfig {
            hitting_budget_pct: 65,
            category_weights: crate::tui::onboarding::strategy_setup::CategoryWeights::default(),
            strategy_overview: Some("Test overview".to_string()),
        };
        let _configure_llm = OnboardingAction::ConfigureStrategyWithLlm("punt saves".to_string());
        let _save_llm = OnboardingAction::SaveLlmConfig {
            provider: LlmProvider::Anthropic,
            model_id: "claude-sonnet-4-6".to_string(),
            api_key: Some("sk-test".to_string()),
        };
        let _go_back = OnboardingAction::GoBack;
        let _go_next = OnboardingAction::GoNext;
        let _skip = OnboardingAction::Skip;
    }

    #[test]
    fn user_command_onboarding_action_variant() {
        let cmd = UserCommand::OnboardingAction(OnboardingAction::GoNext);
        assert!(matches!(cmd, UserCommand::OnboardingAction(OnboardingAction::GoNext)));
    }

    #[test]
    fn ui_update_mode_changed_variant() {
        let update = UiUpdate::ModeChanged(AppMode::Draft);
        assert!(matches!(update, UiUpdate::ModeChanged(AppMode::Draft)));
    }

    #[test]
    fn ui_update_onboarding_update_variant() {
        let update = UiUpdate::OnboardingUpdate(OnboardingUpdate::ConnectionTestResult {
            success: true,
            message: "Connected!".to_string(),
        });
        assert!(matches!(update, UiUpdate::OnboardingUpdate(OnboardingUpdate::ConnectionTestResult { .. })));
    }

    #[test]
    fn app_snapshot_carries_app_mode() {
        use crate::onboarding::OnboardingStep;

        let snap = AppSnapshot {
            app_mode: AppMode::Onboarding(OnboardingStep::StrategySetup),
            pick_count: 0,
            total_picks: 0,
            active_tab: None,
            available_players: vec![],
            positional_scarcity: vec![],
            draft_log: vec![],
            my_roster: vec![],
            budget_spent: 0,
            budget_remaining: 260,
            salary_cap: 260,
            inflation_rate: 1.0,
            max_bid: 0,
            avg_per_slot: 0.0,
            hitting_spent: 0,
            hitting_target: 0,
            pitching_spent: 0,
            pitching_target: 0,
            team_snapshots: vec![],
            llm_configured: false,
        };
        assert_eq!(snap.app_mode, AppMode::Onboarding(OnboardingStep::StrategySetup));
    }

    // -- ProjectionData From impls --

    #[test]
    fn from_espn_batting_projection_populates_all_keys() {
        let proj = EspnBattingProjection {
            pa: 600,
            ab: 530,
            h: 150,
            hr: 30,
            r: 90,
            rbi: 85,
            bb: 60,
            sb: 10,
            avg: 0.283,
        };
        let pd = ProjectionData::from(&proj);
        assert_eq!(pd.get("pa"), Some(600.0));
        assert_eq!(pd.get("ab"), Some(530.0));
        assert_eq!(pd.get("h"), Some(150.0));
        assert_eq!(pd.get("hr"), Some(30.0));
        assert_eq!(pd.get("r"), Some(90.0));
        assert_eq!(pd.get("rbi"), Some(85.0));
        assert_eq!(pd.get("bb"), Some(60.0));
        assert_eq!(pd.get("sb"), Some(10.0));
        assert_eq!(pd.get("avg"), Some(0.283));
        // Pitching keys not present
        assert_eq!(pd.get_or_zero("ip"), 0.0);
    }

    #[test]
    fn from_espn_pitching_projection_populates_all_keys_with_k9() {
        let proj = EspnPitchingProjection {
            ip: 180.0,
            k: 200,
            w: 14,
            sv: 0,
            hd: 0,
            era: 3.20,
            whip: 1.10,
            g: 30,
            gs: 30,
        };
        let pd = ProjectionData::from(&proj);
        assert_eq!(pd.get("ip"), Some(180.0));
        assert_eq!(pd.get("k"), Some(200.0));
        assert_eq!(pd.get("w"), Some(14.0));
        assert_eq!(pd.get("sv"), Some(0.0));
        assert_eq!(pd.get("hd"), Some(0.0));
        assert_eq!(pd.get("era"), Some(3.20));
        assert_eq!(pd.get("whip"), Some(1.10));
        assert_eq!(pd.get("g"), Some(30.0));
        assert_eq!(pd.get("gs"), Some(30.0));
        // k9 = 200 * 9 / 180 = 10.0
        let k9 = pd.get("k9").expect("k9 should be present");
        assert!((k9 - 10.0).abs() < 1e-10);
    }

    #[test]
    fn from_espn_pitching_projection_zero_ip_omits_k9() {
        let proj = EspnPitchingProjection {
            ip: 0.0,
            k: 0,
            w: 0,
            sv: 0,
            hd: 0,
            era: 0.0,
            whip: 0.0,
            g: 0,
            gs: 0,
        };
        let pd = ProjectionData::from(&proj);
        assert_eq!(pd.get("k9"), None);
        assert_eq!(pd.get_or_zero("k9"), 0.0);
    }

    // -- MatchupState deserialization --

    #[test]
    fn deserialize_matchup_state_payload() {
        let json = r#"{
            "type": "MATCHUP_STATE",
            "timestamp": 1711500000,
            "payload": {
                "matchupPeriod": 1,
                "startDate": "2026-03-25",
                "endDate": "2026-04-05",
                "selectedDay": "2026-03-26",
                "myTeam": {
                    "name": "Bob Dole Experience",
                    "record": "0-0-0",
                    "matchupScore": "2-3-7"
                },
                "oppTeam": {
                    "name": "Certified! Smokified!",
                    "record": "0-0-0",
                    "matchupScore": "3-2-7"
                },
                "categories": [
                    { "statId": 20, "abbrev": "R", "myValue": 5.0, "oppValue": 3.0, "lowerIsBetter": false },
                    { "statId": 5, "abbrev": "HR", "myValue": 2.0, "oppValue": 4.0, "lowerIsBetter": false },
                    { "statId": 47, "abbrev": "ERA", "myValue": 3.45, "oppValue": 4.12, "lowerIsBetter": true }
                ],
                "batting": {
                    "headers": ["AB", "H", "R", "HR", "RBI", "BB", "SB", "AVG"],
                    "players": [
                        {
                            "slot": "C",
                            "name": "Ben Rice",
                            "team": "NYY",
                            "positions": ["1B", "C", "DH"],
                            "opponent": "@BOS",
                            "status": null,
                            "stats": [4.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.250]
                        }
                    ],
                    "totals": [29.0, 8.0, 5.0, 2.0, 6.0, 5.0, 1.0, 0.276]
                },
                "pitching": {
                    "headers": ["IP", "H", "ER", "BB", "K", "W", "SV", "HD"],
                    "players": [],
                    "totals": null
                }
            }
        }"#;

        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::MatchupState { timestamp, payload } => {
                assert_eq!(timestamp, 1711500000);
                assert_eq!(payload.matchup_period, 1);
                assert_eq!(payload.start_date, "2026-03-25");
                assert_eq!(payload.end_date, "2026-04-05");
                assert_eq!(payload.selected_day, "2026-03-26");
                assert_eq!(payload.my_team.name, "Bob Dole Experience");
                assert_eq!(payload.my_team.record, "0-0-0");
                assert_eq!(payload.my_team.matchup_score, "2-3-7");
                assert_eq!(payload.opp_team.name, "Certified! Smokified!");
                assert_eq!(payload.opp_team.matchup_score, "3-2-7");
                assert_eq!(payload.categories.len(), 3);
                assert_eq!(payload.categories[0].stat_id, 20);
                assert_eq!(payload.categories[0].abbrev, "R");
                assert_eq!(payload.categories[0].my_value, Some(5.0));
                assert_eq!(payload.categories[0].opp_value, Some(3.0));
                assert!(!payload.categories[0].lower_is_better);
                assert!(payload.categories[2].lower_is_better);
                assert_eq!(payload.batting.headers.len(), 8);
                assert_eq!(payload.batting.players.len(), 1);
                assert_eq!(payload.batting.players[0].name, "Ben Rice");
                assert_eq!(payload.batting.players[0].positions, vec!["1B", "C", "DH"]);
                assert_eq!(payload.batting.players[0].opponent, Some("@BOS".to_string()));
                assert_eq!(payload.batting.players[0].status, None);
                assert_eq!(payload.batting.totals.as_ref().unwrap().len(), 8);
                assert_eq!(payload.pitching.players.len(), 0);
                assert!(payload.pitching.totals.is_none());
            }
            other => panic!("Expected MatchupState, got {:?}", other),
        }
    }

    /// Regression: the matchup content script emits camelCase JSON keys and
    /// `null` for rate-stat category values that ESPN renders as `"--"`
    /// (AVG/ERA/WHIP before any denominator exists). This test pins the exact
    /// shape the extension sends so future drift is caught at the unit-test
    /// level rather than by end-to-end failures.
    #[test]
    fn deserialize_extension_matchup_payload_shape() {
        // Mirrors the exact JSON the background script relays over the
        // WebSocket (`source` is stripped by background-core.js before relay).
        let json = r#"{
            "type": "MATCHUP_STATE",
            "timestamp": 1711500000,
            "payload": {
                "matchupPeriod": 1,
                "startDate": "2026-03-25",
                "endDate": "2026-04-05",
                "selectedDay": "2026-03-26",
                "myTeam": {
                    "name": "Bob Dole Experience",
                    "record": "0-0-0",
                    "matchupScore": "0-0-12"
                },
                "oppTeam": {
                    "name": "Certified! Smokified!",
                    "record": "0-0-0",
                    "matchupScore": "0-0-12"
                },
                "categories": [
                    { "statId": 20, "abbrev": "R", "myValue": 0, "oppValue": 0, "lowerIsBetter": false },
                    { "statId": 2, "abbrev": "AVG", "myValue": null, "oppValue": null, "lowerIsBetter": false },
                    { "statId": 47, "abbrev": "ERA", "myValue": null, "oppValue": 3.00, "lowerIsBetter": true }
                ],
                "batting": { "headers": ["AB", "H"], "players": [], "totals": null },
                "pitching": { "headers": ["IP", "K"], "players": [], "totals": null }
            }
        }"#;

        let msg: ExtensionMessage = serde_json::from_str(json).unwrap();
        match msg {
            ExtensionMessage::MatchupState { payload, .. } => {
                assert_eq!(payload.matchup_period, 1);
                assert_eq!(payload.categories[0].my_value, Some(0.0));
                assert_eq!(payload.categories[1].my_value, None);
                assert_eq!(payload.categories[1].opp_value, None);
                assert_eq!(payload.categories[2].my_value, None);
                assert_eq!(payload.categories[2].opp_value, Some(3.00));
            }
            other => panic!("Expected MatchupState, got {:?}", other),
        }
    }

    #[test]
    fn round_trip_matchup_state() {
        let msg = ExtensionMessage::MatchupState {
            timestamp: 1711500000,
            payload: MatchupStatePayload {
                matchup_period: 2,
                start_date: "2026-04-06".to_string(),
                end_date: "2026-04-12".to_string(),
                selected_day: "2026-04-07".to_string(),
                my_team: MatchupTeamPayload {
                    name: "Team A".to_string(),
                    record: "1-0-0".to_string(),
                    matchup_score: "7-5-0".to_string(),
                },
                opp_team: MatchupTeamPayload {
                    name: "Team B".to_string(),
                    record: "0-1-0".to_string(),
                    matchup_score: "5-7-0".to_string(),
                },
                categories: vec![MatchupCategoryPayload {
                    stat_id: 20,
                    abbrev: "R".to_string(),
                    my_value: Some(10.0),
                    opp_value: Some(8.0),
                    lower_is_better: false,
                }],
                batting: MatchupSectionPayload {
                    headers: vec!["AB".to_string(), "H".to_string()],
                    players: vec![],
                    totals: None,
                },
                pitching: MatchupSectionPayload {
                    headers: vec!["IP".to_string(), "K".to_string()],
                    players: vec![],
                    totals: None,
                },
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ExtensionMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
    }
}
