// ESPN Draft Page Content Script
// Injects a page-context script for React state extraction, falls back to DOM scraping.
// Communicates with the background script via browser.runtime.sendMessage.

'use strict';

// ---------------------------------------------------------------------------
// Configurable ESPN selectors and constants
// All ESPN-specific selectors should be updated after inspecting the live draft page.
// ---------------------------------------------------------------------------

// VERIFY: inspect live ESPN draft page — these are best-guess selectors
const SELECTORS = {
  // Draft container candidates (tried in order)
  draftContainers: [
    '[class*="Draft"]',           // VERIFY: inspect live ESPN draft page
    '#draft-app',                 // VERIFY: inspect live ESPN draft page
    '.draft-board',               // VERIFY: inspect live ESPN draft page
    '[class*="draft"]',           // VERIFY: inspect live ESPN draft page
    '.draftBoard',                // VERIFY: inspect live ESPN draft page
    '#espn-draft',                // VERIFY: inspect live ESPN draft page
  ],

  // DOM scraping selectors (used when React extraction fails)
  pickRows:          '.pick-row, [class*="PickRow"], [class*="pickRow"], tr[class*="pick"]',  // VERIFY: inspect live ESPN draft page
  pickTeamName:      '.pick-team, [class*="teamName"], [class*="TeamName"], td:nth-child(1)', // VERIFY: inspect live ESPN draft page
  pickPlayerName:    '.pick-player, [class*="playerName"], [class*="PlayerName"], td:nth-child(2)', // VERIFY: inspect live ESPN draft page
  pickPrice:         '.pick-price, [class*="price"], [class*="Price"], td:nth-child(3)',      // VERIFY: inspect live ESPN draft page
  pickPosition:      '.pick-position, [class*="position"], [class*="Position"], td:nth-child(4)', // VERIFY: inspect live ESPN draft page

  // Nomination area
  nominationContainer: '[class*="Nomination"], [class*="nomination"], .auction-block, [class*="AuctionBlock"]', // VERIFY: inspect live ESPN draft page
  nominationPlayer:    '[class*="nomPlayer"], [class*="NomPlayer"], .nom-player-name',        // VERIFY: inspect live ESPN draft page
  nominationPosition:  '[class*="nomPosition"], [class*="NomPosition"], .nom-position',       // VERIFY: inspect live ESPN draft page
  nominatedBy:         '[class*="nominatedBy"], [class*="NominatedBy"], .nom-team',            // VERIFY: inspect live ESPN draft page
  currentBid:          '[class*="currentBid"], [class*="CurrentBid"], .bid-amount',            // VERIFY: inspect live ESPN draft page
  currentBidder:       '[class*="bidder"], [class*="Bidder"], .bid-team',                      // VERIFY: inspect live ESPN draft page
  timeRemaining:       '[class*="timer"], [class*="Timer"], [class*="countdown"], .timer',     // VERIFY: inspect live ESPN draft page

  // My team identification
  myTeamIndicator:     '[class*="myTeam"], [class*="MyTeam"], .my-team, [class*="owner"]',     // VERIFY: inspect live ESPN draft page
};

// React fiber property prefixes to search for
const REACT_FIBER_PREFIXES = ['__reactFiber$', '__reactInternalInstance$']; // VERIFY: inspect live ESPN draft page

// Component state keys that indicate draft state
const DRAFT_STATE_KEYS = ['draftState', 'draftPicks', 'picks', 'auction', 'draft']; // VERIFY: inspect live ESPN draft page

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
// Step 1: Inject page-context script for React state access
// Content scripts run in an isolated world and cannot access page JS globals.
// We inject a <script> element that runs in the page context.
// ---------------------------------------------------------------------------

function injectPageScript() {
  const script = document.createElement('script');
  script.textContent = `(${pageContextScript.toString()})();`;
  (document.head || document.documentElement).appendChild(script);
  script.remove(); // Clean up DOM; the code is already executing
  log('Injected page-context script for React state extraction');
}

/**
 * This function runs in the PAGE context (not the content script context).
 * It has access to the actual DOM element properties including React fiber internals.
 * It posts extracted state back to the content script via window.postMessage.
 */
