# Matchup Page Design

## Overview

A TUI page that displays when the user is viewing an ESPN fantasy baseball matchup. It shows the head-to-head category comparison, daily player stats, and analytics to help optimize lineup decisions during a scoring period.

**Data source:** ESPN matchup box score page HTML, parsed by the Firefox extension and relayed via WebSocket. Reference HTML: `docs/espn-matchup.html`.

---

## Data Available from ESPN HTML

### Scoreboard (matchup-stats-table)

| Field | Source | Notes |
|-------|--------|-------|
| Team names | `.team-name` text | e.g. "Bob Dole Experience" |
| Team records | `.team-record` text | W-L-T format, e.g. "0-0-0" |
| Owner names | `.owner-name` spans | Multiple owners per team |
| Matchup score | `.team-score h2` text | W-L-T category score, e.g. "2-3-7" |
| Category totals | `data-statid` cells in matchup-stats-table | All 12 H2H categories |
| Winning categories | `.cell-highlight` class on stat cells | Applied to leader's value |
| Matchup period | Page title | "Matchup 1 (Mar 25 - Apr 5)" |

### Category Stat IDs

| Stat ID | Abbrev | Name | Type | Sort |
|---------|--------|------|------|------|
| 20 | R | Runs | Batting | desc (higher wins) |
| 5 | HR | Home Runs | Batting | desc |
| 21 | RBI | Runs Batted In | Batting | desc |
| 10 | BB | Walks | Batting | desc |
| 23 | SB | Stolen Bases | Batting | desc |
| 2 | AVG | Batting Average | Batting | desc |
| 48 | K | Strikeouts | Pitching | desc |
| 53 | W | Wins | Pitching | desc |
| 57 | SV | Saves | Pitching | desc |
| 60 | HD | Holds | Pitching | desc |
| 47 | ERA | Earned Run Average | Pitching | asc (lower wins) |
| 41 | WHIP | Walks+Hits/IP | Pitching | asc (lower wins) |

### Supporting Pitching Stats (in player tables, not H2H categories)

| Stat ID | Abbrev | Name |
|---------|--------|------|
| 34 | IP | Innings Pitched |
| 37 | H | Hits Allowed |
| 39 | BB | Walks Allowed |
| 45 | ER | Earned Runs |

### Supporting Batting Stats (in player tables, not H2H categories)

| Stat ID | Abbrev | Name |
|---------|--------|------|
| 0 | AB | At Bats |
| 1 | H | Hits |

### Player Table Data (per player row)

| Field | Source |
|-------|--------|
| Roster slot | `.table--cell` with title (e.g. "Catcher" -> "C") |
| Player name | `a.AnchorLink` inside `.player-column__athlete` |
| Team abbreviation | `.playerinfo__playerteam` span |
| Position eligibility | `.playerinfo__playerpos` span (comma-separated) |
| Opponent | `.table--cell.opp` ("--" if no game) |
| Game status | `.game-status` div |
| Daily stats | Stat columns per table type |
| Bench indicator | "BENCH" in slot column |
| IL indicator | "IL" in slot column |

### Daily Structure

The box score page shows one day at a time with headers:
- "March 26 Batting" / "March 26 Pitching" (sub-headers in the player tables)
- Each day has separate batting and pitching stat tables
- Players with no game that day show "--" in opponent column
- TOTALS row at the bottom of each section

### Matchup Carousel

Multiple matchups displayed in a horizontal carousel (`.carousel-container`). Each item shows both teams' names and current W-L-T scores. The selected matchup has the `.selected` class.

---

## Page Layout

### Full Layout (120+ columns)

