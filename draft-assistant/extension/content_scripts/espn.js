// ESPN Draft Page Content Script
// Scrapes draft state from ESPN's DOM and relays it to the background script.
// Communicates with the background script via browser.runtime.sendMessage.

'use strict';

// ---------------------------------------------------------------------------
// ESPN slot ID constants (from ESPN Fantasy API v3)
// Must stay in sync with the constants in src/draft/pick.rs.
// ---------------------------------------------------------------------------

const ESPN_SLOT_C    = 0;
const ESPN_SLOT_1B   = 1;
const ESPN_SLOT_2B   = 2;
const ESPN_SLOT_3B   = 3;
const ESPN_SLOT_SS   = 4;
const ESPN_SLOT_OF   = 5;  // generic OF combo
const ESPN_SLOT_MI   = 6;  // 2B/SS combo
const ESPN_SLOT_CI   = 7;  // 1B/3B combo
const ESPN_SLOT_LF   = 8;
const ESPN_SLOT_CF   = 9;
const ESPN_SLOT_RF   = 10;
const ESPN_SLOT_DH   = 11;
const ESPN_SLOT_UTIL = 12;
const ESPN_SLOT_P    = 13; // generic pitcher combo
const ESPN_SLOT_SP   = 14;
const ESPN_SLOT_RP   = 15;
const ESPN_SLOT_BE   = 16;
const ESPN_SLOT_IL   = 17;

/**
 * Map a position string from the ESPN draft page to the corresponding ESPN
 * slot ID.  Returns null for unrecognised strings.
 *
 * This mirrors espn_slot_from_position() / Position::from_str_pos() in
 * src/draft/pick.rs and is used to convert the position badge shown next to
 * a completed pick in the draft log into the authoritative slot ID that ESPN
 * assigned the player to.  That slot ID is forwarded to the Rust backend as
 * `assignedSlot` so that two-way players like Ohtani land in the correct
 * roster slot (e.g. UTIL) rather than the slot inferred from their primary
 * position (e.g. SP).
 */
function espnSlotIdFromPositionStr(posStr) {
  if (!posStr) return null;
  switch (posStr.toUpperCase()) {
    case 'C':    return ESPN_SLOT_C;
    case '1B':   return ESPN_SLOT_1B;
    case '2B':   return ESPN_SLOT_2B;
    case '3B':   return ESPN_SLOT_3B;
    case 'SS':   return ESPN_SLOT_SS;
    case 'LF':   return ESPN_SLOT_LF;
    case 'CF':   return ESPN_SLOT_CF;
    case 'RF':   return ESPN_SLOT_RF;
    case 'OF':   return ESPN_SLOT_OF;
    case 'DH':   return ESPN_SLOT_DH;
    case 'UTIL': return ESPN_SLOT_UTIL;
    case 'SP':   return ESPN_SLOT_SP;
    case 'RP':   return ESPN_SLOT_RP;
    case 'P':    return ESPN_SLOT_P;
    case 'BE':
    case 'BN':   return ESPN_SLOT_BE;
    case 'IL':
    case 'DL':   return ESPN_SLOT_IL;
    default:     return null;
  }
}

// ---------------------------------------------------------------------------
// ESPN DOM selectors (verified against live ESPN auction draft page)
// ---------------------------------------------------------------------------

const SELECTORS = {
  // Top-level draft container
  draftContainer: 'div.draft-content-wrapper',

  // Pick counter (e.g. "PK 128 OF 260")
  pickLabel: 'div.clock__label',

  // Clock digits: 4 spans inside div.clock__digits for MM:SS
  clockDigits: 'div.clock__digits > span.clock__digit',

  // Team budget list (pick train carousel)
  teamBudgetItems: 'ul.picklist > li.picklist--item',
  teamBudgetName: 'div.team-name.truncate',
  teamBudgetCash: 'div.cash',

  // Current nomination / bidding area
  nominationContainer: 'div.pickArea',
  playerSelected: 'div[data-testid="player-selected"]',
  nominationPlayerName: 'span.playerinfo__playername',
  nominationPlayerPos: 'span.playerinfo__playerpos',
  nominationCurrentOffer: 'div.current-amount',
  biddingForm: 'div[data-testid="bidding-form"]',

  // Bid history within nomination area (primary and fallback selectors)
  bidHistoryItems: 'ul.bid-history__list > li.bid',
  bidHistoryItemsFallback: 'ul.bid-history__list > li',

  // Currently-nominating team in the pick train (highlighted/active item)
  nominatingTeamItem: 'li.picklist--item.is-nominating, li.picklist--item.active, li.picklist--item.is-current',

  // Pick history / draft log (right column)
  pickLogEntries: 'li.pick-message__container',
  pickLogPlayerName: 'span.playerinfo__playername',
  pickLogPlayerPos: 'span.playerinfo__playerpos',
  pickLogInfo: 'div.pick-info',

  // My team identification via pick train
  myTeamContent: 'div.content.auction-pick-component--own',
};

// Timing constants
const MUTATION_DEBOUNCE_MS = 250;
const POLL_INTERVAL_MS = 500;
const POLL_TIMEOUT_MS = 10000;
const FALLBACK_POLL_INTERVAL_MS = 1000;
const KEYFRAME_INTERVAL_MS = 10000;

// ---------------------------------------------------------------------------
// Logging utility
// ---------------------------------------------------------------------------

const LOG_PREFIX = '[WyndhamDraftSync]';

