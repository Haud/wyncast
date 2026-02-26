# ESPN Multi-Position Eligibility: Research Findings

**Date:** 2026-02-25
**Purpose:** Determine how ESPN handles multi-position eligibility during fantasy baseball drafts and whether our local roster logic in `roster.rs` should be replaced, supplemented, or left as-is.

---

## 1. How ESPN Determines Position Eligibility

### Pre-Season (Start of Year) Eligibility

A player qualifies at a position entering the season if they meet **either** threshold from the previous season:

- **20-game threshold:** Played at least 20 games at that position, OR
- **25% threshold:** Played at least 25% of their total games at that position (minimum 5 games)

For pitchers:
- **Starting Pitcher (SP):** At least 5 starts in the previous season
- **Relief Pitcher (RP):** At least 8 relief appearances in the previous season

If no position reaches these thresholds, the player is eligible at the position where they played the most games.

**Special case -- DH:** Designated hitter counts as a qualifying position under the same criteria.

**Special case -- Rookies/Prospects:** Combined MLB and minor league statistics are used.

**Special case -- Injured players:** Players who miss an entire season retain eligibility from their most recent active season.

### In-Season Eligibility Additions

During the 2026 season, players can gain **new** position eligibility:
- **Hitters:** 10 games at the new position (eligibility takes effect the day after the 10th appearance)
- **Pitchers:** 5 starts (for SP) or 8 relief appearances (for RP)

### Outfield Eligibility: OF vs LF/CF/RF

This is critical for our league, which uses **individual outfield slots (LF, CF, RF)** rather than generic OF slots.

- **Generic OF leagues (default ~85% of ESPN leagues):** A player needs 10 combined games across any outfield position (LF+CF+RF totaling 10) to qualify as OF.
- **Individual OF position leagues (our league):** A player needs to meet the 20-game or 25% threshold at that *specific* outfield position. Pre-season eligibility requires the threshold to be met at a singular position (LF, CF, or RF), NOT a combination. In-season, 10 games at the specific position are required.

This means a player who played 15 games in LF and 15 in RF would qualify at both LF and RF in our league, but a player who played 8 in LF and 8 in RF would qualify at neither individual position (even though they would qualify as OF in a standard league).

### Multi-Position Eligibility Examples

- **Yordan Alvarez:** DH (32 of 48 games) + LF (15 games = 31% of 48, exceeding 25%) = eligible at DH and LF
- **Mookie Betts:** Eligible at OF and 2B, could add SS if he plays enough there
- **Ben Rice:** Eligible at 1B, C, and DH simultaneously through substantial playing time at each

---

## 2. ESPN API Data Model for Position Eligibility

### Position/Slot ID Mapping

ESPN's internal API uses numeric IDs for positions and roster slots. Based on the `espn-api` community library (which reverse-engineers ESPN's v3 Fantasy API), the mapping is:

| Slot ID | Position | Notes |
|---------|----------|-------|
| 0 | C | Catcher |
| 1 | 1B | First Base |
| 2 | 2B | Second Base |
| 3 | 3B | Third Base |
| 4 | SS | Shortstop |
| 5 | OF | Generic Outfield |
| 6 | 2B/SS | Middle Infield (combo slot) |
| 7 | 1B/3B | Corner Infield (combo slot) |
| 8 | LF | Left Field |
| 9 | CF | Center Field |
| 10 | RF | Right Field |
| 11 | DH | Designated Hitter |
| 12 | UTIL | Utility |
| 13 | P | Generic Pitcher |
| 14 | SP | Starting Pitcher |
| 15 | RP | Relief Pitcher |
| 16 | BE | Bench |
| 17 | IL | Injured List |
| 19 | IF | Infield (combo slot) |

**Note:** Slot IDs 18, 21, and 22 have been observed in ESPN data but their mappings are unknown.

### Player Data Structure

Each player object in the ESPN API contains three position-related fields:

1. **`defaultPositionId`** (1-indexed): The player's primary/default position. Uses 1-based indexing, so Catcher = 1, mapped to `POSITION_MAP[0]` = 'C'.

2. **`eligibleSlots`** (0-indexed array): An array of slot IDs where the player can legally be placed. This includes their actual position slots AND generic/flex slots. For example, a player eligible at 2B and SS might have `eligibleSlots: [2, 4, 6, 12, 16, 17]` (2B, SS, 2B/SS, UTIL, BE, IL).

3. **`lineupSlotId`** (0-indexed): The slot ID where the player is currently placed on the roster. Uses the same ID space as `eligibleSlots`.

### Key Insight: `eligibleSlots` Is the Source of Truth

ESPN pre-computes the full list of legal placements for every player. The `eligibleSlots` array already includes:
- All position slots the player qualifies at (based on games played rules)
- All applicable flex/combo slots (UTIL, 2B/SS, 1B/3B, IF, P, OF)
- Bench (BE) and Injured List (IL)

This means **ESPN does the eligibility computation server-side** and exposes the result as a flat array. Any client-side logic only needs to check `eligibleSlots.includes(slotId)` to determine if a placement is legal.