function pageContextScript() {
  const LOG_PREFIX = '[WyndhamDraftSync:PageCtx]';

  // VERIFY: inspect live ESPN draft page — these are best-guess selectors
  const DRAFT_ROOT_SELECTORS = [
    '[class*="Draft"]',
    '#draft-app',
    '.draft-board',
    '[class*="draft"]',
    '.draftBoard',
    '#espn-draft',
  ];

  // VERIFY: inspect live ESPN draft page
  const REACT_FIBER_PREFIXES = ['__reactFiber$', '__reactInternalInstance$'];
  const DRAFT_STATE_KEYS = ['draftState', 'draftPicks', 'picks', 'auction', 'draft'];

  /**
   * Find the React fiber key on a DOM element.
   */
  function findFiberKey(element) {
    if (!element) return null;
    const keys = Object.keys(element);
    for (const prefix of REACT_FIBER_PREFIXES) {
      const key = keys.find(k => k.startsWith(prefix));
      if (key) return key;
    }
    return null;
  }

  /**
   * Walk up the React fiber tree looking for a component whose state or props
   * contain draft-related data.
   */
  function findDraftState(fiber, maxDepth = 50) {
    let current = fiber;
    let depth = 0;

    while (current && depth < maxDepth) {
      // Check memoizedState (hooks or class state)
      const state = current.memoizedState;
      if (state && typeof state === 'object') {
        for (const key of DRAFT_STATE_KEYS) {
          if (state[key]) {
            return { source: 'memoizedState', data: state };
          }
        }
      }

      // Check memoizedProps
      const props = current.memoizedProps;
      if (props && typeof props === 'object') {
        for (const key of DRAFT_STATE_KEYS) {
          if (props[key]) {
            return { source: 'memoizedProps', data: props };
          }
        }
      }

      // Check pendingProps
      const pendingProps = current.pendingProps;
      if (pendingProps && typeof pendingProps === 'object') {
        for (const key of DRAFT_STATE_KEYS) {
          if (pendingProps[key]) {
            return { source: 'pendingProps', data: pendingProps };
          }
        }
      }

      // Walk up the fiber tree (try return first, which is the parent fiber)
      current = current.return;
      depth++;
    }

    return null;
  }

  /**
   * Normalize extracted React state into the protocol format.
   * The shape depends on what ESPN's React components actually store.
   */
  function normalizeReactState(rawState) {
    // VERIFY: inspect live ESPN draft page — the actual state shape will vary
    const result = {
      picks: [],
      currentNomination: null,
      myTeamId: null,
    };

    try {
      // Try common state shapes
      const data = rawState.data || rawState;

      // Extract picks
      const picks = data.draftPicks || data.picks || data.draftState?.picks || [];
      if (Array.isArray(picks)) {
        result.picks = picks.map((p, idx) => ({
          pickNumber: p.pickNumber || p.pick_number || p.id || idx + 1,
          teamId: String(p.teamId || p.team_id || p.ownerId || ''),
          teamName: p.teamName || p.team_name || p.ownerName || '',
          playerId: String(p.playerId || p.player_id || p.id || ''),
          playerName: p.playerName || p.player_name || p.fullName || p.name || '',
          position: p.position || p.defaultPosition || p.pos || '',
          price: Number(p.price || p.salary || p.cost || 0),
        })).filter(p => p.playerName); // Filter out empty/invalid picks
      }

      // Extract current nomination
      const nom = data.currentNomination || data.nomination || data.auction?.currentNomination
        || data.draftState?.currentNomination;
      if (nom && (nom.playerName || nom.player_name || nom.fullName || nom.name)) {
        result.currentNomination = {
          playerId: String(nom.playerId || nom.player_id || nom.id || ''),
          playerName: nom.playerName || nom.player_name || nom.fullName || nom.name || '',
          position: nom.position || nom.defaultPosition || nom.pos || '',
          nominatedBy: nom.nominatedBy || nom.nominated_by || nom.nominator || '',
          currentBid: Number(nom.currentBid || nom.current_bid || nom.bid || 0),
          currentBidder: nom.currentBidder || nom.current_bidder || nom.highBidder || null,
          timeRemaining: nom.timeRemaining != null ? Number(nom.timeRemaining) :
                         nom.time_remaining != null ? Number(nom.time_remaining) :
                         nom.timer != null ? Number(nom.timer) : null,
        };
      }

      // Extract my team ID
      result.myTeamId = data.myTeamId || data.my_team_id || data.currentTeamId
        || data.ownerTeamId || data.draftState?.myTeamId || null;
      if (result.myTeamId !== null) {
        result.myTeamId = String(result.myTeamId);
      }
    } catch (e) {
      console.warn(LOG_PREFIX, 'Error normalizing React state:', e);
    }

    return result;
  }

  /**
   * Attempt to extract draft state from React internals.
   * Returns null if extraction fails.
   */
  function extractReactState() {
    // Find a draft root element
    for (const selector of DRAFT_ROOT_SELECTORS) {
      try {
        const elements = document.querySelectorAll(selector);
        for (const el of elements) {
          const fiberKey = findFiberKey(el);
          if (!fiberKey) continue;

          const fiber = el[fiberKey];
          if (!fiber) continue;

          const draftState = findDraftState(fiber);
          if (draftState) {
            const normalized = normalizeReactState(draftState);
            return normalized;
          }
        }
      } catch (e) {
        // Selector might be invalid or element access might fail
        continue;
      }
    }

    return null;
  }

  /**
   * Post the extracted state back to the content script.
   */
  function postState() {
    const reactState = extractReactState();

    if (reactState) {
      window.postMessage({
        source: 'wyndham-draft-sync',
        type: 'REACT_STATE',
        payload: reactState,
        extractionSource: 'react_state',
      }, '*');
    } else {
      // Signal that React extraction failed so content script can use DOM fallback
      window.postMessage({
        source: 'wyndham-draft-sync',
        type: 'REACT_EXTRACTION_FAILED',
      }, '*');
    }
  }

  // Listen for extraction requests from the content script
  window.addEventListener('message', (event) => {
    if (event.source !== window) return;
    if (!event.data || event.data.source !== 'wyndham-draft-sync-request') return;

    if (event.data.type === 'EXTRACT_STATE') {
      postState();
    }
  });

  console.log(LOG_PREFIX, 'Page-context script ready');
}