```
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Matchup 1 (Mar 25 - Apr 5)  │  Bob Dole Experience  vs  Certified! Smokified!  │  Day 2 of 12  │  ← → Navigate Days │ <- Status Bar
├─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│  CATEGORY SCOREBOARD                                                                                                  │
│                                                                                                                       │
│  Team               R    HR   RBI   BB   SB   AVG  │  K    W    SV   HD   ERA    WHIP  │  Score                       │
│  ──────────────────────────────────────────────────────────────────────────────────────────────────                     │
│  Bob Dole Exp.     *5     2   *5    *3    1   .275  │  42   1     0   *2   *3.50  *1.20 │  6-4-2                       │
│  Certified!         3    *3    4     1   *2  *.290  │ *48  *2    *1    0    4.20   1.35 │  4-6-2                       │
│  Differential      +2    -1   +1    +2   -1  -.015 │  -6   -1   -1   +2  -0.70  -0.15 │                              │
├────────────────────────────────────────────────────────────────────────────────┬─────────────────────────────────────────┤
│  [1: Daily Stats]  [2: Analytics]  [3: My Roster]  [4: Opp Roster]            │  CATEGORY TRACKER            ▴▾ scroll │
│ ──────────────────────────────────────────────────────────────────────────     │                                        │
│                                                                               │  Batting                               │
│  ┌─ March 26 Batting ──────────────────────────────────────────────────┐      │  R    ██████████████░░░░  +2   WIN     │
│  │ SLOT  Player           Team  Opp   AB   H   R  HR RBI BB  SB  AVG  │      │  HR   ████████████░░░░░░  -1   LOSS    │
│  │ ────  ──────           ────  ───  ───  ── ──  ── ─── ──  ──  ────  │      │  RBI  █████████████░░░░░  +1   WIN     │
│  │ C     B. Rice          NYY   @BOS   4   1  0   0   1  0   0  .250  │      │  BB   ██████████████░░░░  +2   WIN     │
│  │ 1B    F. Freeman       LAD   SD     3   2  1   1   2  1   0  .667  │      │  SB   ████████████░░░░░░  -1   LOSS    │
│  │ 2B    K. Marte         ARI   COL    4   1  1   0   0  0   1  .250  │      │  AVG  ████████████░░░░░░  -.015 LOSS   │
│  │ SS    G. Henderson     BAL   @TOR   4   0  0   0   0  1   0  .000  │      │                                        │
│  │ 3B    A. Riley         ATL   --    ──  ── ──  ── ─── ──  ──  ────  │      │  Pitching                              │
│  │ LF    I. Happ          CHC   @CIN   3   1  1   0   0  1   0  .333  │      │  K    ████████████░░░░░░  -6   LOSS    │
│  │ CF    R. Greene         DET  @CLE   4   2  1   0   1  0   0  .500  │      │  W    ████████████░░░░░░  -1   LOSS    │
│  │ RF    J. Soto          NYM   @WSH   3   0  0   0   0  2   0  .000  │      │  SV   ████████████░░░░░░  -1   LOSS    │
│  │ UTIL  S. Ohtani        LAD   SD     4   1  1   1   2  0   0  .250  │      │  HD   ██████████████░░░░  +2   WIN     │
│  │ BENCH C. Yelich        MIL   @PIT  ──  ── ──  ── ─── ──  ──  ────  │      │  ERA  ██████████████░░░░  -0.70 WIN    │
│  │ TOTALS                            29   8  5   2   6  5   1  .276  │      │  WHIP ██████████████░░░░  -0.15 WIN    │
│  └────────────────────────────────────────────────────────────────────┘      │                                        │
│                                                                               ├─────────────────────────────────────────┤
│  ┌─ March 26 Pitching ────────────────────────────────────────────────┐      │  LIMITS & RESOURCES                    │
│  │ SLOT  Player           Team  Opp   IP   H  ER  BB   K  W SV HD    │      │                                        │
│  │ ────  ──────           ────  ───  ───  ── ──  ──  ──  ─ ── ──    │      │  Games Started (GS)                    │
│  │ SP    F. Valdez        HOU   @TEX  7.0  4  2   1   8  1  0  0    │      │  ████████████████░░░░  5/7             │
│  │ SP    T. Glasnow       LAD   SD    6.0  3  1   2   9  0  0  0    │      │                                        │
│  │ RP    L. Weaver         NYY  @BOS  1.0  0  0   0   2  0  1  0    │      │  Acquisitions                          │
│  │ RP    R. Suarez         SD   @LAD  1.0  1  0   0   1  0  0  1    │      │  ████████████░░░░░░░░  3/5             │
│  │ BENCH B. Woo           SEA   --   ──  ── ──  ──  ──  ─ ── ──    │      │                                        │
│  │ TOTALS                           15.0  8  3   3  20  1  1  1    │      │  Days Remaining: 10                    │
│  └────────────────────────────────────────────────────────────────────┘      │  Games Today: 8 of 13 roster spots    │
│                                                                               │                                        │
├────────────────────────────────────────────────────────────────────────────────┴─────────────────────────────────────────┤
│  ← → Day   1-4 Tab   ↑↓ Scroll   Tab Focus   q Quit                                                                  │ <- Help Bar
└─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### Narrow Layout (80 columns)

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│ Matchup 1 (Mar 25-Apr 5)  BDE vs C!S!  Day 2/12  ← →                          │
├──────────────────────────────────────────────────────────────────────────────────┤
│        R  HR RBI BB SB  AVG   K  W SV HD  ERA  WHIP  W-L-T                     │
│  BDE  *5   2  *5 *3  1 .275  42  1  0 *2 *3.5 *1.20  6-4-2                    │
│  C!S!  3  *3   4  1 *2 .290 *48 *2 *1  0  4.2  1.35  4-6-2                    │
├──────────────────────────────────────────────────────────────────────────────────┤
│ [1: Daily] [2: Analytics] [3: My Roster] [4: Opp]                              │
│ ── March 26 Batting ──────────────────────────────                              │
│ SLOT Player        Tm  Opp  AB  H  R HR RBI BB SB AVG                          │
│ C    B. Rice       NYY @BOS  4  1  0  0   1  0  0 .250                         │
│ 1B   F. Freeman    LAD SD    3  2  1  1   2  1  0 .667                         │
│ 2B   K. Marte      ARI COL   4  1  1  0   0  0  1 .250                        │
│ SS   G. Henderson  BAL @TOR  4  0  0  0   0  1  0 .000                         │
│ 3B   A. Riley      ATL --   -- -- -- -- --- -- -- ----                          │
│ LF   I. Happ       CHC @CIN  3  1  1  0   0  1  0 .333                        │
│ ...                                                                             │
│ TOTALS                      29  8  5  2   6  5  1 .276                         │
│                                                                                 │
│ ── March 26 Pitching ─────────────────────────────                              │
│ SLOT Player        Tm  Opp  IP  H ER BB  K W SV HD                             │
│ SP   F. Valdez     HOU @TEX 7.0 4  2  1  8 1  0  0                            │
│ SP   T. Glasnow    LAD SD   6.0 3  1  2  9 0  0  0                            │
│ ...                                                                             │
├──────────────────────────────────────────────────────────────────────────────────┤
│ ← → Day  1-4 Tab  ↑↓ Scroll  Tab Focus  q Quit                                │
└──────────────────────────────────────────────────────────────────────────────────┘
```