function log(...args) {
  console.log(LOG_PREFIX, ...args);
}

function warn(...args) {
  console.warn(LOG_PREFIX, ...args);
}

function error(...args) {
  console.error(LOG_PREFIX, ...args);
}

// ---------------------------------------------------------------------------
// DOM Scraping (primary extraction method)
// ---------------------------------------------------------------------------

/**
 * Extract text content from an element matched by a selector within a parent.
 */
function extractText(parent, selector) {
  if (!parent || !selector) return '';
  try {
    const el = parent.querySelector(selector);
    return el ? el.textContent.trim() : '';
  } catch (e) {
    return '';
  }
}

/**
 * Parse a price string like "$42", "$5 - Team Name", or "42" into a number.
 */
function parsePrice(priceStr) {
  if (!priceStr) return 0;
  const cleaned = priceStr.replace(/[^0-9]/g, '');
  return parseInt(cleaned, 10) || 0;
}

/**
 * Parse the clock digits from the 4 span elements into seconds remaining.
 * Clock format: MM:SS displayed as 4 separate span.clock__digit elements.
 */
function parseClockDigits() {
  try {
    const digits = document.querySelectorAll(SELECTORS.clockDigits);
    if (digits.length >= 4) {
      const mm = parseInt(digits[0].textContent + digits[1].textContent, 10) || 0;
      const ss = parseInt(digits[2].textContent + digits[3].textContent, 10) || 0;
      return mm * 60 + ss;
    }
  } catch (e) {
    // Clock not available
  }
  return null;
}

/**
 * Parse "PK 128 OF 260" from the clock label into { current, total }.
 */
function parsePickLabel() {
  try {
    const label = document.querySelector(SELECTORS.pickLabel);
    if (label) {
      const match = label.textContent.match(/PK\s+(\d+)\s+OF\s+(\d+)/i);
      if (match) {
        return {
          current: parseInt(match[1], 10),
          total: parseInt(match[2], 10),
        };
      }
    }
  } catch (e) {
    // Label not available
  }
  return null;
}

/**
 * Parse a pick-info string like "$5 - Boscolo Colon" into { price, teamName }.
 */
function parsePickInfo(infoStr) {
  if (!infoStr) return { price: 0, teamName: '' };
  // Format: "$5 - Team Name" or "$15 - Team Name"
  const match = infoStr.match(/^\$(\d+)\s*-\s*(.+)$/);
  if (match) {
    return {
      price: parseInt(match[1], 10) || 0,
      teamName: match[2].trim(),
    };
  }
  // Fallback: try to extract just the price
  return {
    price: parsePrice(infoStr),
    teamName: infoStr.replace(/\$\d+\s*-?\s*/, '').trim(),
  };
}

/**
 * Parse "Current offer: $2" into a number.
 */
function parseCurrentOffer(offerStr) {
  if (!offerStr) return 0;
  const match = offerStr.match(/\$(\d+)/);
  return match ? parseInt(match[1], 10) : 0;
}

/**
 * Extract the current bidder from a pre-queried bid history NodeList.
 * The first entry in the list is the most recent bid.
 */
function extractCurrentBidder(bidItems) {
  try {
    if (bidItems && bidItems.length > 0) {
      // First item is most recent bid: e.g. "$2 Jamaica Jiggle Party"
      const text = bidItems[0].textContent.trim();
      // Parse "$N TeamName" format
      const match = text.match(/^\$\d+\s+(.+)$/);
      return match ? match[1].trim() : text;
    }
  } catch (e) {
    // Bid history not available
  }
  return null;
}

/**
 * Extract the nominating team from a pre-queried bid history NodeList.
 * The last entry in the bid history list is the original nomination.
 */
function extractNominatedBy(bidItems) {
  try {
    if (bidItems && bidItems.length > 0) {
      // Last item is the original nomination
      const lastItem = bidItems[bidItems.length - 1];
      const text = lastItem.textContent.trim();
      const match = text.match(/^\$\d+\s+(.+)$/);
      return match ? match[1].trim() : text;
    }
  } catch (e) {
    // Bid history not available
  }
  return '';
}

/**
 * Fallback nominator extraction: try to identify the currently-nominating
 * team from the pick train carousel. ESPN highlights the active nominator
 * with a CSS class modifier (e.g. is-nominating, active, is-current).
 * Returns the team name, or empty string if not found.
 */
function extractNominatingTeamFromPickTrain() {
  try {
    const nominatingItem = document.querySelector(SELECTORS.nominatingTeamItem);
    if (nominatingItem) {
      const nameEl = nominatingItem.querySelector(SELECTORS.teamBudgetName);
      if (nameEl) {
        const name = nameEl.textContent.trim();
        // Strip the leading number prefix (e.g. "3. Team Name" -> "Team Name")
        return name.replace(/^\d+\.\s*/, '');
      }
    }
  } catch (e) {
    // Pick train nominator extraction failed
  }
  return '';
}

/**
 * Extract team budgets from the pick train carousel.
 * Returns an array of { teamId, teamName, budget } objects.
 * The teamId is extracted from the leading number in the team name (e.g. "1. London Ligers" -> "1").
 */
