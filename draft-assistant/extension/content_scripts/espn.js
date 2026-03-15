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

  // Draft board grid (always fully rendered, never virtualized)
  draftBoardGrid: 'div.draftBoardGrid',
  draftBoardHeader: 'div.draft-board-grid-header',
  draftBoardHeaderCell: 'div.draft-board-grid-header-cell',
  draftBoardCell: 'div.draft-board-grid-pick-cell',

  // Pick history tables (all rounds fully rendered)
  pickHistoryTables: 'div.pick-history-tables',
  pickHistoryTable: 'div.pick-history-table',
  pickHistoryCaption: 'div.caption',

  // Roster module (user's team only)
  rosterModule: 'div.roster-module',

  // ESPN team ID dropdown (maps team names to ESPN numeric IDs)
  teamIdDropdown: 'div.roster__dropdown select.dropdown__select',
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
 * Falls back to the pick history `my-pick` CSS class if the pick train
 * method fails (ESPN may change CSS classes across updates).
 * Returns the team name, or null if not found.
 */
function identifyMyTeam() {
  if (cachedMyTeamName) return cachedMyTeamName;

  // Primary: CSS class on pick train
  try {
    const ownContent = document.querySelector(SELECTORS.myTeamContent);
    if (ownContent) {
      const nameEl = ownContent.querySelector(SELECTORS.teamBudgetName);
      if (nameEl) {
        const name = nameEl.textContent.trim();
        cachedMyTeamName = name.replace(/^\d+\.\s*/, '');
        return cachedMyTeamName;
      }
    }
  } catch (e) {
    // Could not identify own team from pick train
  }

  // Fallback: find any pick with the my-pick CSS class in the pick history tables
  try {
    const myPickEl = document.querySelector('div.pick-history-tables .player-column.my-pick');
    if (myPickEl) {
      // Navigate up to the row, find the team name cell
      const row = myPickEl.closest('[aria-rowindex]');
      if (row) {
        const cells = Array.from(row.querySelectorAll('[role="gridcell"]'));
        cells.sort((a, b) => {
          const leftA = parseFloat(a.style.left);
          const leftB = parseFloat(b.style.left);
          return (isNaN(leftA) ? Infinity : leftA) - (isNaN(leftB) ? Infinity : leftB);
        });
        // Cell 3 (index 2) is the team name cell
        if (cells.length >= 3) {
          const boldTeam = cells[2].querySelector('span.fw-bold');
          const teamName = boldTeam ? boldTeam.textContent.trim() : cells[2].textContent.trim();
          if (teamName) {
            log('Identified my team from pick history my-pick class:', teamName);
            cachedMyTeamName = teamName;
            return cachedMyTeamName;
          }
        }
      }
    }
  } catch (e) {
    // Fallback identification failed
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
 * Scrape the draft board grid for complete team/roster state.
 *
 * The draft board grid (`div.draftBoardGrid`) contains ALL 10 teams × 26
 * roster slots and is always fully rendered (never virtualized). This makes
 * it the most reliable source for team rosters, especially when resuming
 * a draft mid-way where the pick log is virtualized and incomplete.
 *
 * Returns: { teams: [...], onTheClockTeam: string|null }
 */
function scrapeDraftBoard() {
  const result = { teams: [], onTheClockTeam: null };
  try {
    // 1. Parse header cells for team names, myTeam, onTheClock
    const headerCells = document.querySelectorAll(SELECTORS.draftBoardHeaderCell);
    if (headerCells.length === 0) return result;

    const teamsByColumn = {};
    headerCells.forEach((cell) => {
      // Extract column from grid-area style (e.g. "1 / 3" -> column 3)
      const gridArea = cell.style.gridArea || '';
      const colMatch = gridArea.match(/\d+\s*\/\s*(\d+)/);
      const column = colMatch ? parseInt(colMatch[1], 10) : 0;
      if (column === 0) return;

      const nameSpan = cell.querySelector('span');
      const teamName = nameSpan ? nameSpan.textContent.trim() : '';
      if (!teamName) return;

      const isMyTeam = cell.classList.contains('myTeam');
      const isOnTheClock = cell.classList.contains('onTheClock');

      if (isOnTheClock) {
        result.onTheClockTeam = teamName;
      }

      teamsByColumn[column] = {
        teamName: teamName,
        column: column,
        isMyTeam: isMyTeam,
        isOnTheClock: isOnTheClock,
        slots: [],
      };
    });

    // 2. Parse all pick cells
    const pickCells = document.querySelectorAll(SELECTORS.draftBoardCell);
    pickCells.forEach((cell) => {
      // Extract row/col from grid-area (e.g. "3 / 2" -> row 3, col 2)
      const gridArea = cell.style.gridArea || '';
      const areaMatch = gridArea.match(/(\d+)\s*\/\s*(\d+)/);
      if (!areaMatch) return;

      const row = parseInt(areaMatch[1], 10);
      const col = parseInt(areaMatch[2], 10);
      const team = teamsByColumn[col];
      if (!team) return;

      const isCompleted = cell.classList.contains('completedPick');
      const rosterSlotEl = cell.querySelector('.rosterSlot');
      const rosterSlot = rosterSlotEl ? rosterSlotEl.textContent.trim() : '';

      const slot = {
        row: row,
        rosterSlot: rosterSlot,
        filled: isCompleted,
      };

      if (isCompleted) {
        const firstNameEl = cell.querySelector('.playerFirstName');
        const lastNameEl = cell.querySelector('.playerLastName');
        const proTeamEl = cell.querySelector('.playerProTeam');
        const positionPillEl = cell.querySelector('.pickCellBottom .positionPill');
        const priceEl = cell.querySelector('.winningPrice');

        slot.firstName = firstNameEl ? firstNameEl.textContent.trim() : '';
        slot.lastName = lastNameEl ? lastNameEl.textContent.trim() : '';
        slot.proTeam = proTeamEl ? proTeamEl.textContent.trim() : '';
        slot.naturalPosition = positionPillEl ? positionPillEl.textContent.trim() : '';
        slot.price = priceEl ? parsePrice(priceEl.textContent) : 0;
      }

      team.slots.push(slot);
    });

    // Convert to array
    result.teams = Object.values(teamsByColumn);
  } catch (e) {
    error('Error scraping draft board:', e);
  }
  return result;
}

/**
 * Scrape the pick history tables for chronological pick order.
 *
 * The pick history section (`div.pick-history-tables`) contains 19 round
 * tables with 10 picks each, ALL fully rendered (never virtualized). This
 * gives us the complete chronological draft order with player IDs, eligible
 * positions, and team assignments.
 *
 * Returns: array of { pickNumber, round, playerName, espnPlayerId,
 *          eligiblePositions: string[], teamName, price, isMyPick }
 */
function scrapePickHistory() {
  const picks = [];
  try {
    const tables = document.querySelectorAll(SELECTORS.pickHistoryTable);
    if (tables.length === 0) return picks;

    // Determine picks per round from the maximum data row count across all
    // tables. Using the max (rather than just the first table) handles the
    // edge case where the extension connects mid-Round-1 before all teams
    // have picked, which would undercount if we only checked tables[0].
    let picksPerRound = 0;
    tables.forEach((table) => {
      const tableRows = table.querySelectorAll('[aria-rowindex]');
      let dataRowCount = 0;
      tableRows.forEach((r) => {
        if (parseInt(r.getAttribute('aria-rowindex'), 10) > 1) {
          dataRowCount++;
        }
      });
      if (dataRowCount > picksPerRound) {
        picksPerRound = dataRowCount;
      }
    });
    if (picksPerRound === 0) {
      picksPerRound = 10; // fallback: assumes 10-team league
      warn('Could not determine picks per round from tables, falling back to', picksPerRound);
    }

    tables.forEach((table) => {
      // Extract round number from caption
      const captionEl = table.querySelector(SELECTORS.pickHistoryCaption);
      const captionText = captionEl ? captionEl.textContent.trim() : '';
      const roundMatch = captionText.match(/Round\s+(\d+)/i);
      const round = roundMatch ? parseInt(roundMatch[1], 10) : 0;
      if (round === 0) return;

      // Find data rows (aria-rowindex > 1, skipping the header)
      const rows = table.querySelectorAll('[aria-rowindex]');
      rows.forEach((row) => {
        const rowIndex = parseInt(row.getAttribute('aria-rowindex'), 10);
        if (rowIndex <= 1) return; // Skip header row

        // Get all cells, sorted by their CSS left offset
        const cells = Array.from(row.querySelectorAll('[role="gridcell"]'));
        if (cells.length < 4) return;

        // Sort cells by their CSS left offset for consistent column ordering.
        // Use parseFloat to handle fractional px values. Treat missing/NaN
        // left values as Infinity so unstyled cells sort to the end rather
        // than all collapsing to position 0 (which would make the sort unstable).
        cells.sort((a, b) => {
          const leftA = parseFloat(a.style.left);
          const leftB = parseFloat(b.style.left);
          return (isNaN(leftA) ? Infinity : leftA) - (isNaN(leftB) ? Infinity : leftB);
        });

        // Cell 1: Pick number within round
        const pickInRound = parseInt(
          extractText(cells[0], '.cellContent') || cells[0].textContent.trim(),
          10
        ) || 0;
        if (pickInRound === 0) return;

        // Cell 2: Player column
        const playerCol = cells[1];
        const nameAnchor = playerCol.querySelector(
          'span.playerinfo__playername span.truncate a[title]'
        );
        const playerName = nameAnchor ? nameAnchor.getAttribute('title') : '';
        if (!playerName) return;

        // ESPN player ID from headshot image URL
        let espnPlayerId = '';
        const headshot = playerCol.querySelector('img');
        if (headshot) {
          const src = headshot.src || '';
          const idMatch = src.match(/\/full\/(\d+)\.png/);
          if (idMatch) {
            espnPlayerId = idMatch[1];
          }
        }

        // Eligible positions from ALL positionPill spans
        const positionPills = playerCol.querySelectorAll(
          'span.playerinfo__playerpos span.positionPill'
        );
        const eligiblePositions = [];
        positionPills.forEach((pill) => {
          const pos = pill.textContent.trim();
          if (pos) eligiblePositions.push(pos);
        });

        // Is this the user's pick?
        const playerColumnDiv = playerCol.querySelector('.player-column');
        const isMyPick = playerColumnDiv
          ? playerColumnDiv.classList.contains('my-pick')
          : false;

        // Cell 3: Team name
        const teamCell = cells[2];
        const boldTeam = teamCell.querySelector('span.fw-bold');
        const teamName = boldTeam
          ? boldTeam.textContent.trim()
          : teamCell.textContent.trim();

        // Cell 4: Price
        const priceCell = cells[3];
        const price = parsePrice(priceCell.textContent);

        // Global pick number
        const pickNumber = (round - 1) * picksPerRound + pickInRound;

        picks.push({
          pickNumber: pickNumber,
          round: round,
          playerName: playerName,
          espnPlayerId: espnPlayerId,
          eligiblePositions: eligiblePositions,
          teamName: teamName,
          price: price,
          isMyPick: isMyPick,
        });
      });
    });
  } catch (e) {
    error('Error scraping pick history:', e);
  }
  return picks;
}

/**
 * Scrape the roster dropdown for team name -> ESPN team ID mapping.
 *
 * The roster section has a `<select>` dropdown where each `<option>` maps
 * a team name to its ESPN numeric ID (e.g. `<option value="2">Team Name</option>`).
 *
 * Returns: array of { teamName, espnTeamId }
 */
function scrapeTeamIdMapping() {
  if (cachedTeamIdMapping) return cachedTeamIdMapping;
  const mapping = [];
  try {
    const dropdown = document.querySelector(SELECTORS.teamIdDropdown);
    if (!dropdown) return mapping;

    const options = dropdown.querySelectorAll('option');
    options.forEach((option) => {
      const teamName = option.textContent.trim();
      const espnTeamId = option.value;
      if (teamName && espnTeamId) {
        mapping.push({
          teamName: teamName,
          espnTeamId: espnTeamId,
        });
      }
    });
  } catch (e) {
    error('Error scraping team ID mapping:', e);
  }
  if (mapping.length > 0) {
    cachedTeamIdMapping = mapping;
  }
  return mapping;
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

    // Scrape team ID mapping from roster dropdown
    state.teamIdMapping = scrapeTeamIdMapping();
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

/** Cached team ID mapping (static for the duration of a draft) */
let cachedTeamIdMapping = null;

/** Cached my team name (static for the duration of a draft) */
let cachedMyTeamName = null;

/**
 * Compute a lightweight fingerprint of the state for deduplication.
 * Excludes timeRemaining since it changes every second and would defeat dedup.
 */
function computeFingerprint(state) {
  const picks = state.picks || [];
  const nom = state.currentNomination;
  const teams = state.teams || [];
  const teamBudgets = teams.map((t) => t.teamName + ':' + t.budget).join(',');

  // Include draft board grid filled slot count so that late-rendering
  // grids trigger a fingerprint change (and thus a new FULL_STATE_SYNC
  // via the periodic keyframe).
  let gridFilledCount = 0;
  try {
    const pickCells = document.querySelectorAll(SELECTORS.draftBoardCell);
    pickCells.forEach((cell) => {
      if (cell.classList.contains('completedPick')) {
        gridFilledCount++;
      }
    });
  } catch (e) {
    // Grid not available
  }

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
    (state.draftId || '') +
    '|g' +
    gridFilledCount
  );
}

/**
 * Build a state payload object from the current scraped state.
 *
 * @param {Object} state - The scraped state
 * @param {Object} [extras] - Optional extra fields (pickHistory, draftBoard)
 *                            that are only included on FULL_STATE_SYNC
 */
function buildStatePayload(state, extras) {
  const payload = {
    picks: state.picks || [],
    currentNomination: state.currentNomination || null,
    myTeamId: state.myTeamId || null,
    teams: state.teams || [],
    pickCount: state.pickCount ?? null,
    totalPicks: state.totalPicks ?? null,
    draftId: state.draftId || null,
    source: state.source || 'unknown',
    teamIdMapping: state.teamIdMapping || null,
  };

  // Only include expensive data when explicitly provided (FULL_STATE_SYNC)
  if (extras) {
    if (extras.pickHistory) payload.pickHistory = extras.pickHistory;
    if (extras.draftBoard) payload.draftBoard = extras.draftBoard;
  }

  return payload;
}

/**
 * Send a full state snapshot to the background script with type FULL_STATE_SYNC.
 *
 * Called on initial connect or reconnect so the backend can reset its in-memory
 * draft state and rebuild it from scratch. Unlike STATE_UPDATE (which carries
 * incremental diffs), FULL_STATE_SYNC always includes the complete current pick
 * history and team budgets visible on the page.
 */
function sendFullStateSync() {
  const state = scrapeDom();
  if (!state) return;

  // Scrape expensive data only on FULL_STATE_SYNC
  const pickHistory = scrapePickHistory();
  const draftBoard = scrapeDraftBoard();

  log(
    'Sending FULL_STATE_SYNC with',
    (state.picks || []).length,
    'pick log entries,',
    pickHistory.length,
    'pick history entries'
  );

  const message = {
    source: 'wyndham-draft-sync',
    type: 'FULL_STATE_SYNC',
    timestamp: Date.now(),
    payload: buildStatePayload(state, {
      pickHistory: pickHistory,
      draftBoard: draftBoard,
    }),
  };

  try {
    browser.runtime.sendMessage(message).catch((err) => {
      warn('Failed to send FULL_STATE_SYNC to background:', err.message || err);
    });
  } catch (e) {
    warn('runtime.sendMessage not available:', e.message || e);
  }

  // Update fingerprint so the next STATE_UPDATE doesn't re-send the same data
  lastFingerprint = computeFingerprint(state);
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
  log('Initializing ESPN draft page scraper (DOM-only mode)');

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