In the narrow layout, the sidebar is hidden. The category tracker and limits info are accessible via the Analytics tab instead.

---

## Component Architecture

### Screen: `MatchupScreen`

Top-level screen component, analogous to `DraftScreen`. Owns all matchup state and child components.

```rust
pub struct MatchupScreen {
    // Child components
    pub main_panel: MatchupMainPanel,
    pub sidebar: MatchupSidebar,

    // Matchup state (set by parent from UiUpdate)
    pub matchup_info: Option<MatchupInfo>,
    pub my_team: TeamMatchupState,
    pub opp_team: TeamMatchupState,
    pub category_scores: Vec<CategoryScore>,
    pub selected_day: usize,          // Index into scoring period days
    pub scoring_period_days: Vec<ScoringDay>,

    // Limits
    pub games_started: u8,            // Current GS count
    pub acquisitions_used: u8,        // Current acquisition count

    // Focus
    pub focused_panel: Option<MatchupFocusPanel>,
}
```

### Data Structures

```rust
pub struct MatchupInfo {
    pub matchup_period: u8,           // e.g. 1
    pub start_date: NaiveDate,        // Mar 25
    pub end_date: NaiveDate,          // Apr 5
    pub my_team_name: String,
    pub opp_team_name: String,
    pub my_record: TeamRecord,        // W-L-T overall
    pub opp_record: TeamRecord,
}

pub struct TeamRecord {
    pub wins: u16,
    pub losses: u16,
    pub ties: u16,
}

pub struct CategoryScore {
    pub stat_id: u16,
    pub abbrev: String,               // "R", "HR", etc.
    pub my_value: f64,
    pub opp_value: f64,
    pub i_am_winning: Option<bool>,   // None = tied
    pub lower_is_better: bool,        // true for ERA, WHIP
}

pub struct ScoringDay {
    pub date: NaiveDate,
    pub label: String,                // "March 26"
    pub batting_rows: Vec<DailyPlayerRow>,
    pub pitching_rows: Vec<DailyPlayerRow>,
    pub batting_totals: Option<DailyTotals>,
    pub pitching_totals: Option<DailyTotals>,
}

pub struct DailyPlayerRow {
    pub slot: String,                 // "C", "SP", "BENCH", etc.
    pub player_name: String,
    pub team: String,                 // "NYY", "LAD"
    pub positions: Vec<String>,       // ["1B", "C", "DH"]
    pub opponent: Option<String>,     // None if no game ("--")
    pub game_status: Option<String>,  // Injury, PPD, etc.
    pub stats: Vec<StatValue>,        // Ordered per table columns
    pub is_bench: bool,
    pub is_il: bool,
}

pub struct StatValue {
    pub stat_id: u16,
    pub value: Option<f64>,           // None if no game
    pub display: String,              // Formatted string ("3", ".275", "--")
}

pub struct DailyTotals {
    pub stats: Vec<StatValue>,
}

pub struct TeamMatchupState {
    pub name: String,
    pub abbrev: String,               // Short display name
    pub record: TeamRecord,
    pub category_score: TeamRecord,   // W-L-T in this matchup
    pub roster: Vec<DailyPlayerRow>,  // Full roster for current day
}
```

---

## Components

### 1. Status Bar (reuse existing `StatusBar` widget pattern)

Displays matchup period, team names, current day, and day navigation hint.

```
│ Matchup 1 (Mar 25 - Apr 5)  │  Bob Dole Exp. vs Certified!  │  Day 2 of 12  │  ← → Days │
```

**Data source:** `MatchupInfo`, `selected_day`, `scoring_period_days.len()`

**Behavior:** Static display. Day navigation keys (← →) shown as hint.

**Height:** 1 row (fixed)

### 2. Scoreboard (`ScoreboardWidget`)

Category-by-category comparison between the two teams, with differentials.

```
          R    HR   RBI   BB   SB   AVG  │  K    W    SV   HD   ERA    WHIP  │  Score
 BDE     *5     2   *5    *3    1   .275 │  42    1    0   *2   *3.50  *1.20 │  6-4-2
 C!S!     3    *3    4     1   *2  *.290 │ *48   *2   *1    0    4.20   1.35 │  4-6-2
 Diff    +2    -1   +1    +2   -1  -.015 │  -6   -1   -1   +2  -0.70  -0.15 │
```

**Layout:** 5 rows fixed height (1 header + 2 team rows + 1 differential + 1 border)