---

## 3. How Slot Assignment Works During an Auction Draft

### Draft-Time Behavior

During an ESPN auction draft:

1. **Any manager can nominate any available player.** The nominated player is shown in the "On the Block" area with their position eligibility listed.

2. **Bidding proceeds.** All teams with sufficient budget can bid. The system enforces that you cannot bid on a player if you have no legal roster slot to place them in (all eligible position slots, bench slots, etc. are full).

3. **When a player is won,** ESPN places them into a roster slot. The exact auto-placement algorithm is not publicly documented, but observed behavior suggests:
   - The player is placed at their **default position** first if a slot is available
   - If the default position slot is full, ESPN tries other eligible position slots
   - If all position slots are full, the player goes to Bench
   - The owner can rearrange roster slot assignments after the draft (or between picks)

4. **Position maximums are enforced.** If all slots where a player could legally go are filled, the system prevents bidding on that player entirely.

### What the Extension Can Scrape

The ESPN draft page is a React application. From the DOM and/or React state, the extension can potentially extract:

- **Player `eligibleSlots` array:** Available via the player objects in React state (accessed through `__reactInternalInstance$` fiber tree or network API responses)
- **Current roster `lineupSlotId` for each rostered player:** Shows which slot each player is currently assigned to
- **League roster configuration:** Which slot types the league uses (OF vs LF/CF/RF, etc.)

---

## 4. Analysis of Current Local Logic vs ESPN Runtime Data

### What Our Current `roster.rs` Does

The current implementation in `roster.rs`:

1. **Single-position model:** Each player is recorded with a single `Position` enum value (from `pick.rs`). The position string from ESPN (e.g., "CF") is parsed via `Position::from_str_pos()`.

2. **Slot assignment priority:** Dedicated slot -> Outfield cross-slot (for OF positions) -> UTIL (hitters only) -> Bench.

3. **Outfield handling:** LF/CF/RF players can fill any of the three OF slots, which is correct for our league's individual outfield position configuration.

4. **Generic "OF" mapping:** `"OF"` maps to `CenterField`, which is a lossy conversion -- an "OF"-eligible player should potentially fill LF, CF, or RF, but the current code routes them to CF specifically.

### Gaps and Risks in Local Logic

| Issue | Severity | Description |
|-------|----------|-------------|
| **No multi-position awareness** | High | A player listed as "2B/SS" on ESPN is recorded as only the first position parsed. If ESPN sends "2B" as the position string, we miss SS eligibility entirely. The player might fit in an open SS slot but our logic won't try it. |
| **"OF" -> CF is lossy** | Medium | When ESPN reports "OF", mapping it to CF means we always try CF first, then cross-fill to LF/RF. If the CF slot is taken but LF is open, it works (due to cross-slot logic). But if the player is actually eligible at CF only (not LF/RF), our cross-slot logic could produce wrong results. In practice, ESPN's OF eligibility means all three, so this is mostly fine -- but it's fragile. |
| **No combo/flex slot support** | Low | Our league doesn't use 2B/SS, 1B/3B, IF, or P combo slots, so this is currently moot. But if league settings change, the local logic won't handle them. |
| **No DH slot** | Low | The league roster config has no DH slot, but our `Position` enum includes `DesignatedHitter`. Players with DH eligibility are correctly routed through UTIL->BE fallback, which matches ESPN's behavior for leagues without a DH slot. |
| **Bid eligibility check** | Medium | Our `has_empty_slot()` and `add_player()` only check for exact position match + UTIL + BE. ESPN's system checks `eligibleSlots` comprehensively. If our logic says "no slot available" but ESPN would allow it (or vice versa), the assistant gives wrong advice about which players can still be drafted. |

### What ESPN Exposes at Runtime

From the draft page, the extension could extract:
- `player.eligibleSlots` -- the definitive list of where each player can be placed
- `team.roster[].lineupSlotId` -- where each player is currently slotted
- The league's roster slot configuration -- which slot types are in use

This data would let us bypass all local eligibility computation entirely.

---

## 5. Recommendation: Hybrid Approach

### Summary

**Use ESPN's `eligibleSlots` as the source of truth for eligibility, but keep local roster tracking for state management and analysis.**

### Detailed Approach

#### Phase 1: Extend the Protocol (Extension -> Backend)

Modify the extension scraper and `PickData`/`NominationData` protocol types to include:

```
eligibleSlots: [number]   // ESPN slot IDs from player.eligibleSlots
```

When the extension scrapes a nomination or completed pick, it should include the player's full `eligibleSlots` array from ESPN's data. This is the single most impactful change -- it gives the backend ESPN's authoritative eligibility data.

#### Phase 2: Update the Position Model

1. **Add `eligible_slots: Vec<u16>` to `RosteredPlayer` and `DraftPick`.** Store the raw ESPN slot IDs alongside the display position string.

