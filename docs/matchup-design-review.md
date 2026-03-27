# Matchup Page Design Review

Review of `main/draft-assistant/design/matchup-page.md` — three design gaps identified around reusability and hardcoding.

---

## Q1: Polling Logic Standardization

### Question

> Are we standardizing the polling logic in this? That should clearly be a part of this since both pages are using the keyframe/iframe structure / pattern we adhered to in the draft. This will be constant, so we should include something EXPLICIT as part of this to genericize it, reuse it across draft+matchup pages, and make it reusable for more components in the future as a central component.

### Answer

**The design doc does NOT address the polling pattern.** The only reference is a single hand-wave sentence: "Similar to the draft page's `compute_state_diff`, the extension should detect changes... Send differential updates when possible, full snapshots periodically." It says nothing about MutationObserver setup, debounce timing, periodic fallback polling, keyframe intervals, fingerprint deduplication, container discovery, or init orchestration.

#### Current Draft Page Architecture (espn.js)

The draft content script uses a 3-layer polling architecture:

1. **MutationObserver** — Watches the draft container DOM for changes (childList, subtree, characterData, class/data-testid attributes). Debounced at 250ms. Triggers `requestStateExtraction()` which calls `scrapeDom()` -> `handleStateUpdate()`.
2. **Periodic Polling Fallback** — `setInterval` at 1000ms, calls `requestStateExtraction()` in case MutationObserver misses virtual DOM updates.
3. **Periodic Keyframe** — `setInterval` at 10000ms, sends a `FULL_STATE_SYNC` (complete state snapshot including expensive data like draft board grid and pick history) whenever the fingerprint has changed since last send.

Supporting infrastructure: fingerprint deduplication, container polling on init (500ms for up to 10s, fallback to document.body), `STATE_UPDATE` vs `FULL_STATE_SYNC` message types, background script `REQUEST_FULL_STATE_SYNC` on reconnect.

#### What a Genericized `PageObserver` Component Would Look Like

The current `espn.js` has cleanly separable generic vs page-specific layers:

**Generic (shared infrastructure):**
- Container discovery (poll for target element by selector, fallback to body)
- MutationObserver attachment + debounce
- Periodic polling fallback
- Periodic keyframe sync
- Fingerprint dedup mechanism (compare, update on send)
- Message forwarding to background script
- `REQUEST_FULL_STATE_SYNC` handler
- Init orchestration
- Logging with configurable prefix

**Page-specific (provided per page):**
- Container selector string
- `scrapeState()` function
- `scrapeExpensiveData()` function
- `computeFingerprint(state)` function
- `buildPayload(state, extras?)` function
- Message type strings (e.g. `STATE_UPDATE` / `FULL_STATE_SYNC`)
- MutationObserver attribute filter list
- One-time fetches (e.g. ESPN projections for draft)

**Proposed factory interface:**

```js
// extension/lib/page-observer.js  (shared module)

function createPageObserver(config) {
  // config = {
  //   logPrefix: string,
  //   containerSelector: string,
  //   attributeFilter: string[],          // default: ['class', 'data-testid']
  //   timings: {
  //     mutationDebounceMs: number,        // default: 250
  //     containerPollMs: number,           // default: 500
  //     containerPollTimeoutMs: number,    // default: 10000
  //     fallbackPollMs: number,            // default: 1000
  //     keyframeMs: number,               // default: 10000
  //   },
  //   scrapeState: () => Object|null,
  //   scrapeExpensiveData: () => Object|null,
  //   computeFingerprint: (state) => string,
  //   buildPayload: (state, extras?) => Object,
  //   stateUpdateType: string,            // e.g. 'STATE_UPDATE'
  //   fullSyncType: string,               // e.g. 'FULL_STATE_SYNC'
  //   onFullSync: () => void,             // optional hook for re-sending cached data
  // }

  // Returns: { init(), destroy() }
}
```