function scrapeTeamBudgets() {
  const teams = [];
  try {
    const items = document.querySelectorAll(SELECTORS.teamBudgetItems);
    items.forEach((item) => {
      const name = extractText(item, SELECTORS.teamBudgetName);
      const cashStr = extractText(item, SELECTORS.teamBudgetCash);
      if (name) {
        // Extract the leading number as teamId and strip it from the name
        const match = name.match(/^(\d+)\.\s*(.*)/);
        const teamId = match ? match[1] : '';
        const cleanName = match ? match[2] : name;
        teams.push({
          teamId: teamId,
          teamName: cleanName,
          budget: parsePrice(cashStr),
        });
      }
    });
  } catch (e) {
    error('Error scraping team budgets:', e);
  }
  return teams;
}

/**
 * Identify my team from the pick train using the own-team modifier class.
 * Returns the team name, or null if not found.
 */
function identifyMyTeam() {
  try {
    const ownContent = document.querySelector(SELECTORS.myTeamContent);
    if (ownContent) {
      const nameEl = ownContent.querySelector(SELECTORS.teamBudgetName);
      if (nameEl) {
        const name = nameEl.textContent.trim();
        return name.replace(/^\d+\.\s*/, '');
      }
    }
  } catch (e) {
    // Could not identify own team
  }
  return null;
}

/**
 * Scrape completed picks from the draft log (right column).
 * Pick log entries are in reverse chronological order (most recent first).
 * Returns picks in chronological order (oldest first) with pick numbers.
 *
 * ESPN virtualizes the pick list, so only a window of recent picks may be
 * present in the DOM. We use the pick counter label ("PK 128 OF 260") to
 * compute correct absolute pick numbers instead of relying on array index.
 */
function scrapePickLog() {
  const picks = [];
  try {
    const entries = document.querySelectorAll(SELECTORS.pickLogEntries);
    // Entries are most-recent-first; we want chronological order
    const entriesArray = Array.from(entries).reverse();

    // Use ESPN's pick counter to compute absolute pick numbers.
    // The pick label shows the CURRENT nomination (in-progress), e.g.
    // "PK 2 OF 260" means pick #2 is being nominated, so only 1 pick is
    // complete. Subtract 1 to get the count of completed picks.
    // If the label says "PK 128 OF 260" and we have 30 entries visible,
    // they represent picks 98-127 (not 1-30).
    const pickLabel = parsePickLabel();
    const completedPicks = pickLabel
      ? Math.max(pickLabel.current - 1, entriesArray.length)
      : entriesArray.length;

    entriesArray.forEach((entry, idx) => {
      const playerName = extractText(entry, SELECTORS.pickLogPlayerName);
      let position = extractText(entry, SELECTORS.pickLogPlayerPos);
      const pickInfoStr = extractText(entry, SELECTORS.pickLogInfo);

      // Handle compound position strings like "SP, RP" — take only the first
      if (position.includes(',')) {
        position = position.split(',')[0].trim();
      }

      if (playerName) {
        const { price, teamName } = parsePickInfo(pickInfoStr);
        // Map the position string scraped from the pick log to the ESPN slot
        // ID that ESPN assigned the player to.  This is the authoritative
        // placement slot — e.g. for Ohtani drafted to UTIL the badge reads
        // "UTIL" and we forward slot ID 12 so the backend places him there
        // instead of inferring SP from his eligible positions.
        const assignedSlot = espnSlotIdFromPositionStr(position);
        picks.push({
          pickNumber: completedPicks - entriesArray.length + idx + 1,
          teamId: teamName,
          teamName: teamName,
          playerId: '',
          playerName: playerName,
          position: position,
          price: price,
          eligibleSlots: [],
          assignedSlot: assignedSlot,
        });
      }
    });
  } catch (e) {
    error('Error scraping pick log:', e);
  }
  return picks;
}

/**
 * Scrape the current nomination from the pick area.
 *
 * Only returns a nomination when it is in the active "offer" (bidding) stage.
 * During the pre-nomination phase the nominator is browsing/selecting a player
 * and ESPN shows a player card without a bidding form or bid history. Treating
 * that as an active nomination would trigger premature LLM analysis and UI
 * updates for a pick that may never happen or may change.
 *
 * The bidding form (`data-testid="bidding-form"`) and bid history entries are
 * only present once the nomination is confirmed and bidding has started. We
 * require at least one of these signals before reporting a nomination.
 */
function scrapeCurrentNomination() {
  try {
    const pickArea = document.querySelector(SELECTORS.nominationContainer);
    if (!pickArea) return null;

    const playerSelected = pickArea.querySelector(SELECTORS.playerSelected);
    if (!playerSelected) return null;

    const playerName = extractText(playerSelected, SELECTORS.nominationPlayerName);
    if (!playerName) return null;

    // Query bid history scoped to the pickArea, not the entire document.
    // Try the primary selector first, then a fallback in case ESPN uses
    // a different class on the <li> elements.
    let bidItems = pickArea.querySelectorAll(SELECTORS.bidHistoryItems);
    if (bidItems.length === 0) {
      bidItems = pickArea.querySelectorAll(SELECTORS.bidHistoryItemsFallback);
    }

    // Check that the nomination is in the active "offer" stage.
    // During the pre-nomination phase (nominator browsing/selecting), the
    // player card is visible but there is no bidding form and no bid history.
    // We require either a bidding form or bid history entries to confirm the
    // nomination is real. This prevents premature analysis triggers.
    const hasBiddingForm = !!pickArea.querySelector(SELECTORS.biddingForm);
    const hasBidHistory = bidItems.length > 0;
    if (!hasBiddingForm && !hasBidHistory) {
      console.debug(LOG_PREFIX, 'Skipping premature nomination for:', playerName);
      return null;
    }

    let position = extractText(playerSelected, SELECTORS.nominationPlayerPos);
    const offerStr = extractText(playerSelected, SELECTORS.nominationCurrentOffer);
    const currentBid = parseCurrentOffer(offerStr);
    const timeRemaining = parseClockDigits();

    // Handle compound position strings like "SP, RP" — take only the first
    if (position.includes(',')) {
      position = position.split(',')[0].trim();
    }

    const currentBidder = extractCurrentBidder(bidItems);
    let nominatedBy = extractNominatedBy(bidItems);

    // Fallback: if bid history hasn't rendered yet but a bidding form is
    // present, try to identify the nominator from the pick train. ESPN
    // highlights the currently-nominating team with a CSS modifier class.
    if (!nominatedBy) {
      nominatedBy = extractNominatingTeamFromPickTrain();
    }

    return {
      playerId: '',
      playerName: playerName,
      position: position,
      nominatedBy: nominatedBy,
      currentBid: currentBid,
      currentBidder: currentBidder,
      timeRemaining: timeRemaining,
      eligibleSlots: [],
    };
  } catch (e) {
    error('Error scraping nomination:', e);
  }
  return null;
}