// ---------------------------------------------------------------------------
// Step 2: Listen for state updates from page context
// ---------------------------------------------------------------------------

/** Last state received from either React extraction or DOM scraping */
let lastState = null;

/** Whether React extraction is working (if false, use DOM fallback) */
let reactExtractionAvailable = true;

/** Track consecutive React failures to switch to DOM-only mode */
let reactFailureCount = 0;
const MAX_REACT_FAILURES = 5;

window.addEventListener('message', (event) => {
  if (event.source !== window) return;
  if (!event.data || event.data.source !== 'wyndham-draft-sync') return;

  if (event.data.type === 'REACT_STATE') {
    reactFailureCount = 0;
    reactExtractionAvailable = true;
    const state = event.data.payload;
    state.source = 'react_state';
    handleStateUpdate(state);
  } else if (event.data.type === 'REACT_EXTRACTION_FAILED') {
    reactFailureCount++;
    if (reactFailureCount >= MAX_REACT_FAILURES) {
      reactExtractionAvailable = false;
    }
    // Fall back to DOM scraping
    const domState = scrapeDom();
    if (domState) {
      handleStateUpdate(domState);
    }
  }
});

// ---------------------------------------------------------------------------
// Step 3: MutationObserver for draft container changes
// ---------------------------------------------------------------------------

let mutationObserver = null;
let debounceTimer = null;

/**
 * Request state extraction (triggers page-context script or DOM fallback).
 * Debounced to avoid excessive extractions during rapid DOM mutations.
 */