Draft content script becomes:
```js
const observer = createPageObserver({
  logPrefix: '[WyndhamDraftSync]',
  containerSelector: SELECTORS.draftContainer,
  scrapeState: scrapeDom,
  scrapeExpensiveData: () => ({
    pickHistory: scrapePickHistory(),
    draftBoard: scrapeDraftBoard(),
  }),
  computeFingerprint,
  buildPayload: buildStatePayload,
  stateUpdateType: 'STATE_UPDATE',
  fullSyncType: 'FULL_STATE_SYNC',
  onFullSync: () => sendProjectionsToBackend(),
});
observer.init();
fetchEspnProjections();
```

Matchup content script becomes:
```js
const observer = createPageObserver({
  logPrefix: '[WyndhamMatchupSync]',
  containerSelector: '.boxscore-container',
  scrapeState: scrapeMatchupState,
  scrapeExpensiveData: scrapeFullMatchupData,
  computeFingerprint: computeMatchupFingerprint,
  buildPayload: buildMatchupPayload,
  stateUpdateType: 'MATCHUP_STATE_UPDATE',
  fullSyncType: 'MATCHUP_FULL_STATE_SYNC',
});
observer.init();
```

#### What the Design Doc Needs to Add

1. **Name this as a first-class shared component** (e.g. "PageObserver") and state it is extracted from the existing draft content script.
2. **Define the interface** — list the config parameters and what each page provides.
3. **Specify matchup-specific timing constants** — matchup updates are much slower-cadence than real-time bidding. The doc should explicitly state whether the same timings apply or recommend different values and why.
4. **Define fingerprint strategy** for matchup state (category scores, selected day, player count, totals).
5. **Define both message types** — `MATCHUP_STATE_UPDATE` and `MATCHUP_FULL_STATE_SYNC`. The current doc only shows `matchup_state`.
6. **Address background script changes** — URL validation currently only accepts `/baseball/draft`; needs `/baseball/boxscore`.
7. **Address content script loading strategy** — how does the extension decide which scraper to activate? URL-based dispatch at a shared entry point, or separate registrations?
8. **Future extensibility** — state this pattern is designed for N pages (draft, matchup, free agent, trade, etc.).

---

## Q2: Position Hardcoding

### Question

> Are we hardcoding positions here? Look at the project to understand how the project handles positions; we are trying to be broadly supportive of all positions ESPN supports but configuring it per my league settings for now. So we need this to be comprehensive and not exclude other stats. Include the firehose.

### Answer

**Yes, positions are hardcoded as strings throughout the design doc, bypassing the project's existing typed `Position` enum.**

#### How Positions Work in the Existing Draft Pipeline

The draft page is fully typed:
- **Extension** converts position strings -> ESPN slot IDs (u16) via `espnSlotIdFromPositionStr()`
- **WebSocket** sends numeric `eligible_slots: [u16]` and `assigned_slot: Option<u16>`
- **Backend** converts to `Position` enum via `position_from_espn_slot()` (in `src/draft/pick.rs`)
- **TUI** renders via `slot.position.display_str()`, uses `is_combo_slot()`, `accepted_positions()` for combo-aware highlighting

No raw strings are used internally. The only string representation is at the display layer via `display_str()`.

The project's `Position` enum (`src/draft/pick.rs`) covers all 18 ESPN-supported slots:

```rust
pub enum Position {
    Catcher, FirstBase, SecondBase, ThirdBase, ShortStop,
    LeftField, CenterField, RightField,
    StartingPitcher, ReliefPitcher, DesignatedHitter,
    Utility, Bench, InjuredList,
    Outfield,        // OF combo slot (LF/CF/RF)
    MiddleInfield,   // MI combo slot (2B/SS)
    CornerInfield,   // CI combo slot (1B/3B)
    GenericPitcher,  // P combo slot (SP/RP)
}
```

ESPN slot ID constants (0-17) are also defined and used for wire protocol.

#### What the Design Doc Gets Wrong

The design doc introduces a parallel string-based position system:

```rust
pub struct DailyPlayerRow {
    pub slot: String,              // "C", "SP", "BENCH", etc.
    pub positions: Vec<String>,    // ["1B", "C", "DH"]
}
```

This creates two independent position representations that can drift apart.

#### Missing Positions

