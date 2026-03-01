// ESPN Draft Page Content Script
// Scrapes draft state from ESPN's DOM and relays it to the background script.
// Communicates with the background script via browser.runtime.sendMessage.

'use strict';

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

  // Bid history within nomination area
  bidHistoryItems: 'ul.bid-history__list > li.bid',

  // Pick history / draft log (right column)
  pickLogEntries: 'li.pick-message__container',
  pickLogPlayerName: 'span.playerinfo__playername',
  pickLogPlayerPos: 'span.playerinfo__playerpos',
  pickLogInfo: 'div.pick-info',

  // My team identification via pick train
  myTeamContent: 'div.content.auction-pick-component--own',

  // Draft board grid (for scraping ESPN-assigned roster slots)
  draftBoardGrid: 'div.draftBoardGrid',
  draftBoardHeaders: 'div.draft-board-grid-header-cell > span',
  draftBoardCompletedPick: 'div.draft-board-grid-pick-cell.completedPick',
  draftBoardRosterSlot: 'div.rosterSlot',
  draftBoardPlayerFirstName: 'span.playerFirstName',
  draftBoardPlayerLastName: 'span.playerLastName',
};

// Timing constants
const MUTATION_DEBOUNCE_MS = 250;
const POLL_INTERVAL_MS = 500;
const POLL_TIMEOUT_MS = 10000;
const FALLBACK_POLL_INTERVAL_MS = 1000;

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
 * Scrape ESPN-assigned roster slot positions from the draft board grid.
 *
 * The draft board is a CSS grid where each column is a team and each row
 * is a roster slot. Completed pick cells contain the roster slot label
 * (e.g. "UTIL", "BE", "SS"), the player's first/last name, and the price.
 *
 * Returns a Map keyed by "playerLastName|column" -> rosterSlot string.
 * We use last name + column index because:
 *   - Last names are displayed prominently and reliably scraped
 *   - Column index maps to a team (same order as draft board headers)
 *   - This combination is unique enough to match against pick log entries
 *
 * Also returns the team names in column order for mapping.
 */
function scrapeDraftBoardSlots() {
  const slotMap = new Map();
  const teamNames = [];

  try {
    // Extract team names from the draft board header cells
    const headers = document.querySelectorAll(SELECTORS.draftBoardHeaders);
    headers.forEach((h) => {
      teamNames.push(h.textContent.trim());
    });

    if (teamNames.length === 0) {
      return { slotMap, teamNames };
    }

    // Extract roster slot assignments from completed pick cells
    const completedCells = document.querySelectorAll(SELECTORS.draftBoardCompletedPick);
    completedCells.forEach((cell) => {
      const rosterSlotEl = cell.querySelector(SELECTORS.draftBoardRosterSlot);
      const lastNameEl = cell.querySelector(SELECTORS.draftBoardPlayerLastName);
      if (!rosterSlotEl || !lastNameEl) return;

      const rosterSlot = rosterSlotEl.textContent.trim();
      const lastName = lastNameEl.textContent.trim();
      if (!rosterSlot || !lastName) return;

      // Extract the column from the grid-area CSS style.
      // Format: "grid-area: row / col;" e.g. "grid-area: 5 / 1;"
      const style = cell.getAttribute('style') || '';
      const gridMatch = style.match(/grid-area:\s*\d+\s*\/\s*(\d+)/);
      if (!gridMatch) return;

      const colIdx = parseInt(gridMatch[1], 10) - 1; // 1-indexed to 0-indexed
      if (colIdx < 0 || colIdx >= teamNames.length) return;

      const teamName = teamNames[colIdx];
      // Key: "lastName|teamName" for matching against pick log entries
      const key = lastName.toLowerCase() + '|' + teamName.toLowerCase();
      slotMap.set(key, rosterSlot);
    });
  } catch (e) {
    error('Error scraping draft board slots:', e);
  }

  return { slotMap, teamNames };
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

    // Scrape the draft board grid for ESPN-assigned roster slot positions.
    // This map lets us enrich each pick with the slot ESPN placed the player in.
    const { slotMap } = scrapeDraftBoardSlots();

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

        // Look up ESPN-assigned roster slot from the draft board grid.
        // The pick log shows "First Last" as the player name. Extract the
        // last name (last whitespace-delimited token) for the lookup key.
        let rosterSlot = null;
        if (slotMap.size > 0 && teamName) {
          const nameParts = playerName.trim().split(/\s+/);
          const lastName = nameParts[nameParts.length - 1];
          const key = lastName.toLowerCase() + '|' + teamName.toLowerCase();
          rosterSlot = slotMap.get(key) || null;
        }

        picks.push({
          pickNumber: completedPicks - entriesArray.length + idx + 1,
          teamId: teamName,
          teamName: teamName,
          playerId: '',
          playerName: playerName,
          position: position,
          price: price,
          eligibleSlots: [],
          rosterSlot: rosterSlot,
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
 */
function scrapeCurrentNomination() {
  try {
    const pickArea = document.querySelector(SELECTORS.nominationContainer);
    if (!pickArea) return null;

    const playerSelected = pickArea.querySelector(SELECTORS.playerSelected);
    if (!playerSelected) return null;

    const playerName = extractText(playerSelected, SELECTORS.nominationPlayerName);
    if (!playerName) return null;

    let position = extractText(playerSelected, SELECTORS.nominationPlayerPos);
    const offerStr = extractText(playerSelected, SELECTORS.nominationCurrentOffer);
    const currentBid = parseCurrentOffer(offerStr);
    const timeRemaining = parseClockDigits();

    // Handle compound position strings like "SP, RP" — take only the first
    if (position.includes(',')) {
      position = position.split(',')[0].trim();
    }

    // Query bid history scoped to the pickArea, not the entire document
    const bidItems = pickArea.querySelectorAll(SELECTORS.bidHistoryItems);
    const currentBidder = extractCurrentBidder(bidItems);
    const nominatedBy = extractNominatedBy(bidItems);

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
    (nom ? nom.playerName + '|' + nom.currentBid + '|' + (nom.currentBidder || '') : 'none') +
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
    payload: {
      picks: state.picks || [],
      currentNomination: state.currentNomination || null,
      myTeamId: state.myTeamId || null,
      teams: state.teams || [],
      pickCount: state.pickCount ?? null,
      totalPicks: state.totalPicks ?? null,
      draftId: state.draftId || null,
      source: state.source || 'unknown',
    },
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
// Initialization
// ---------------------------------------------------------------------------

function init() {
  log('Initializing ESPN draft page scraper (DOM-only mode)');

  // Poll for draft container and start observing
  initObserver();

  // Start periodic polling fallback
  startPeriodicPolling();

  log('Content script initialized');
}

// Run initialization
init();