function requestStateExtraction() {
  if (debounceTimer) {
    clearTimeout(debounceTimer);
  }
  debounceTimer = setTimeout(() => {
    debounceTimer = null;
    if (reactExtractionAvailable) {
      // Ask the page-context script to extract React state
      window.postMessage({
        source: 'wyndham-draft-sync-request',
        type: 'EXTRACT_STATE',
      }, '*');
    } else {
      // React not available, use DOM scraping directly
      const domState = scrapeDom();
      if (domState) {
        handleStateUpdate(domState);
      }
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
    attributes: true,
    attributeFilter: ['class', 'style', 'data-*'],
  });

  log('MutationObserver attached to:', target.tagName, target.className || target.id || '');

  // Trigger an immediate extraction
  requestStateExtraction();
}

// ---------------------------------------------------------------------------
// Step 4: Poll for draft container element, then start observing
// ---------------------------------------------------------------------------

/**
 * Find the draft container element using configured selectors.
 */
function findDraftContainer() {
  for (const selector of SELECTORS.draftContainers) {
    try {
      const el = document.querySelector(selector);
      if (el) return el;
    } catch (e) {
      // Invalid selector, skip
      continue;
    }
  }
  return null;
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
// DOM Scraping Fallback
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
 * Parse a price string like "$42" or "42" into a number.
 */
function parsePrice(priceStr) {
  if (!priceStr) return 0;
  const cleaned = priceStr.replace(/[^0-9]/g, '');
  return parseInt(cleaned, 10) || 0;
}

/**
 * Parse a time string like "0:15" or "15" into seconds.
 */
function parseTime(timeStr) {
  if (!timeStr) return null;
  const cleaned = timeStr.trim();
  // Handle "M:SS" format
  const colonMatch = cleaned.match(/(\d+):(\d+)/);
  if (colonMatch) {
    return parseInt(colonMatch[1], 10) * 60 + parseInt(colonMatch[2], 10);
  }
  // Handle plain seconds
  const num = parseInt(cleaned.replace(/[^0-9]/g, ''), 10);
  return isNaN(num) ? null : num;
}

/**
 * Scrape draft state from the DOM as a fallback when React extraction fails.
 */
function scrapeDom() {
  const state = {
    picks: [],
    currentNomination: null,
    myTeamId: null,
    source: 'dom_scrape',
  };

  try {
    // Scrape completed picks
    const pickRows = document.querySelectorAll(SELECTORS.pickRows);
    pickRows.forEach((row, idx) => {
      const teamName = extractText(row, SELECTORS.pickTeamName);
      const playerName = extractText(row, SELECTORS.pickPlayerName);
      const priceStr = extractText(row, SELECTORS.pickPrice);
      const position = extractText(row, SELECTORS.pickPosition);

      if (playerName) {
        state.picks.push({
          pickNumber: idx + 1,
          teamId: '',       // DOM scraping may not have team ID
          teamName: teamName,
          playerId: '',     // DOM scraping may not have player ID
          playerName: playerName,
          position: position,
          price: parsePrice(priceStr),
        });
      }
    });

    // Scrape current nomination
    const nomContainer = document.querySelector(SELECTORS.nominationContainer);
    if (nomContainer) {
      const playerName = extractText(nomContainer, SELECTORS.nominationPlayer);
      if (playerName) {
        const timeStr = extractText(nomContainer, SELECTORS.timeRemaining);
        state.currentNomination = {
          playerId: '',
          playerName: playerName,
          position: extractText(nomContainer, SELECTORS.nominationPosition),
          nominatedBy: extractText(nomContainer, SELECTORS.nominatedBy),
          currentBid: parsePrice(extractText(nomContainer, SELECTORS.currentBid)),
          currentBidder: extractText(nomContainer, SELECTORS.currentBidder) || null,
          timeRemaining: parseTime(timeStr),
        };
      }
    }

    // Try to find my team indicator
    const myTeamEl = document.querySelector(SELECTORS.myTeamIndicator);
    if (myTeamEl) {
      // The team ID might be in a data attribute or we just use the text
      state.myTeamId = myTeamEl.dataset.teamId || myTeamEl.dataset.id || null;
    }
  } catch (e) {
    error('DOM scraping error:', e);
  }

  return state;
}

// ---------------------------------------------------------------------------
// State handling and forwarding to background script
// ---------------------------------------------------------------------------

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
  log('Initializing ESPN draft page scraper');

  // Step 1: Inject page-context script
  injectPageScript();

  // Step 4: Poll for draft container and start observing
  initObserver();

  // Start periodic polling fallback
  startPeriodicPolling();

  log('Content script initialized');
}

// Run initialization
init();