2. **Add a slot ID -> Position mapping** that mirrors ESPN's `POSITION_MAP`. Use this to translate between ESPN's numeric IDs and our `Position` enum. Handle combo slots (2B/SS, 1B/3B, IF, P, OF) by mapping them to a new `SlotType` enum or by expanding the existing `Position` enum.

3. **Update `Roster::add_player()` to accept `eligible_slots`** and use them instead of single-position matching. The assignment priority becomes:
   - Try each eligible position slot in league roster order
   - Try UTIL
   - Try Bench

#### Phase 3: Keep Local Logic as Fallback

The local single-position logic in `roster.rs` should remain as a **fallback** for situations where `eligibleSlots` is unavailable:
- Manual picks entered via the TUI (user types "Mike Trout, CF, $45")
- Crash recovery / replay from saved picks that lack slot data
- Pre-draft roster analysis where we only have projection data with a single listed position

In these cases, the current heuristic (dedicated slot -> cross-OF -> UTIL -> BE) is a reasonable approximation.

### Why Not Defer Entirely to ESPN?

Fully deferring to ESPN (reading roster state from the DOM on every update) was considered but rejected because:

1. **Latency:** Waiting for the extension to scrape and send updated roster state after every pick adds delay. Local tracking gives instant state updates.
2. **Analysis requirements:** The valuation engine, scarcity tracker, and LLM prompt builder all need roster state. Local tracking makes this data immediately available without waiting for DOM scrapes.
3. **Offline/pre-draft planning:** The assistant needs to simulate roster construction before the draft starts. This requires local logic that works without a live ESPN connection.
4. **Resilience:** If the extension disconnects mid-draft, local state tracking keeps the assistant functional.

### Why Not Keep Pure Local Logic?

Pure local logic was considered but rejected because:

1. **Multi-position eligibility is the primary risk.** The single biggest failure mode is recommending a player to fill a positional need they can't actually slot into. ESPN's `eligibleSlots` eliminates this class of bugs entirely.
2. **ESPN's rules are complex and change.** The 20-game/25% threshold, in-season 10-game additions, DH eligibility, specific OF vs generic OF -- maintaining parity with all these rules locally is error-prone and requires updates whenever ESPN changes its policies.
3. **The data is readily available.** The extension is already scraping the draft page. Adding `eligibleSlots` to the scraped data is trivial.

---

## 6. Implementation Impact

### Files to Modify

| File | Change |
|------|--------|
| `src/protocol.rs` | Add `eligible_slots: Vec<u16>` to `PickData` and `NominationData` |
| `src/draft/pick.rs` | Add `eligible_slots: Vec<u16>` to `DraftPick`; add ESPN slot ID constants and mapping functions |
| `src/draft/roster.rs` | Add `add_player_with_slots()` method that uses `eligibleSlots` for placement; keep `add_player()` as fallback |
| `src/draft/state.rs` | Thread `eligible_slots` through `record_pick()` and state diff logic |
| `extension/content_scripts/espn.js` | Scrape `eligibleSlots` from player objects in React state / API responses |
| `config/league.toml` | (No change needed -- roster slot config already defines which slots the league uses) |

### Estimated Effort

- Protocol and data model changes: Small (~1-2 hours)
- Roster placement logic update: Medium (~2-3 hours, including tests)
- Extension scraping update: Small (~1 hour, depends on Task 18 progress)
- Total: ~4-6 hours of implementation work

### Risk Mitigation

- **Graceful degradation:** If `eligible_slots` is empty (manual entry, old data), fall back to single-position logic
- **Validation:** Log warnings when local logic disagrees with ESPN's `eligibleSlots` to detect drift
- **Testing:** Add tests with real multi-position player scenarios (e.g., Mookie Betts as SS/2B/OF)

---

## Sources

- [ESPN Position Eligibility (Fan Support)](https://support.espn.com/hc/en-us/articles/360000093592-Position-Eligibility)
- [ESPN Fantasy Baseball: New Default Player Eligibility Rules (2024)](https://www.espn.com/fantasy/baseball/story/_/id/39293536/espn-fantasy-baseball-new-default-player-eligibility-rules-2024)
- [ESPN Fantasy Baseball: New Positional Eligibility Rule Explained](https://www.espn.com/fantasy/baseball/story/_/id/36286893/fantasy-baseball-espn-new-positional-eligibility-rule-explained)
- [espn-api Python library (cwendt94/espn-api)](https://github.com/cwendt94/espn-api) -- Community reverse-engineering of ESPN Fantasy API v3
- [espn-api baseball constants](https://github.com/cwendt94/espn-api/blob/master/espn_api/baseball/constant.py) -- POSITION_MAP and slot ID definitions
- [ESPN Auction Draft Overview](https://www.espn.com/fantasy/baseball/flb/story?page=flbrulesauctionoverview2008)
- [Value of Multi-Position Eligibility (Gramling)](https://www.espn.com/fantasy/baseball/story/_/id/8972347/many-players-eligibility-multiple-positions-fantasy-baseball-some-players-much-more-valuable-one-spot-another)