/**
 * Scrape complete draft state from the DOM.
 */
function scrapeDom() {
  const state = {
    picks: [],
    currentNomination: null,
    myTeamId: null,
    teams: [],
    pickCount: null,
    totalPicks: null,
    draftId: null,
    source: 'dom_scrape',
  };

  try {
    // Scrape completed picks from the draft log
    state.picks = scrapePickLog();

    // Scrape current nomination
    state.currentNomination = scrapeCurrentNomination();

    // Scrape team budgets from pick train
    state.teams = scrapeTeamBudgets();

    // Parse pick counter label (e.g. "PK 128 OF 260")
    const pickLabel = parsePickLabel();
    if (pickLabel) {
      state.pickCount = pickLabel.current;
      state.totalPicks = pickLabel.total;
    }

    // Identify my team
    const myTeamName = identifyMyTeam();
    if (myTeamName) {
      // Use team name as ID since ESPN DOM doesn't expose numeric team IDs
      state.myTeamId = myTeamName;
    }

    // Extract draft identifier
    state.draftId = scrapeDraftId();
  } catch (e) {
    error('DOM scraping error:', e);
  }

  return state;
}

// ---------------------------------------------------------------------------
// Draft identifier extraction
// ---------------------------------------------------------------------------

/**
 * Extract a stable draft identifier from the ESPN page.
 *
 * Combines the leagueId from the URL query string with the current year
 * to produce a stable identifier per league per season.
 * ESPN draft URLs look like: https://fantasy.espn.com/baseball/draft?leagueId=12345
 *
 * Returns a string like "espn_12345_2026", or null if leagueId is not in the URL.
 */
function scrapeDraftId() {
  try {
    const params = new URLSearchParams(window.location.search);
    const leagueId = params.get('leagueId');
    if (leagueId) {
      // NOTE: Uses the current calendar year. This means a draft that
      // hypothetically spans midnight on Dec 31 would produce a different
      // ID after the year rolls over. In practice this is not an issue
      // because baseball drafts never cross the year boundary.
      const year = new Date().getFullYear();
      return 'espn_' + leagueId + '_' + year;
    }
  } catch (e) {
    // URL parsing failed
  }
  return null;
}

// ---------------------------------------------------------------------------
// React State Extraction (via page-context script injection)
// ---------------------------------------------------------------------------

// Cached team rosters received from the injected page-context script.
// Updated asynchronously via window.postMessage from the injected script.
let cachedTeamRosters = null;

/**
 * Set up a listener for messages from the injected page-context script.
 * The injected script posts results via window.postMessage with a known source.
 */
function setupPageContextListener() {
  window.addEventListener('message', (event) => {
    // Only accept messages from the same window (page-context script)
    if (event.source !== window) return;
    if (!event.data || event.data.source !== 'wyndham-draft-sync-page-extract') return;

    if (event.data.error) {
      warn('Page-context extraction error:', event.data.error);
      cachedTeamRosters = null;
      return;
    }

    const rosters = event.data.teamRosters;
    if (rosters && Array.isArray(rosters)) {
      const totalPlayers = rosters.reduce((sum, t) => sum + (t.players ? t.players.length : 0), 0);
      log('Received team rosters from page-context script:', totalPlayers, 'players across', rosters.length, 'teams');
      cachedTeamRosters = rosters;
    } else {
      log('Page-context script returned no roster data');
      cachedTeamRosters = null;
    }
  });
  log('Page-context message listener registered');
}

/**
 * Inject a script element into the page that runs in the PAGE's JavaScript
 * context (not the content script sandbox). In the page context, React fiber
 * keys ARE visible on DOM elements via Object.keys().
 *
 * The injected script:
 * - Finds a React root element
 * - Walks the React fiber tree looking for draft state (draftDetail + teams)
 * - Builds team rosters in the format expected by the Rust backend
 * - Posts the result back via window.postMessage
 *
 * No network calls are made — it only reads React state from memory.
 */
function injectPageContextExtractor() {
  try {
    const script = document.createElement('script');
    script.textContent = `(${pageContextExtractorIIFE.toString()})();`;
    (document.documentElement || document.head || document.body).appendChild(script);
    script.remove();
  } catch (e) {
    warn('Failed to inject page-context extractor script:', e.message || e);
  }
}

/**
 * The IIFE that runs in the page's JavaScript context.
 * Defined as a named function so we can .toString() it for injection.
 * Everything inside runs with full access to the page's JS objects.
 */