The design doc examples only show: `C, 1B, 2B, SS, 3B, LF, CF, RF, DH, UTIL, SP, RP, BENCH`.

**Missing (supported by ESPN and the `Position` enum):**
- `OF` (combo slot: Outfield, accepts LF/CF/RF)
- `MI` (combo slot: Middle Infield, accepts 2B/SS)
- `CI` (combo slot: Corner Infield, accepts 1B/3B)
- `P` (combo slot: Generic Pitcher, accepts SP/RP)
- `IL` (appears in text but not consistently in table examples)

Other ESPN leagues can use any combination of these 18 slot types.

#### Specific Hardcoding Problems

1. **`DailyPlayerRow.slot: String`** -> should be `Position`
2. **`DailyPlayerRow.positions: Vec<String>`** -> should be `Vec<Position>` or `Vec<u16>` (ESPN slot IDs)
3. **`count_games_started()`** does `row.slot == "SP"` string comparison -> should be `row.slot == Position::StartingPitcher`
4. **`is_bench` / `is_il` fields** are redundant if `slot` is typed — derivable from `slot == Position::Bench` / `slot == Position::InjuredList`
5. **Table column headers** are hardcoded to this league's specific stat columns — should be driven by ESPN's `data-statid` attributes
6. **WebSocket message `slot` field** should use ESPN slot IDs (numeric), not strings, matching the draft page protocol
7. **Roster View `Pos` column** renders from raw strings — should use `Position::display_str()`

#### Recommended Changes

**A. Replace string-based position fields with `Position` enum:**
```rust
pub struct DailyPlayerRow {
    pub slot: Position,
    pub player_name: String,
    pub team: String,
    pub eligible_positions: Vec<Position>,  // was Vec<String> named "positions"
    pub opponent: Option<String>,
    pub game_status: Option<String>,
    pub stats: Vec<StatValue>,
    // is_bench and is_il are derivable from slot, consider removing
}
```

**B. Fix `count_games_started` to use enum comparison:**
```rust
fn count_games_started(days: &[ScoringDay]) -> u8 {
    days.iter()
        .flat_map(|d| &d.pitching_rows)
        .filter(|row| row.slot == Position::StartingPitcher && row.opponent.is_some())
        .count() as u8
}
```

**C. WebSocket message should use ESPN slot IDs, not strings:**
```json
{
  "slot_id": 0,
  "eligible_slots": [0, 1, 11],
  ...
}
```

**D. Add combo slot examples to layout mockups:**
```
SLOT  Player           Team  Opp   ...
C     B. Rice          NYY   @BOS  ...
1B    F. Freeman       LAD   SD    ...
CI    A. Riley         ATL   --    ...    <-- Corner Infield combo slot
OF    J. Soto          NYM   @WSH  ...   <-- Outfield combo slot
MI    G. Henderson     BAL   @TOR  ...   <-- Middle Infield combo slot
```

**E. Make stat columns data-driven** from ESPN's `data-statid` attributes rather than hardcoded header lists.

---

## Q3: Scoring Categories Hardcoding

### Question

> Same as #2 but for scoring categories as well; I think those look hardcoded as part of some of this and we should do something more. We definitely have some scoring categories enum in the project so look those up.

### Answer

**Yes, categories are heavily hardcoded. The project has a split personality on category configurability — `LeagueConfig` stores them as dynamic `Vec<String>`, but everything else uses hardcoded named struct fields.** There is no stat enum or metadata registry in the project today.

#### Current State of Category Definitions in the Project

| Location | Representation | Dynamic? |
|----------|---------------|----------|
| `config.rs` `LeagueConfig.batting_categories` / `pitching_categories` | `Vec<String>` | Yes |
| `config.rs` `CategoryWeights` | 12 named `f64` fields (R, HR, ...) | No |
| `valuation/zscore.rs` `HitterZScores` / `PitcherZScores` | Named fields per stat | No |
| `valuation/projections.rs` `HitterProjection` / `PitcherProjection` | Named fields per stat | No |
| `valuation/analysis.rs` `CategoryNeeds` | 12 named fields | No |
| `valuation/mod.rs` z-score weighting | Hardcoded arithmetic | No |
| `tui/onboarding/strategy_setup.rs` `CATEGORIES` const | `&[&str]` of 12 | No |
| `extension/espn.js` `BATTING_STAT_MAP` / `PITCHING_STAT_MAP` | ESPN stat ID -> field name | No |
| `llm/prompt.rs` | Reads from `LeagueConfig` dynamically | Yes |