**Data source:** `category_scores`, `my_team`, `opp_team`

**Visual rules:**
- Winning values marked with `*` prefix (bold + green in TUI)
- Losing values in default color
- Tied values in yellow
- Differential row: positive values (good for us) in green, negative in red, zero in yellow
- Vertical separator between batting and pitching categories
- Team names truncated to fit (abbreviate if needed)
- Score column shows the matchup W-L-T (categories won-lost-tied)

**Height:** 5 rows (fixed)

### 3. Main Panel (`MatchupMainPanel`)

Tab container with 4 tabs. Follows existing `MainPanel` pattern from draft screen.

**Tabs:**
1. **Daily Stats** (default)
2. **Analytics**
3. **My Roster** (full scoring period)
4. **Opponent Roster** (full scoring period)

```rust
pub struct MatchupMainPanel {
    pub active_tab: MatchupTab,
    pub daily_panel: DailyStatsPanel,
    pub analytics_panel: AnalyticsPanel,
    pub my_roster_panel: RosterViewPanel,
    pub opp_roster_panel: RosterViewPanel,
}

#[derive(Debug, Clone, Copy)]
pub enum MatchupTab {
    DailyStats,
    Analytics,
    MyRoster,
    OppRoster,
}
```

#### Tab 1: Daily Stats Panel (`DailyStatsPanel`)

Shows batting and pitching tables for the currently selected day.

```
── March 26 Batting ──────────────────────────────────────────────────
SLOT  Player           Team  Opp    AB   H   R  HR  RBI  BB  SB   AVG
────  ──────           ────  ───   ───  ──  ──  ──  ───  ──  ──  ────
C     B. Rice          NYY   @BOS    4   1   0   0    1   0   0  .250
1B    F. Freeman       LAD   SD      3   2   1   1    2   1   0  .667
2B    K. Marte         ARI   COL     4   1   1   0    0   0   1  .250
SS    G. Henderson     BAL   @TOR    4   0   0   0    0   1   0  .000
3B    A. Riley         ATL   --     --  --  --  --   --  --  --    --
LF    I. Happ          CHC   @CIN    3   1   1   0    0   1   0  .333
CF    R. Greene         DET  @CLE    4   2   1   0    1   0   0  .500
RF    J. Soto          NYM   @WSH    3   0   0   0    0   2   0  .000
DH    M. Betts         LAD   SD      4   1   0   0    0   0   1  .250
UTIL  S. Ohtani        LAD   SD      4   1   1   1    2   0   0  .250
────────────────────────────────────────────────────────────────────
BENCH C. Yelich        MIL   @PIT   --  --  --  --   --  --  --    --
BENCH T. Grisham       NYY   @BOS    3   0   0   0    0   0   0  .000
TOTALS                             36  9   5   2    6   5   2  .250
══════════════════════════════════════════════════════════════════════

── March 26 Pitching ─────────────────────────────────────────────────
SLOT  Player           Team  Opp    IP    H  ER  BB   K   W  SV  HD
────  ──────           ────  ───   ───   ──  ──  ──  ──   ─  ──  ──
SP    F. Valdez        HOU   @TEX  7.0    4   2   1   8   1   0   0
SP    T. Glasnow       LAD   SD    6.0    3   1   2   9   0   0   0
RP    L. Weaver        NYY   @BOS  1.0    0   0   0   2   0   1   0
RP    R. Suarez        SD    @LAD  1.0    1   0   0   1   0   0   1
RP    R. Walker        SF    COL   1.0    0   0   0   1   0   0   0
RP    B. Abreu         HOU   @TEX  1.0    0   0   1   2   0   0   1
────────────────────────────────────────────────────────────────────
BENCH B. Woo           SEA   --    --    -- --  --  --   -  --  --
TOTALS                            17.0   8   3   4  23   1   1   2
```

**Data source:** `scoring_period_days[selected_day]`

