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
  teamBudgetOwnModifier: 'auction-pick-component--own',

  // Current nomination / bidding area
  nominationContainer: 'div.pickArea',
  playerSelected: 'div[data-testid="player-selected"]',
  nominationPlayerName: 'span.playerinfo__playername',
  nominationPlayerTeam: 'span.playerinfo__playerteam',
  nominationPlayerPos: 'span.playerinfo__playerpos',
  nominationPreDraftVal: 'span.player-default-bid',
  nominationCurrentOffer: 'div.current-amount',
  nominationBidButton: 'button.bid-player__button',

  // Bid history within nomination area
  bidHistoryContainer: 'div.bid-history__container',
  bidHistoryItems: 'ul.bid-history__list > li.bid',
  bidHistoryOwnBid: 'li.bid.own-bid',

  // Pick history / draft log (right column)
  pickLogEntries: 'li.pick-message__container',
  pickLogPlayerName: 'span.playerinfo__playername',
  pickLogPlayerTeam: 'span.playerinfo__playerteam',
  pickLogPlayerPos: 'span.playerinfo__playerpos',
  pickLogInfo: 'div.pick-info',

  // My team identification via pick train
  myTeamContent: 'div.content.auction-pick-component--own',

  // Roster table (left sidebar)
  rosterModule: 'div.roster-module',
  rosterRows: 'div.roster-module tr.Table__TR--sm',
  rosterDropdown: 'div.roster__dropdown select.dropdown__select',
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
 * Extract the current bidder from the bid history list.
 * The most recent bid entry that is not the "own-bid" is the current high bidder.
 * The first entry in the list is the most recent bid.
 */
function extractCurrentBidder() {
  try {
    const items = document.querySelectorAll(SELECTORS.bidHistoryItems);
    if (items.length > 0) {
      // First item is most recent bid: e.g. "$2 Jamaica Jiggle Party"
      const text = items[0].textContent.trim();
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
 * Extract the nominating team from the bid history.
 * In ESPN's auction, the first bid is typically from the nominating team.
 * The last entry in the bid history list is the original nomination.
 */
function extractNominatedBy() {
  try {
    const items = document.querySelectorAll(SELECTORS.bidHistoryItems);
    if (items.length > 0) {
      // Last item is the original nomination
      const lastItem = items[items.length - 1];
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
 * Returns an array of { teamName, budget } objects.
 */
function scrapeTeamBudgets() {
  const teams = [];
  try {
    const items = document.querySelectorAll(SELECTORS.teamBudgetItems);
    items.forEach((item) => {
      const name = extractText(item, SELECTORS.teamBudgetName);
      const cashStr = extractText(item, SELECTORS.teamBudgetCash);
      if (name) {
        // Strip the leading number + dot from team name: "1. London Ligers" -> "London Ligers"
        const cleanName = name.replace(/^\d+\.\s*/, '');
        teams.push({
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
 */
function scrapePickLog() {
  const picks = [];
  try {
    const entries = document.querySelectorAll(SELECTORS.pickLogEntries);
    // Entries are most-recent-first; we want chronological order
    const entriesArray = Array.from(entries).reverse();

    entriesArray.forEach((entry, idx) => {
      const playerName = extractText(entry, SELECTORS.pickLogPlayerName);
      const playerTeam = extractText(entry, SELECTORS.pickLogPlayerTeam);
      const position = extractText(entry, SELECTORS.pickLogPlayerPos);
      const pickInfoStr = extractText(entry, SELECTORS.pickLogInfo);

      if (playerName) {
        const { price, teamName } = parsePickInfo(pickInfoStr);
        picks.push({
          pickNumber: idx + 1,
          teamId: '',
          teamName: teamName,
          playerId: '',
          playerName: playerName,
          position: position,
          price: price,
          eligibleSlots: [],
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

    const position = extractText(playerSelected, SELECTORS.nominationPlayerPos);
    const offerStr = extractText(playerSelected, SELECTORS.nominationCurrentOffer);
    const currentBid = parseCurrentOffer(offerStr);
    const timeRemaining = parseClockDigits();
    const currentBidder = extractCurrentBidder();
    const nominatedBy = extractNominatedBy();

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
    source: 'dom_scrape',
  };

  try {
    // Scrape completed picks from the draft log
    state.picks = scrapePickLog();

    // Scrape current nomination
    state.currentNomination = scrapeCurrentNomination();

    // Scrape team budgets from pick train
    state.teams = scrapeTeamBudgets();

    // Identify my team
    const myTeamName = identifyMyTeam();
    if (myTeamName) {
      // Use team name as ID since ESPN DOM doesn't expose numeric team IDs
      state.myTeamId = myTeamName;
    }
  } catch (e) {
    error('DOM scraping error:', e);
  }

  return state;
}

// ---------------------------------------------------------------------------
// State handling and forwarding to background script
// ---------------------------------------------------------------------------

/** Last state sent (for deduplication) */
let lastState = null;

/**
 * Process an extracted state update and forward to the background script.
 */
function handleStateUpdate(state) {
  // Simple deduplication: skip if the state is identical to the last one sent
  const stateJson = JSON.stringify(state);
  if (stateJson === lastState) {
    return;
  }
  lastState = stateJson;

  const message = {
    source: 'wyndham-draft-sync',
    type: 'STATE_UPDATE',
    timestamp: Date.now(),
    payload: {
      picks: state.picks || [],
      currentNomination: state.currentNomination || null,
      myTeamId: state.myTeamId || null,
      teams: state.teams || [],
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
    // Observe all attribute changes to catch class/style/data-* updates
    attributes: true,
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