function pageContextExtractorIIFE() {
  var LOG = '[WyndhamDraftSync:PageCtx]';

  try {
    // ---- Helper: find React fiber key on a DOM element ----
    function findReactFiber(element) {
      if (!element) return null;
      var keys = Object.keys(element);
      for (var i = 0; i < keys.length; i++) {
        if (keys[i].startsWith('__reactFiber$') || keys[i].startsWith('__reactInternalInstance$')) {
          return element[keys[i]];
        }
      }
      return null;
    }

    // ---- Helper: check if an object contains draft data ----
    function checkObjectForDraftData(obj, label, depth) {
      if (!obj || typeof obj !== 'object') return null;
      try {
        // Look for draftDetail.picks (most reliable indicator)
        if (obj.draftDetail && Array.isArray(obj.draftDetail.picks) && obj.draftDetail.picks.length > 0) {
          console.log(LOG, 'Found draftDetail at', label, 'depth', depth, 'with', obj.draftDetail.picks.length, 'picks');
          if (Array.isArray(obj.teams)) {
            return obj;
          }
        }
        // Check nested: obj might wrap the data one level deeper
        var objKeys = Object.keys(obj);
        for (var i = 0; i < objKeys.length; i++) {
          var key = objKeys[i];
          try {
            var val = obj[key];
            if (val && typeof val === 'object' && !Array.isArray(val)) {
              if (val.draftDetail && Array.isArray(val.draftDetail.picks) && val.draftDetail.picks.length > 0) {
                console.log(LOG, 'Found draftDetail at', label + '.' + key, 'depth', depth, 'with', val.draftDetail.picks.length, 'picks');
                if (Array.isArray(val.teams)) {
                  return val;
                }
              }
            }
          } catch (e) { /* skip */ }
        }
      } catch (e) { /* skip */ }
      return null;
    }

    // ---- Helper: walk React fiber tree from an element ----
    function extractFromReactFiber(element) {
      var fiber = findReactFiber(element);
      if (!fiber) {
        console.log(LOG, 'No React fiber found on', element.tagName, element.className || '');
        return null;
      }
      console.log(LOG, 'Found React fiber on', element.tagName, element.className || '');

      var current = fiber;
      var depth = 0;
      var maxDepth = 50;

      while (current && depth < maxDepth) {
        try {
          var state = current.memoizedState;
          var props = current.memoizedProps;
          var stateNode = current.stateNode;

          // Check props
          if (props) {
            var result = checkObjectForDraftData(props, 'props', depth);
            if (result) return result;
          }

          // Check class component state
          if (stateNode && stateNode.state) {
            var result = checkObjectForDraftData(stateNode.state, 'stateNode.state', depth);
            if (result) return result;
          }

          // Check stateNode for Redux store
          if (stateNode && typeof stateNode === 'object' && stateNode !== null) {
            if (typeof stateNode.getState === 'function') {
              try {
                var storeState = stateNode.getState();
                var result = checkObjectForDraftData(storeState, 'redux-store', depth);
                if (result) return result;
              } catch (e) { /* skip */ }
            }
          }

          // Check hooks state chain (linked list)
          if (state && typeof state === 'object') {
            var hookState = state;
            var hookIdx = 0;
            while (hookState && hookIdx < 20) {
              if (hookState.memoizedState && typeof hookState.memoizedState === 'object') {
                var result = checkObjectForDraftData(hookState.memoizedState, 'hook[' + hookIdx + ']', depth);
                if (result) return result;
              }
              hookState = hookState.next;
              hookIdx++;
            }
          }
        } catch (e) { /* skip inaccessible fibers */ }

        current = current.return;
        depth++;
      }

      console.log(LOG, 'React fiber walk: no draft state found in', depth, 'levels');
      return null;
    }

    // ---- Helper: build team rosters from React state ----
    function buildTeamRosters(stateObj) {
      var teams = stateObj.teams;
      var draftDetail = stateObj.draftDetail;
      if (!Array.isArray(teams) || teams.length === 0) return null;

      // Build price map from draftDetail.picks
      var priceMap = {};
      if (draftDetail && Array.isArray(draftDetail.picks)) {
        for (var i = 0; i < draftDetail.picks.length; i++) {
          var pick = draftDetail.picks[i];
          if (pick && pick.playerId != null && pick.bidAmount != null) {
            priceMap[pick.playerId] = pick.bidAmount;
          }
        }
      }

      // Build team info map
      var teamInfoMap = {};
      for (var i = 0; i < teams.length; i++) {
        var team = teams[i];
        if (!team) continue;
        var teamName = '';
        if (team.location && team.nickname) {
          teamName = (team.location + ' ' + team.nickname).trim();
        } else if (team.name) {
          teamName = team.name;
        } else if (team.abbrev) {
          teamName = team.abbrev;
        } else {
          teamName = 'Team ' + (team.id || '?');
        }
        if (team.id != null) {
          teamInfoMap[team.id] = teamName;
        }
      }

      // Build from teams[].roster.entries (primary)
      var result = [];
      var totalPlayers = 0;

      for (var i = 0; i < teams.length; i++) {
        var team = teams[i];
        if (!team) continue;
        var teamId = team.id != null ? team.id : 0;
        var teamName = teamInfoMap[teamId] || ('Team ' + teamId);
        var players = [];

        if (team.roster && Array.isArray(team.roster.entries)) {
          for (var j = 0; j < team.roster.entries.length; j++) {
            var entry = team.roster.entries[j];
            if (!entry) continue;
            var playerId = entry.playerId != null ? String(entry.playerId) : '';
            var lineupSlotId = entry.lineupSlotId != null ? entry.lineupSlotId : 16;

            var playerName = '';
            var eligibleSlots = [];
            var ppe = entry.playerPoolEntry;
            if (ppe && ppe.player) {
              var player = ppe.player;
              playerName = player.fullName || ((player.firstName || '') + ' ' + (player.lastName || '')).trim();
              eligibleSlots = Array.isArray(player.eligibleSlots) ? player.eligibleSlots.slice() : [];
            }

            var numericId = entry.playerId != null ? entry.playerId : parseInt(playerId, 10);
            var price = priceMap[numericId] != null ? priceMap[numericId] : 0;

            if (playerName) {
              players.push({ playerId: playerId, playerName: playerName, lineupSlotId: lineupSlotId, eligibleSlots: eligibleSlots, price: price });
            }
          }
        }

        result.push({ teamId: teamId, teamName: teamName, players: players });
        totalPlayers += players.length;
      }

      if (totalPlayers > 0) {
        console.log(LOG, 'Built rosters from roster.entries:', totalPlayers, 'players across', result.length, 'teams');
        return result;
      }

      // Fallback: build from draftDetail.picks
      if (!draftDetail || !Array.isArray(draftDetail.picks) || draftDetail.picks.length === 0) {
        return null;
      }

      console.log(LOG, 'roster.entries had 0 players, falling back to draftDetail.picks');

      // Build player info map from roster entries (might have partial data)
      var playerInfoMap = {};
      for (var i = 0; i < teams.length; i++) {
        var team = teams[i];
        if (!team || !team.roster || !Array.isArray(team.roster.entries)) continue;
        for (var j = 0; j < team.roster.entries.length; j++) {
          var entry = team.roster.entries[j];
          if (!entry || entry.playerId == null) continue;
          var ppe = entry.playerPoolEntry;
          if (ppe && ppe.player) {
            var player = ppe.player;
            var name = player.fullName || ((player.firstName || '') + ' ' + (player.lastName || '')).trim();
            var es = Array.isArray(player.eligibleSlots) ? player.eligibleSlots.slice() : [];
            if (name) playerInfoMap[entry.playerId] = { name: name, eligibleSlots: es };
          }
        }
      }

      var teamPlayersMap = {};
      for (var i = 0; i < draftDetail.picks.length; i++) {
        var pick = draftDetail.picks[i];
        if (!pick || pick.playerId == null || pick.teamId == null) continue;
        var info = playerInfoMap[pick.playerId];
        var playerName = info ? info.name : '';
        var eligibleSlots = info ? info.eligibleSlots : [];
        var lineupSlotId = pick.lineupSlotId != null ? pick.lineupSlotId : 16;
        if (!playerName) continue;
        if (!teamPlayersMap[pick.teamId]) teamPlayersMap[pick.teamId] = [];
        teamPlayersMap[pick.teamId].push({
          playerId: String(pick.playerId),
          playerName: playerName, lineupSlotId: lineupSlotId, eligibleSlots: eligibleSlots,
          price: pick.bidAmount != null ? pick.bidAmount : 0,
        });
      }

      var fbResult = [];
      for (var tid in teamInfoMap) {
        if (!teamInfoMap.hasOwnProperty(tid)) continue;
        var numTid = parseInt(tid, 10);
        fbResult.push({ teamId: isNaN(numTid) ? tid : numTid, teamName: teamInfoMap[tid], players: teamPlayersMap[tid] || [] });
      }
      var fbTotal = 0;
      for (var i = 0; i < fbResult.length; i++) fbTotal += fbResult[i].players.length;
      if (fbTotal > 0) {
        console.log(LOG, 'Built rosters from draftDetail.picks:', fbTotal, 'players across', fbResult.length, 'teams');
        return fbResult;
      }

      return null;
    }

    // ---- Helper: extract from global ESPN objects ----
    function extractFromGlobalObject(obj, globalKey) {
      try {
        var str = JSON.stringify(obj);
        if (!str || str.length < 10) return null;
        if (str.length > 10 * 1024 * 1024) {
          console.warn(LOG, 'Global', globalKey, 'too large:', str.length, 'bytes');
          return null;
        }
        var data = JSON.parse(str);
        return checkObjectForDraftData(data, 'global:' + globalKey, 0);
      } catch (e) {
        console.warn(LOG, 'Failed to extract from global', globalKey + ':', e.message || e);
        return null;
      }
    }

    // ---- Main extraction logic ----
    console.log(LOG, 'Page-context extractor running');

    var stateObj = null;

    // Strategy 1: Find React fiber on draft container
    var selectors = ['div.draft-content-wrapper', '#fitt-analytics', '#global-viewport', '#app'];
    for (var s = 0; s < selectors.length; s++) {
      var el = document.querySelector(selectors[s]);
      if (el) {
        stateObj = extractFromReactFiber(el);
        if (stateObj) break;
      }
    }

    // Strategy 2: Search common ESPN global state objects
    if (!stateObj) {
      var globalKeys = ['__espnfitt__', '__espn__', 'espn', '__NEXT_DATA__', '__INITIAL_STATE__'];
      for (var g = 0; g < globalKeys.length; g++) {
        try {
          var val = window[globalKeys[g]];
          if (val && typeof val === 'object') {
            console.log(LOG, 'Found page global:', globalKeys[g]);
            stateObj = extractFromGlobalObject(val, globalKeys[g]);
            if (stateObj) break;
          }
        } catch (e) { /* skip */ }
      }
    }

    // Strategy 3: Look for draft-related properties on window
    if (!stateObj) {
      try {
        var ownKeys = Object.getOwnPropertyNames(window);
        for (var k = 0; k < ownKeys.length; k++) {
          try {
            var key = ownKeys[k];
            if (key.toLowerCase().indexOf('draft') !== -1 || key.toLowerCase().indexOf('fantasy') !== -1) {
              var val = window[key];
              if (val && typeof val === 'object' && !Array.isArray(val)) {
                console.log(LOG, 'Found potentially relevant global:', key);
                stateObj = checkObjectForDraftData(val, 'global:' + key, 0);
                if (stateObj) break;
              }
            }
          } catch (e) { /* skip */ }
        }
      } catch (e) { /* skip */ }
    }

    // Build rosters and post result
    if (stateObj) {
      var teamRosters = buildTeamRosters(stateObj);
      console.log(LOG, 'Posting result:', teamRosters ? (teamRosters.length + ' teams') : 'null');
      window.postMessage({ source: 'wyndham-draft-sync-page-extract', teamRosters: teamRosters }, '*');
    } else {
      console.log(LOG, 'No draft state found via any strategy');
      window.postMessage({ source: 'wyndham-draft-sync-page-extract', teamRosters: null }, '*');
    }

  } catch (e) {
    console.error(LOG, 'Page-context extraction failed:', e.message || e);
    window.postMessage({ source: 'wyndham-draft-sync-page-extract', teamRosters: null, error: e.message || String(e) }, '*');
  }
}