**Behavior:**
- Scrollable vertically (batting + pitching in one scrollable view)
- Players with no game show "--" for all stats and are dimmed (dark gray text)
- Bench players shown below a separator line
- IL players shown after bench (if any)
- TOTALS row at bottom of each section in bold
- Bench player stats do NOT count toward totals (they're informational only)

**Columns:**
- Batting: SLOT, Player, Team, Opp, AB, H, R, HR, RBI, BB, SB, AVG
- Pitching: SLOT, Player, Team, Opp, IP, H, ER, BB, K, W, SV, HD

```rust
pub struct DailyStatsPanel {
    scroll: ScrollState,
}

pub enum DailyStatsPanelMessage {
    Scroll(ScrollDirection),
}
```

#### Tab 2: Analytics Panel (`MatchupAnalyticsPanel`)

Computed analytics derived from parsed HTML data. All calculations happen client-side.

```
── MATCHUP ANALYSIS ─────────────────────────────────────────────────

Category Outlook (Day 2 of 12)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  WINNING (6)                    LOSING (4)              TIED (2)
  R    +2                        HR   -1                 SB   0
  RBI  +1                        K    -6                 AVG  .000
  BB   +2                        W    -1
  HD   +2                        SV   -1
  ERA  -0.70 (lower=better)
  WHIP -0.15 (lower=better)

── CLOSE CATEGORIES (swingable) ─────────────────────────────────────

  Category  Mine  Theirs  Diff   Status
  ────────  ────  ──────  ────   ──────
  HR          2       3    -1    LOSING - 1 HR to tie, 2 to lead
  SB          1       2    -1    LOSING - 1 SB to tie, 2 to lead
  RBI         5       4    +1    WINNING - lead is narrow
  SV          0       1    -1    LOSING - 1 SV to tie

── PACE PROJECTIONS ─────────────────────────────────────────────────

  Based on 2 days played, projecting over 12-day period:

  Category  Current  Projected  Opp Proj  Proj Result
  ────────  ───────  ─────────  ────────  ───────────
  R            5        30        18       WIN (+12)
  HR           2        12        18       LOSS (-6)
  RBI          5        30        24       WIN (+6)
  BB           3        18         6       WIN (+12)
  ...

── GAMES STARTED TRACKER ────────────────────────────────────────────

  Used: 5 / 7 GS limit
  Remaining SP starts this week:
    Mar 27: F. Valdez (vs TEX)
    Mar 28: T. Glasnow (vs SD)
    Mar 29: (none scheduled)
  WARNING: Only 2 GS remaining - manage carefully

── ACQUISITIONS ─────────────────────────────────────────────────────

  Used: 3 / 5 per matchup
  Remaining: 2
```

**Data source:** Computed from `category_scores`, `scoring_period_days`, `games_started`, `acquisitions_used`

**Analytics computations (all from parsed HTML):**

1. **Category grouping**: Sort categories into Winning/Losing/Tied buckets based on `i_am_winning` field and accounting for `lower_is_better` (ERA, WHIP).

2. **Close categories**: Categories where the absolute differential is small enough to swing with normal daily production. Thresholds:
   - Counting stats (R/HR/RBI/BB/SB/K/W/SV/HD): |diff| <= 3
   - Rate stats (AVG): |diff| <= .020
   - Rate stats (ERA): |diff| <= 1.00
   - Rate stats (WHIP): |diff| <= 0.20

3. **Pace projections**: `projected = (current_total / days_elapsed) * total_days`. For rate stats (AVG/ERA/WHIP), project the underlying counting stats (H, AB, ER, IP, etc.) and recompute the rate.

4. **GS tracker**: Count rows where slot = "SP" and opponent is not "--" across all days in the scoring period. Show remaining scheduled starts.

5. **Acquisitions**: Display current count vs limit. (Count parsed from page state if available, otherwise tracked locally.)

```rust
pub struct MatchupAnalyticsPanel {
    scroll: ScrollState,
}

pub enum MatchupAnalyticsPanelMessage {
    Scroll(ScrollDirection),
}
```

#### Tab 3 & 4: Roster View Panel (`RosterViewPanel`)

Shows a team's full roster for the entire scoring period, with aggregate stats.

Reused for both "My Roster" and "Opponent Roster" tabs — same component, different data.

```
── BOB DOLE EXPERIENCE - Full Roster ───────────────────────────────

  SLOT  Player           Team  Pos         GP  AB   H   R  HR  RBI  BB  SB   AVG
  ────  ──────           ────  ───         ──  ──  ──  ──  ──  ───  ──  ──  ────
  C     B. Rice          NYY   1B,C,DH      2   7   2   1   0    2   1   0  .286
  1B    F. Freeman       LAD   1B           2   7   4   2   1    3   1   0  .571
  2B    K. Marte         ARI   2B           2   8   3   1   0    1   0   2  .375
  SS    G. Henderson     BAL   SS,3B        2   8   1   0   0    1   2   0  .125
  3B    A. Riley         ATL   3B           0   0   0   0   0    0   0   0   --
  ...
  TOTALS                                        62  22   9   4   14   8   4  .355

  SLOT  Player           Team  Pos         GS  IP    H  ER  BB   K   W  SV  HD  ERA   WHIP
  ────  ──────           ────  ───         ──  ───   ──  ── ──  ──   ─  ──  ──  ────  ────
  SP    F. Valdez        HOU   SP           1  7.0    4   2   1   8   1   0   0  2.57  0.71
  SP    T. Glasnow       LAD   SP           1  6.0    3   1   2   9   0   0   0  1.50  0.83
  ...
```

**Data source:** Aggregate across all `scoring_period_days` for the team's players

**Behavior:**
- Scrollable vertically
- Shows games played (GP) for hitters, games started (GS) for pitchers
- Aggregate stats across all days in the scoring period
- Players with 0 games show "--" for rate stats

```rust
pub struct RosterViewPanel {
    scroll: ScrollState,
}

pub enum RosterViewPanelMessage {
    Scroll(ScrollDirection),
}
```

### 4. Sidebar (`MatchupSidebar`)

Fixed sidebar on the right (35% width). Contains two sections stacked vertically.

```rust
pub struct MatchupSidebar {
    pub category_tracker: CategoryTrackerPanel,
    pub limits_panel: LimitsPanel,
}
```

**Visibility:** Hidden when terminal width < 100 columns. Analytics tab serves as fallback for this data.

#### Category Tracker (`CategoryTrackerPanel`)

Visual bars showing relative position in each category.

```
CATEGORY TRACKER

Batting
R    ██████████████░░░░  +2   WIN
HR   ████████████░░░░░░  -1   LOSS
RBI  █████████████░░░░░  +1   WIN
BB   ██████████████░░░░  +2   WIN
SB   ████████████░░░░░░  -1   LOSS
AVG  ████████████░░░░░░  -.015 LOSS

Pitching
K    ████████████░░░░░░  -6   LOSS
W    ████████████░░░░░░  -1   LOSS
SV   ████████████░░░░░░  -1   LOSS
HD   ██████████████░░░░  +2   WIN
ERA  ██████████████░░░░  -0.70 WIN
WHIP ██████████████░░░░  -0.15 WIN
```

**Bar visualization:** Each bar represents relative standing. The bar is split at the midpoint:
- Green fill = my value's proportion
- Red fill = opponent's proportion
- For `lower_is_better` stats (ERA, WHIP), the colors are inverted

**Labels:**
- Differential value with +/- sign
- WIN (green), LOSS (red), or TIED (yellow) status

**Data source:** `category_scores`

**Height:** ~16 rows (2 headers + 12 categories + 2 section breaks)

```rust
pub struct CategoryTrackerPanel {
    scroll: ScrollState,
}
```

#### Limits Panel (`LimitsPanel`)

Tracks GS limit and acquisition count.

```
LIMITS & RESOURCES

Games Started (GS)
████████████████░░░░  5/7

Acquisitions
████████████░░░░░░░░  3/5

Days Remaining: 10
Games Today: 8 of 13 roster spots
```

**Bar visualization:** Progress bar showing used/total.
- Green when plenty remaining (< 60% used)
- Yellow when getting tight (60-85% used)
- Red when nearly exhausted (> 85% used)

**"Games Today":** Count of roster players (non-bench, non-IL) who have a game today (opponent != "--").

**Data source:** `games_started`, `acquisitions_used`, `scoring_period_days`, `selected_day`

**Height:** ~8 rows (fixed)

```rust
pub struct LimitsPanel;  // Stateless widget, data passed to view()
```

---

## Message Architecture

Following the existing ELM pattern:

```rust
#[derive(Debug, Clone)]
pub enum MatchupScreenMessage {
    // Day navigation
    PreviousDay,
    NextDay,

    // Tab switching
    SwitchTab(MatchupTab),

    // Focus cycling
    CycleFocus,
    CycleFocusBack,

    // Delegated to child
    MainPanel(MatchupMainPanelMessage),
    Sidebar(MatchupSidebarMessage),
}

#[derive(Debug, Clone)]
pub enum MatchupMainPanelMessage {
    DailyStats(DailyStatsPanelMessage),
    Analytics(MatchupAnalyticsPanelMessage),
    MyRoster(RosterViewPanelMessage),
    OppRoster(RosterViewPanelMessage),
}

#[derive(Debug, Clone)]
pub enum MatchupSidebarMessage {
    CategoryTracker(CategoryTrackerPanelMessage),
}
```

### Key Bindings

| Key | Context | Action | Priority |
|-----|---------|--------|----------|
| `←` / `h` | Always | Previous day | NORMAL |
| `→` / `l` | Always | Next day | NORMAL |
| `1` | Always | Switch to Daily Stats tab | NORMAL |
| `2` | Always | Switch to Analytics tab | NORMAL |
| `3` | Always | Switch to My Roster tab | NORMAL |
| `4` | Always | Switch to Opponent Roster tab | NORMAL |
| `Tab` | Always | Cycle focus forward | NORMAL |
| `Shift+Tab` | Always | Cycle focus backward | NORMAL |
| `↑` / `k` | Panel focused | Scroll up | NORMAL |
| `↓` / `j` | Panel focused | Scroll down | NORMAL |
| `q` | Always | Quit / back to home | NORMAL |
| `Ctrl+C` | Always | Force quit | highest (no hint) |

### Focus Panels

```rust
#[derive(Debug, Clone, Copy)]
pub enum MatchupFocusPanel {
    MainPanel,
    CategoryTracker,
    Limits,
}
```

Focus cycles: `MainPanel → CategoryTracker → Limits → None → MainPanel → ...`

When no panel is focused, scroll events go to the active tab's panel.

---

## Layout Construction

```rust
pub struct MatchupLayout {
    pub status_bar: Rect,
    pub scoreboard: Rect,
    pub main_panel: Rect,
    pub sidebar: Rect,       // None if terminal too narrow
    pub help_bar: Rect,
}

pub fn build_matchup_layout(area: Rect) -> MatchupLayout {
    // Vertical split:
    //   Status bar:  1 row
    //   Scoreboard:  5 rows
    //   Content:     remaining
    //   Help bar:    1 row

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),           // status bar
            Constraint::Length(5),           // scoreboard
            Constraint::Min(10),            // content area
            Constraint::Length(1),           // help bar
        ])
        .split(area);

    // Horizontal split of content area (only if wide enough):
    let (main_panel, sidebar) = if area.width >= 100 {
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(65),  // main panel
                Constraint::Percentage(35),  // sidebar
            ])
            .split(vertical[2]);
        (horizontal[0], Some(horizontal[1]))
    } else {
        (vertical[2], None)
    };

    // Sidebar internal split (if sidebar exists):
    // CategoryTracker gets ~65% of sidebar, Limits gets ~35%

    MatchupLayout {
        status_bar: vertical[0],
        scoreboard: vertical[1],
        main_panel,
        sidebar,
        help_bar: vertical[3],
    }
}
```

---

## Data Flow

### Extension → Backend → TUI

1. **Firefox extension** detects the user is on an ESPN matchup/boxscore page (URL pattern: `/baseball/boxscore`)
2. Extension scrapes the DOM:
   - Matchup header (team names, records, scores)
   - Category stats table (`matchup-stats-table`)
   - Player tables (batting + pitching sections, with daily sub-headers)
   - `__NEXT_DATA__` JSON blob for supplementary data
3. Extension sends a `MatchupState` message via WebSocket to the Rust backend
4. Backend processes the state into `UiUpdate::MatchupSnapshot`
5. TUI's `App::apply_update()` routes to `MatchupScreen` state fields
6. Next frame render picks up new state

### State Diff Detection

Similar to the draft page's `compute_state_diff`, the extension should detect changes:
- Day navigation (user clicks a different day on the ESPN page)
- Score changes (live game updates)
- Roster changes (lineup moves, acquisitions)

Send differential updates when possible, full snapshots periodically.

### WebSocket Message Format

```json
{
  "type": "matchup_state",
  "matchup_period": 1,
  "start_date": "2026-03-25",
  "end_date": "2026-04-05",
  "selected_day": "2026-03-26",
  "my_team": {
    "name": "Bob Dole Experience",
    "record": "0-0-0",
    "matchup_score": "2-3-7"
  },
  "opp_team": {
    "name": "Certified! Smokified!",
    "record": "0-0-0",
    "matchup_score": "3-2-7"
  },
  "categories": [
    { "stat_id": 20, "abbrev": "R", "my_value": 5, "opp_value": 3, "lower_is_better": false },
    ...
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
        "stats": [4, 1, 0, 0, 1, 0, 0, 0.250]
      },
      ...
    ],
    "totals": [29, 8, 5, 2, 6, 5, 1, 0.276]
  },
  "pitching": {
    "headers": ["IP", "H", "ER", "BB", "K", "W", "SV", "HD"],
    "players": [...],
    "totals": [...]
  }
}
```

---

## Integration with App

### Screen Switching

The `App` component needs a new `AppMode` variant:

```rust
pub enum AppMode {
    Onboarding(OnboardingStep),
    Settings(SettingsSection),
    Draft,
    Matchup,  // NEW
}
```

The mode switches to `Matchup` when the backend receives a `matchup_state` WebSocket message (i.e., the extension detects the user is on a matchup page). It switches back to `Draft` when a `draft_state` message arrives, or to a home/disconnected view on disconnect.

### UiUpdate Variant

```rust
pub enum UiUpdate {
    // ... existing variants ...
    MatchupSnapshot(MatchupSnapshot),
    MatchupDayUpdate(MatchupDayUpdate),
}
```

---

## Analytics Computation Details

All analytics are computed in the TUI from parsed data. No external API calls.

### 1. Category Win/Loss/Tie Status

```rust
fn category_status(score: &CategoryScore) -> CategoryStatus {
    let diff = if score.lower_is_better {
        score.opp_value - score.my_value  // For ERA/WHIP, lower is better for us
    } else {
        score.my_value - score.opp_value
    };

    if diff > 0.0 { CategoryStatus::Winning }
    else if diff < 0.0 { CategoryStatus::Losing }
    else { CategoryStatus::Tied }
}
```

### 2. Close Category Detection

A category is "close" if it could realistically swing in a single day of play.

```rust
fn is_close_category(score: &CategoryScore) -> bool {
    let diff = (score.my_value - score.opp_value).abs();
    match score.abbrev.as_str() {
        "R" | "RBI" | "BB" => diff <= 5.0,
        "HR" | "SB" | "W" | "SV" | "HD" => diff <= 3.0,
        "K" => diff <= 10.0,
        "AVG" => diff <= 0.020,
        "ERA" => diff <= 1.00,
        "WHIP" => diff <= 0.20,
        _ => false,
    }
}
```

### 3. Pace Projections

```rust
fn project_counting_stat(current: f64, days_elapsed: u32, total_days: u32) -> f64 {
    if days_elapsed == 0 { return 0.0; }
    (current / days_elapsed as f64) * total_days as f64
}

// Rate stats require projecting the components:
fn project_avg(hits: f64, at_bats: f64, days_elapsed: u32, total_days: u32) -> f64 {
    let proj_hits = project_counting_stat(hits, days_elapsed, total_days);
    let proj_ab = project_counting_stat(at_bats, days_elapsed, total_days);
    if proj_ab == 0.0 { return 0.0; }
    proj_hits / proj_ab
}

fn project_era(earned_runs: f64, innings: f64, days_elapsed: u32, total_days: u32) -> f64 {
    let proj_er = project_counting_stat(earned_runs, days_elapsed, total_days);
    let proj_ip = project_counting_stat(innings, days_elapsed, total_days);
    if proj_ip == 0.0 { return 0.0; }
    (proj_er / proj_ip) * 9.0
}
```

### 4. GS Tracking

```rust
fn count_games_started(days: &[ScoringDay]) -> u8 {
    days.iter()
        .flat_map(|d| &d.pitching_rows)
        .filter(|row| row.slot == "SP" && row.opponent.is_some() && !row.is_bench)
        .count() as u8
}
```

### 5. Games Today Counter

```rust
fn count_games_today(day: &ScoringDay) -> (u8, u8) {
    let active_with_game = day.batting_rows.iter()
        .chain(day.pitching_rows.iter())
        .filter(|r| !r.is_bench && !r.is_il && r.opponent.is_some())
        .count() as u8;
    let total_active = day.batting_rows.iter()
        .chain(day.pitching_rows.iter())
        .filter(|r| !r.is_bench && !r.is_il)
        .count() as u8;
    (active_with_game, total_active)
}
```

---

## Color Scheme

Consistent with existing TUI conventions:

| Element | Color | Condition |
|---------|-------|-----------|
| Winning category value | Green, Bold | `cell-highlight` equivalent |
| Losing category value | Default (White) | |
| Tied category value | Yellow | |
| Positive differential | Green | We're ahead |
| Negative differential | Red | We're behind |
| Zero differential | Yellow | Tied |
| Bench player row | Dark Gray | Dimmed |
| IL player row | Dark Gray + "IL" tag in Red | |
| No-game stats ("--") | Dark Gray | |
| TOTALS row | White, Bold | |
| GS bar (safe) | Green | < 60% used |
| GS bar (caution) | Yellow | 60-85% used |
| GS bar (critical) | Red | > 85% used |
| Focused panel border | Cyan | Standard focus indicator |
| Section headers | White, Bold | |
| Tab bar (active) | White on White bg, Bold | Matches draft page |
| Tab bar (inactive) | White on Black bg | Matches draft page |
| "WIN" label | Green | |
| "LOSS" label | Red | |
| "TIED" label | Yellow | |
| Close category row | Yellow highlight | Attention-worthy |

---

## File Structure

```
src/tui/
  matchup/
    mod.rs              -- MatchupScreen component, message routing, update()
    layout.rs           -- build_matchup_layout(), MatchupLayout struct
    main_panel/
      mod.rs            -- MatchupMainPanel, tab container
      daily_stats.rs    -- DailyStatsPanel (Tab 1)
      analytics.rs      -- MatchupAnalyticsPanel (Tab 2)
      roster_view.rs    -- RosterViewPanel (Tabs 3 & 4, reused)
    sidebar/
      mod.rs            -- MatchupSidebar container
      category_tracker.rs  -- CategoryTrackerPanel
      limits.rs         -- LimitsPanel
    widgets/
      scoreboard.rs     -- ScoreboardWidget (the header scoreboard)
```

---

## Parsing Requirements for Extension

The Firefox extension needs to scrape these elements from the ESPN matchup page:

### Required CSS Selectors

| Data | Selector | Notes |
|------|----------|-------|
| Page type | URL contains `/baseball/boxscore` | Identifies matchup page |
| Team names | `.team-header .teamName` | Two on page (away, home) |
| Team records | `.team-header .team-record` | W-L-T format |
| Team matchup score | `.team-header .team-score h2` | W-L-T format |
| Owner names | `.team-header .owner-name` | Multiple per team |
| Category headers | `.matchup-stats-table th[data-statid]` | stat ID + abbreviation |
| Category values | `.matchup-stats-table td` | Values in order matching headers |
| Winning indicator | `.cell-highlight` on stat cells | Which team wins each category |
| Player rows | `.players-table .Table__TR--lg` | Each row = one player |
| Roster slot | First `.Table__TD .table--cell` | "C", "SP", "BENCH", etc. |
| Player name | `.player-column__athlete a` | Player name text |
| Player team | `.playerinfo__playerteam` | Team abbreviation |
| Player positions | `.playerinfo__playerpos` | Comma-separated |
| Opponent | `.table--cell.opp` | "--" if no game |
| Game status | `.game-status` | Injury/PPD status |
| Stat values | Remaining `.Table__TD` cells | In column order |
| Day sub-headers | `.Table__sub-header` containing date text | "March 26 Batting" |
| Section type | Sub-header text | "Batting" vs "Pitching" |
| Totals row | Row containing "TOTALS" | Aggregate stats |
| Matchup period | Page title or `.header` text | "Matchup 1 (Mar 25 - Apr 5)" |

### `__NEXT_DATA__` Supplementary Data

The `<script id="__NEXT_DATA__">` JSON blob contains additional structured data that may be easier to parse than DOM scraping for some fields:

- League configuration (roster settings, stat settings)
- Scoring period dates
- Player IDs and full stat objects
- Matchup period metadata

The extension should attempt to parse `__NEXT_DATA__` first for structured data, falling back to DOM scraping for rendered state (current day's stats, visual indicators like `cell-highlight`).