#### What's Hardcoded in the Design Doc

1. **Category stat ID table** (lines 27-40) — lists all 12 specific categories with ESPN stat IDs, types, and sort directions.
2. **`is_close_category()` thresholds** (lines 875-886) — hardcoded match arms per category abbreviation, with `_ => false` silently ignoring unknown categories.
3. **Scoreboard column layout** — hardcodes `R HR RBI BB SB AVG | K W SV HD ERA WHIP`.
4. **Player table column headers** — hardcodes `AB, H, R, HR, RBI, BB, SB, AVG` for batting and `IP, H, ER, BB, K, W, SV, HD` for pitching.
5. **Pace projection functions** — bespoke `project_avg()` and `project_era()` only handle known rate stats.
6. **Sidebar height** — assumes exactly 12 categories (`~16 rows = 2 headers + 12 categories + 2 section breaks`).
7. **WebSocket `headers` arrays** — hardcoded strings in the message format.

#### Recommendation: Build a `StatDefinition` Metadata Type

This becomes shared infrastructure that both the draft valuation pipeline and matchup page can reference:

```rust
pub struct StatDefinition {
    pub abbrev: &'static str,        // "R", "HR", "ERA", etc.
    pub full_name: &'static str,     // "Runs", "Home Runs", etc.
    pub espn_stat_id: u16,           // 20, 5, 47, etc.
    pub stat_type: StatType,         // Batting | Pitching
    pub sort_direction: SortDir,     // Desc (higher wins) | Asc (lower wins)
    pub is_rate_stat: bool,          // true for AVG, ERA, WHIP
    pub close_threshold: f64,        // threshold for "close category" detection
    pub format_precision: u8,        // 0 for counting stats, 3 for AVG, 2 for ERA/WHIP
    pub rate_components: Option<(u16, u16)>,  // e.g. AVG = H(1)/AB(0), ERA = ER(45)*9/IP(34)
}
```

**A stat registry** (initialized from the league config's category lists) would map abbreviation to `StatDefinition`. For the 12 currently configured categories, the metadata is known at compile time; the registry selects which are active based on league config.

#### Specific Design Doc Changes

**A. Refactor `CategoryScore`** to reference stat definition rather than carrying ad-hoc fields:
```rust
pub struct CategoryScore {
    pub stat: &'static StatDefinition,  // replaces abbrev + lower_is_better
    pub my_value: f64,
    pub opp_value: f64,
    pub i_am_winning: Option<bool>,
}
```

**B. Make `is_close_category()` generic** — replace match arms with `score.stat.close_threshold` lookup.

**C. Make scoreboard widget data-driven** — iterate over `league_config.batting_categories` and `league_config.pitching_categories`, look up display width and formatting from the stat registry.

**D. Make player table columns data-driven** — define "display stats" (supporting stats like AB, H, IP, ER + H2H categories) vs "H2H categories" (scored categories). The split should be metadata, not hardcoded header lists.

**E. Make projection dispatch data-driven** — rate stats need special logic because they're ratios. The `rate_components` field on `StatDefinition` specifies that AVG = H/AB, ERA = ER*9/IP, WHIP = (H+BB)/IP. Counting stats all share the same linear projection formula.

**F. Sidebar height should compute from actual category count**, not assume 12.

#### Pragmatic Note

Fully genericizing the valuation pipeline's named-field structs (`HitterZScores`, `CategoryNeeds`, `CategoryWeights`) into `HashMap<String, f64>` would be a massive refactor and probably not worth it for a single-league tool. The stat registry gives the matchup page a clean data-driven approach without requiring that refactor. The set of 12 categories is effectively fixed at compile time, but the matchup page should reference the registry rather than embedding its own hardcoded tables and match arms.