// ---------------------------------------------------------------------------
// State handling and forwarding to background script
// ---------------------------------------------------------------------------

/** Last state fingerprint (for deduplication, excludes timeRemaining) */
let lastFingerprint = null;

/**
 * Compute a lightweight fingerprint of the state for deduplication.
 * Excludes timeRemaining since it changes every second and would defeat dedup.
 */
function computeFingerprint(state) {
  const picks = state.picks || [];
  const nom = state.currentNomination;
  const teams = state.teams || [];
  const teamBudgets = teams.map((t) => t.teamName + ':' + t.budget).join(',');
  return (
    picks.length +
    '|' +
    (nom
      ? nom.playerName +
        '|' +
        nom.currentBid +
        '|' +
        (nom.currentBidder || '') +
        '|' +
        (nom.nominatedBy || '')
      : 'none') +
    '|' +
    (state.myTeamId || '') +
    '|' +
    teamBudgets +
    '|' +
    (state.pickCount ?? '') +
    '|' +
    (state.totalPicks ?? '') +
    '|' +
    (state.draftId || '')
  );
}

/**
 * Build a state payload object from the current scraped state.
 */
function buildStatePayload(state) {
  return {
    picks: state.picks || [],
    currentNomination: state.currentNomination || null,
    myTeamId: state.myTeamId || null,
    teams: state.teams || [],
    pickCount: state.pickCount ?? null,
    totalPicks: state.totalPicks ?? null,
    draftId: state.draftId || null,
    source: state.source || 'unknown',
  };
}

/**
 * Send a full state snapshot to the background script with type FULL_STATE_SYNC.
 *
 * Called on initial connect or reconnect so the backend can reset its in-memory
 * draft state and rebuild it from scratch. Unlike STATE_UPDATE (which carries
 * incremental diffs), FULL_STATE_SYNC always includes the complete current pick
 * history and team budgets visible on the page.
 *
 * Injects a page-context script to extract complete roster data from ESPN's
 * React state, waits briefly for the async message to arrive, then sends
 * the FULL_STATE_SYNC with whatever roster data is cached.
 */
function sendFullStateSync() {
  const state = scrapeDom();
  if (!state) return;

  log('Preparing FULL_STATE_SYNC with', (state.picks || []).length, 'DOM picks');

  // Re-inject the page-context extractor to get fresh React state data.
  // The injected script runs synchronously in the page context and posts
  // results via window.postMessage. The content script's message handler
  // fires asynchronously, so we wait a short delay before sending.
  injectPageContextExtractor();

  setTimeout(() => {
    const payload = buildStatePayload(state);

    // Attach cached roster data from the page-context extraction
    if (cachedTeamRosters && cachedTeamRosters.length > 0) {
      const totalPlayers = cachedTeamRosters.reduce((sum, t) => sum + (t.players ? t.players.length : 0), 0);
      if (totalPlayers > 0) {
        payload.teamRosters = cachedTeamRosters;
        log('FULL_STATE_SYNC enriched with React state data:', totalPlayers, 'players across', cachedTeamRosters.length, 'teams');
      }
    } else {
      log('FULL_STATE_SYNC: no React state roster data available, sending DOM-only');
    }

    const message = {
      source: 'wyndham-draft-sync',
      type: 'FULL_STATE_SYNC',
      timestamp: Date.now(),
      payload: payload,
    };

    try {
      browser.runtime.sendMessage(message).catch((err) => {
        warn('Failed to send FULL_STATE_SYNC to background:', err.message || err);
      });
    } catch (e) {
      warn('runtime.sendMessage not available:', e.message || e);
    }

    lastFingerprint = computeFingerprint(state);
  }, 50);
}

/**
 * Process an extracted state update and forward to the background script.
 */
function handleStateUpdate(state) {
  // Deduplication: skip if the fingerprint (excluding timeRemaining) is unchanged
  const fingerprint = computeFingerprint(state);
  if (fingerprint === lastFingerprint) {
    return;
  }
  lastFingerprint = fingerprint;

  const message = {
    source: 'wyndham-draft-sync',
    type: 'STATE_UPDATE',
    timestamp: Date.now(),
    payload: buildStatePayload(state),
  };

  // Send to background script via browser.runtime.sendMessage
  try {
    browser.runtime.sendMessage(message).catch((err) => {
      // Background script might not be ready yet
      warn('Failed to send message to background:', err.message || err);
    });
  } catch (e) {
    warn('runtime.sendMessage not available:', e.message || e);
  }
}

// ---------------------------------------------------------------------------
// MutationObserver for draft container changes
// ---------------------------------------------------------------------------

let mutationObserver = null;
let debounceTimer = null;

/**
 * Request state extraction via DOM scraping.
 * Debounced to avoid excessive extractions during rapid DOM mutations.
 */
function requestStateExtraction() {
  if (debounceTimer) {
    clearTimeout(debounceTimer);
  }
  debounceTimer = setTimeout(() => {
    debounceTimer = null;
    const domState = scrapeDom();
    if (domState) {
      handleStateUpdate(domState);
    }
  }, MUTATION_DEBOUNCE_MS);
}

/**
 * Start observing a DOM element for mutations.
 */
function startObserving(target) {
  if (mutationObserver) {
    mutationObserver.disconnect();
  }

  mutationObserver = new MutationObserver((_mutations) => {
    requestStateExtraction();
  });

  mutationObserver.observe(target, {
    childList: true,
    subtree: true,
    characterData: true,
    // Only observe class and data-testid attribute changes to avoid noise
    attributes: true,
    attributeFilter: ['class', 'data-testid'],
  });

  log('MutationObserver attached to:', target.tagName, target.className || target.id || '');

  // Trigger an immediate extraction
  requestStateExtraction();
}

// ---------------------------------------------------------------------------
// Poll for draft container element, then start observing
// ---------------------------------------------------------------------------

/**
 * Find the draft container element.
 */
function findDraftContainer() {
  try {
    return document.querySelector(SELECTORS.draftContainer);
  } catch (e) {
    return null;
  }
}

function initObserver() {
  const startTime = Date.now();

  const pollId = setInterval(() => {
    const container = findDraftContainer();
    if (container) {
      clearInterval(pollId);
      log('Found draft container element');
      startObserving(container);
      return;
    }

    // Safety timeout: fall back to observing document.body
    if (Date.now() - startTime >= POLL_TIMEOUT_MS) {
      clearInterval(pollId);
      warn('Draft container not found after', POLL_TIMEOUT_MS, 'ms. Falling back to document.body');
      startObserving(document.body);
    }
  }, POLL_INTERVAL_MS);
}

// ---------------------------------------------------------------------------
// Periodic polling fallback
// Posts state updates at regular intervals even without DOM mutations,
// in case MutationObserver misses changes (e.g., virtual DOM updates that
// don't trigger subtree mutations).
// ---------------------------------------------------------------------------

function startPeriodicPolling() {
  setInterval(() => {
    requestStateExtraction();
  }, FALLBACK_POLL_INTERVAL_MS);
}

// ---------------------------------------------------------------------------
// Periodic keyframe: automatically sends a FULL_STATE_SYNC every
// KEYFRAME_INTERVAL_MS so the backend always has a recent known-good
// snapshot. This prevents state drift from accumulating between reconnects.
// ---------------------------------------------------------------------------

let keyframeIntervalId = null;

function startPeriodicKeyframe() {
  if (keyframeIntervalId) clearInterval(keyframeIntervalId);
  keyframeIntervalId = setInterval(() => {
    const state = scrapeDom();
    if (!state) return; // DOM not ready yet
    const fingerprint = computeFingerprint(state);
    if (fingerprint !== lastFingerprint) {
      sendFullStateSync();
    }
  }, KEYFRAME_INTERVAL_MS);
}

// ---------------------------------------------------------------------------
// Message listener: respond to requests from the background script
// ---------------------------------------------------------------------------

/**
 * Listen for messages from the background script.
 * Handles REQUEST_FULL_STATE_SYNC: sent by the background after a WebSocket
 * reconnect to trigger an immediate FULL_STATE_SYNC from the content script.
 */
browser.runtime.onMessage.addListener((message) => {
  if (!message || message.source !== 'wyndham-draft-sync-bg') {
    return;
  }
  if (message.type === 'REQUEST_FULL_STATE_SYNC') {
    log('Received REQUEST_FULL_STATE_SYNC from background');
    sendFullStateSync();
  }
});

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

function init() {
  log('Initializing ESPN draft page scraper');

  // Set up listener for page-context script results (must be first, before
  // any injection, so the listener is ready when results arrive)
  setupPageContextListener();

  // Inject the page-context extractor to pre-populate cachedTeamRosters.
  // This runs in the page's JS context where React fiber keys are visible.
  injectPageContextExtractor();

  // Poll for draft container and start observing
  initObserver();

  // Start periodic polling fallback
  startPeriodicPolling();

  // Start periodic keyframe: sends a FULL_STATE_SYNC every 10 seconds
  // so the backend always has a recent known-good snapshot
  startPeriodicKeyframe();

  log('Content script initialized');
}

// Run initialization
init();
