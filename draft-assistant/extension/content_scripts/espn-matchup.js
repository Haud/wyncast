// ESPN Matchup/Boxscore Page Content Script
// Scrapes matchup state from ESPN's DOM and relays it to the background script.
// Activates only on boxscore pages (URL path contains /boxscore).

'use strict';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const LOG_PREFIX = '[WyndhamMatchupSync]';
const MUTATION_DEBOUNCE_MS = 250;
const POLL_INTERVAL_MS = 5000;
const CONTAINER_POLL_MS = 500;
const CONTAINER_TIMEOUT_MS = 15000;

// ESPN stat IDs for the 12 H2H categories
const LOWER_IS_BETTER_STATS = new Set([47, 41]); // ERA, WHIP

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

function log(...args) {
  console.log(LOG_PREFIX, ...args);
}

function warn(...args) {
  console.warn(LOG_PREFIX, ...args);
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

/**
 * Extract trimmed text content from the first element matching a selector.
 */
function extractText(parent, selector) {
  if (!parent) return '';
  try {
    const el = parent.querySelector(selector);
    return el ? el.textContent.trim() : '';
  } catch (_e) {
    return '';
  }
}

/**
 * Parse a stat value string to a number.
 * Handles integers, decimals, and rate stats (e.g. ".250", "3.50").
 * Returns null for empty/missing values ("--", "", etc.).
 */
function parseStatValue(text) {
  if (!text || text === '--' || text === '-') return null;
  const cleaned = text.trim();
  if (cleaned === '') return null;
  const num = parseFloat(cleaned);
  return isNaN(num) ? null : num;
}

/**
 * Parse matchup period info from the page title.
 * Format: "Matchup 1 (Mar 25 - Apr 5) Box Score - League Name"
 * Returns { period, startDate, endDate } or null.
 */
function parseMatchupPeriodFromTitle() {
  try {
    const title = document.title || '';
    const match = title.match(/Matchup\s+(\d+)\s*\(([^)]+)\)/);
    if (!match) return null;

    const period = parseInt(match[1], 10);
    const dateRange = match[2]; // e.g. "Mar 25 - Apr 5"

    const dates = parseDateRange(dateRange);
    return {
      period: period,
      startDate: dates ? dates.start : null,
      endDate: dates ? dates.end : null,
    };
  } catch (_e) {
    return null;
  }
}

/**
 * Parse a date range string like "Mar 25 - Apr 5" into ISO date strings.
 * Infers the year from the current season context.
 */
function parseDateRange(rangeStr) {
  if (!rangeStr) return null;
  try {
    const parts = rangeStr.split('-').map(s => s.trim());
    if (parts.length !== 2) return null;

    const year = inferSeasonYear();
    const start = parseMonthDay(parts[0], year);
    const end = parseMonthDay(parts[1], year);

    if (!start || !end) return null;
    return { start: start, end: end };
  } catch (_e) {
    return null;
  }
}

/**
 * Parse "Mar 25" into "YYYY-MM-DD" format.
 */
function parseMonthDay(str, year) {
  const months = {
    'jan': '01', 'feb': '02', 'mar': '03', 'apr': '04',
    'may': '05', 'jun': '06', 'jul': '07', 'aug': '08',
    'sep': '09', 'oct': '10', 'nov': '11', 'dec': '12',
  };

  const match = str.trim().match(/^([A-Za-z]+)\s+(\d+)$/);
  if (!match) return null;

  const monthKey = match[1].toLowerCase().substring(0, 3);
  const month = months[monthKey];
  if (!month) return null;

  const day = match[2].padStart(2, '0');
  return `${year}-${month}-${day}`;
}

/**
 * Infer the MLB season year. Try __NEXT_DATA__ first, then fall back to
 * current date logic (MLB seasons run Mar-Oct, so Jan-Feb = current year).
 */
function inferSeasonYear() {
  try {
    if (typeof __NEXT_DATA__ !== 'undefined' && __NEXT_DATA__.props) {
      const season = __NEXT_DATA__.props.pageProps.page.config.currentSeason;
      if (season) return season;
    }
  } catch (_e) {
    // Fall through
  }
  return new Date().getFullYear();
}

// ---------------------------------------------------------------------------
// __NEXT_DATA__ helpers
// ---------------------------------------------------------------------------

/**
 * Try to extract matchup period dates from __NEXT_DATA__ scoring periods.
 * Returns { startDate, endDate } for the given matchup period, or null.
 */
function getMatchupDatesFromNextData(matchupPeriod) {
  try {
    if (typeof __NEXT_DATA__ === 'undefined') return null;

    const consts = __NEXT_DATA__.props.pageProps.page.config.constants;
    const scoringPeriods = consts.scoringPeriods;
    const segments = consts.segments;

    // Find the weekly period type (id=2) which maps matchup periods to scoring periods
    let weeklyPeriods = null;
    for (const seg of segments) {
      for (const pt of seg.periodTypes || []) {
        if (pt.weekly) {
          weeklyPeriods = pt.periods;
          break;
        }
      }
      if (weeklyPeriods) break;
    }

    if (!weeklyPeriods) return null;

    // Find the matchup period entry
    const mp = weeklyPeriods.find(p => p.id === matchupPeriod);
    if (!mp) return null;

    // Build scoring period lookup
    const spLookup = {};
    for (const sp of scoringPeriods) {
      spLookup[sp.id] = sp;
    }

    const startSp = spLookup[mp.scoringPeriodStart];
    const endSp = spLookup[mp.scoringPeriodEnd];
    if (!startSp || !endSp) return null;

    // Convert epoch ms to YYYY-MM-DD
    const startDate = new Date(startSp.startDate).toISOString().split('T')[0];
    const endDate = new Date(endSp.endDate).toISOString().split('T')[0];

    return { startDate: startDate, endDate: endDate };
  } catch (_e) {
    return null;
  }
}

// ---------------------------------------------------------------------------
// DOM Scraping: Matchup Header
// ---------------------------------------------------------------------------

/**
 * Scrape team info from the H2H matchup header.
 * Returns { myTeam, oppTeam } or null.
 */
function scrapeMatchupHeader() {
  try {
    const headers = document.querySelectorAll('.h2h-matchup-header');
    if (headers.length < 2) return null;

    const awayHeader = document.querySelector('.h2h-matchup-header.away-team');
    const homeHeader = document.querySelector('.h2h-matchup-header.home-team');
    if (!awayHeader || !homeHeader) return null;

    return {
      awayTeam: scrapeTeamHeader(awayHeader),
      homeTeam: scrapeTeamHeader(homeHeader),
    };
  } catch (_e) {
    return null;
  }
}

/**
 * Scrape a single team's info from its matchup header div.
 */
function scrapeTeamHeader(headerEl) {
  const name = extractText(headerEl, '.teamName') ||
               extractText(headerEl, '.team--link');
  const record = extractText(headerEl, '.team-record');
  const score = extractText(headerEl, '.team-score h2') ||
                extractText(headerEl, '.team-score');

  return {
    name: name,
    record: record,
    matchup_score: score,
  };
}

// ---------------------------------------------------------------------------
// DOM Scraping: Category Scoreboard
// ---------------------------------------------------------------------------

/**
 * Scrape the category scoreboard table.
 * Returns an array of category objects with stat_id, abbrev, my_value, opp_value.
 */
function scrapeScoreboard() {
  const categories = [];
  try {
    // Find stat column headers with data-statid attributes
    const statHeaders = document.querySelectorAll('th span[data-statid]');
    if (statHeaders.length === 0) return categories;

    // Build ordered list of stat definitions from headers
    const statDefs = [];
    statHeaders.forEach(span => {
      const statId = parseInt(span.getAttribute('data-statid'), 10);
      const abbrev = span.textContent.trim();
      if (!isNaN(statId)) {
        statDefs.push({ statId: statId, abbrev: abbrev });
      }
    });

    // The scoreboard has only 12 stat headers (the H2H categories)
    // These appear in the first responsive table that has the team score rows
    // Find the summary table rows (Table__TR--md with team names)
    const summaryRows = document.querySelectorAll('.Table__TR--md');
    if (summaryRows.length < 2) return categories;

    const awayRow = summaryRows[0];
    const homeRow = summaryRows[1];

    // Extract stat values from each row
    // The row structure is: [team-name] [stat1] [stat2] ... [stat12] [score]
    const awayCells = awayRow.querySelectorAll('td');
    const homeCells = homeRow.querySelectorAll('td');

    // First cell is team name, last cell is team score
    // Stat cells are indices 1 through (length - 2)
    // We only want the first 12 stat values (matching our category headers)
    const headerCount = Math.min(statDefs.length, 12);

    for (let i = 0; i < headerCount; i++) {
      const awayCell = awayCells[i + 1]; // +1 to skip team name cell
      const homeCell = homeCells[i + 1];

      const awayText = awayCell ? awayCell.textContent.trim() : '';
      const homeText = homeCell ? homeCell.textContent.trim() : '';

      categories.push({
        stat_id: statDefs[i].statId,
        abbrev: statDefs[i].abbrev,
        my_value: parseStatValue(awayText),
        opp_value: parseStatValue(homeText),
        lower_is_better: LOWER_IS_BETTER_STATS.has(statDefs[i].statId),
      });
    }
  } catch (e) {
    warn('Error scraping scoreboard:', e);
  }
  return categories;
}

// ---------------------------------------------------------------------------
// DOM Scraping: Player Tables
// ---------------------------------------------------------------------------

/**
 * Scrape all player tables for a given section type ("Batters" or "Pitchers").
 * ESPN renders separate batting and pitching responsive tables.
 * Returns { headers, players, totals }.
 */
function scrapePlayerSection(sectionType) {
  const result = { headers: [], players: [], totals: [] };

  try {
    // Find all players-table containers
    const tables = document.querySelectorAll('.players-table');

    for (const table of tables) {
      // Check if this table is for the requested section type
      const sectionHeader = table.querySelector('th[colspan]');
      if (!sectionHeader) continue;
      const headerText = sectionHeader.textContent.trim();
      if (headerText !== sectionType) continue;

      // Extract stat column headers from the sub-header row
      // These are in the scrollable portion (second colgroup area)
      const subHeaderRow = table.querySelector('.Table__sub-header');
      if (!subHeaderRow) continue;

      // Get stat headers from spans with data-statid
      const statSpans = table.querySelectorAll('.Table__sub-header span[data-statid]');
      const headers = [];
      statSpans.forEach(span => {
        headers.push(span.textContent.trim());
      });
      result.headers = headers;

      // Extract player rows (Table__TR--lg)
      const playerRows = table.querySelectorAll('.Table__TR--lg');
      // Also get the scrollable stat cells — ESPN splits the table into
      // fixed-left (slot + player info) and scrollable (stats) portions.
      // Both use the same row indices via data-idx.

      // Build a map of data-idx -> stat cells from the scrollable side
      const statCellsByIdx = buildStatCellMap(table, headers.length);

      playerRows.forEach(row => {
        const idx = row.getAttribute('data-idx');
        const player = scrapePlayerRow(row, statCellsByIdx[idx] || []);
        if (player) {
          result.players.push(player);
        }
      });

      // Look for TOTALS row
      const totals = scrapeTotalsRow(table, headers.length);
      if (totals) {
        result.totals = totals;
      }

      break; // Found our section, done
    }
  } catch (e) {
    warn('Error scraping', sectionType, 'section:', e);
  }
  return result;
}

/**
 * Build a map from data-idx to stat value cells for the scrollable portion.
 * ESPN splits the table: fixed-left has slot/player/opp/status, scrollable has stats.
 * We need to correlate them by row index.
 */
function buildStatCellMap(tableContainer, headerCount) {
  const map = {};
  try {
    // The scrollable table is the second Table element in the flex container
    const allTables = tableContainer.querySelectorAll('table.Table');
    if (allTables.length < 2) return map;

    const scrollTable = allTables[allTables.length - 1]; // Last table is scrollable
    const rows = scrollTable.querySelectorAll('.Table__TBODY .Table__TR');

    rows.forEach(row => {
      const idx = row.getAttribute('data-idx');
      if (idx === null) return;

      const cells = row.querySelectorAll('td');
      const values = [];
      cells.forEach(cell => {
        const text = cell.textContent.trim();
        values.push(parseStatValue(text));
      });
      map[idx] = values;
    });
  } catch (_e) {
    // Fall through with empty map
  }
  return map;
}

/**
 * Scrape a single player row from the fixed-left portion.
 */
function scrapePlayerRow(row, statValues) {
  try {
    // Slot: first table--cell text (C, 1B, SP, BENCH, etc.)
    const slotCell = row.querySelector('.table--cell');
    const slot = slotCell ? slotCell.textContent.trim() : '';

    // Player name
    const nameEl = row.querySelector('.player-column__athlete a.AnchorLink') ||
                   row.querySelector('.player-column__athlete .truncate');
    const name = nameEl ? nameEl.textContent.trim() : '';

    // Skip rows without a player name (empty slots, header rows)
    if (!name) return null;

    // Team abbreviation
    const teamEl = row.querySelector('.playerinfo__playerteam');
    const team = teamEl ? teamEl.textContent.trim() : '';

    // Positions (comma-separated)
    const posEl = row.querySelector('.playerinfo__playerpos');
    const posText = posEl ? posEl.textContent.trim() : '';
    const positions = posText ? posText.split(',').map(s => s.trim()).filter(Boolean) : [];

    // Opponent
    const oppCell = row.querySelector('.table--cell.opp');
    const opponent = oppCell ? oppCell.textContent.trim() : '--';

    // Game status
    const statusCell = row.querySelector('.table--cell.game-status');
    const status = statusCell ? statusCell.textContent.trim() || null : null;

    return {
      slot: slot,
      name: name,
      team: team,
      positions: positions,
      opponent: opponent === '' ? '--' : opponent,
      status: status,
      stats: statValues,
    };
  } catch (_e) {
    return null;
  }
}

/**
 * Scrape the TOTALS row from a player section table.
 * Returns an array of stat values, or null if not found.
 */
function scrapeTotalsRow(tableContainer, headerCount) {
  try {
    // TOTALS is in a row that spans the fixed-left portion with colspan
    // The stat values are in the scrollable portion's corresponding row
    const allTables = tableContainer.querySelectorAll('table.Table');
    if (allTables.length < 2) return null;

    const scrollTable = allTables[allTables.length - 1];
    const rows = scrollTable.querySelectorAll('.Table__TBODY .Table__TR');

    // Find the TOTALS row — it's typically marked with bg-clr-gray-08
    // or is the last non-bench/IL row
    for (const row of rows) {
      // Check both the row itself and the fixed-left table for TOTALS text
      const text = row.textContent.trim();
      if (text === '') {
        // This might be the totals row if the fixed side has TOTALS
        // Check by looking at the corresponding row in the fixed table
        continue;
      }
    }

    // Alternative approach: find TOTALS text in the fixed-left table
    const fixedTable = allTables[0];
    const fixedRows = fixedTable.querySelectorAll('.Table__TBODY .Table__TR');

    for (let i = 0; i < fixedRows.length; i++) {
      const fixedText = fixedRows[i].textContent.trim();
      if (fixedText.includes('TOTALS')) {
        // Get the corresponding scrollable row
        const scrollRows = scrollTable.querySelectorAll('.Table__TBODY .Table__TR');
        if (i < scrollRows.length) {
          const cells = scrollRows[i].querySelectorAll('td');
          const values = [];
          cells.forEach(cell => {
            values.push(parseStatValue(cell.textContent.trim()));
          });
          return values;
        }
      }
    }
  } catch (_e) {
    // TOTALS not found
  }
  return null;
}

// ---------------------------------------------------------------------------
// Identify "my team" (away team = logged-in user's team on boxscore page)
// ---------------------------------------------------------------------------

/**
 * Determine which team is "mine" (the logged-in user's team).
 * On ESPN matchup pages, the away team is always the team whose page the
 * user is viewing. If viewing your own matchup, you're the away team.
 * Returns 'away' or 'home'.
 */
function identifyMyTeamSide() {
  // The default ESPN boxscore shows the logged-in user's team as away.
  // This is consistent across the ESPN UI.
  return 'away';
}

// ---------------------------------------------------------------------------
// Full state assembly
// ---------------------------------------------------------------------------

/**
 * Scrape the full matchup state from the DOM.
 * Returns the complete MATCHUP_STATE payload, or null if the page isn't ready.
 */
function scrapeMatchupState() {
  try {
    const header = scrapeMatchupHeader();
    if (!header) return null;

    // Parse matchup period from title
    const periodInfo = parseMatchupPeriodFromTitle();
    const matchupPeriod = periodInfo ? periodInfo.period : null;

    // Get dates — try __NEXT_DATA__ first, then fall back to title parsing
    let startDate = null;
    let endDate = null;

    if (matchupPeriod) {
      const nextDataDates = getMatchupDatesFromNextData(matchupPeriod);
      if (nextDataDates) {
        startDate = nextDataDates.startDate;
        endDate = nextDataDates.endDate;
      }
    }

    if (!startDate && periodInfo) {
      startDate = periodInfo.startDate;
      endDate = periodInfo.endDate;
    }

    // Determine which side is "my team"
    const side = identifyMyTeamSide();
    const myTeam = side === 'away' ? header.awayTeam : header.homeTeam;
    const oppTeam = side === 'away' ? header.homeTeam : header.awayTeam;

    // Scrape category scoreboard
    const categories = scrapeScoreboard();

    // Scrape player sections
    const batting = scrapePlayerSection('Batters');
    const pitching = scrapePlayerSection('Pitchers');

    // Determine selected day from any day selector or default to today
    const selectedDay = detectSelectedDay() || new Date().toISOString().split('T')[0];

    return {
      matchup_period: matchupPeriod,
      start_date: startDate,
      end_date: endDate,
      selected_day: selectedDay,
      my_team: myTeam,
      opp_team: oppTeam,
      categories: categories,
      batting: batting,
      pitching: pitching,
    };
  } catch (e) {
    warn('Error assembling matchup state:', e);
    return null;
  }
}

/**
 * Try to detect which day is currently selected on the page.
 * ESPN may show a date dropdown or highlight the current day.
 * Falls back to today's date.
 */
function detectSelectedDay() {
  try {
    // Look for a dropdown or selector with date information
    // ESPN uses a dropdown with scoring period dates
    const dropdown = document.querySelector('.dropdown__select');
    if (dropdown && dropdown.selectedIndex >= 0) {
      const option = dropdown.options[dropdown.selectedIndex];
      if (option) {
        const text = option.textContent.trim();
        // Try to parse date from option text (e.g. "Wed 3/26")
        const dateMatch = text.match(/(\d{1,2})\/(\d{1,2})/);
        if (dateMatch) {
          const year = inferSeasonYear();
          const month = dateMatch[1].padStart(2, '0');
          const day = dateMatch[2].padStart(2, '0');
          return `${year}-${month}-${day}`;
        }
      }
    }
  } catch (_e) {
    // Fall through
  }
  return null;
}

// ---------------------------------------------------------------------------
// Message sending
// ---------------------------------------------------------------------------

let lastFingerprint = '';

/**
 * Compute a simple fingerprint of the matchup state for deduplication.
 */
function computeFingerprint(state) {
  if (!state) return '';
  try {
    // Include key fields that change during live games
    const parts = [
      state.matchup_period,
      state.selected_day,
      state.my_team ? state.my_team.matchup_score : '',
      state.opp_team ? state.opp_team.matchup_score : '',
      state.categories ? state.categories.map(c =>
        `${c.stat_id}:${c.my_value}:${c.opp_value}`
      ).join(',') : '',
      state.batting ? state.batting.players.length : 0,
      state.pitching ? state.pitching.players.length : 0,
      state.batting && state.batting.totals ?
        state.batting.totals.join(',') : '',
      state.pitching && state.pitching.totals ?
        state.pitching.totals.join(',') : '',
    ];
    return parts.join('|');
  } catch (_e) {
    return String(Date.now());
  }
}

/**
 * Send the matchup state to the background script.
 */
function sendMatchupState(state) {
  const message = {
    source: 'wyndham-draft-sync',
    type: 'MATCHUP_STATE',
    timestamp: Date.now(),
    payload: state,
  };

  try {
    browser.runtime.sendMessage(message).catch(err => {
      warn('Failed to send MATCHUP_STATE to background:', err.message || err);
    });
  } catch (e) {
    warn('runtime.sendMessage not available:', e.message || e);
  }
}

/**
 * Scrape and send state if it has changed since the last send.
 */
function handleStateUpdate() {
  const state = scrapeMatchupState();
  if (!state) return;

  const fingerprint = computeFingerprint(state);
  if (fingerprint === lastFingerprint) return;
  lastFingerprint = fingerprint;

  log('Sending MATCHUP_STATE update');
  sendMatchupState(state);
}

// ---------------------------------------------------------------------------
// MutationObserver + Polling
// ---------------------------------------------------------------------------

let mutationObserver = null;
let debounceTimer = null;

/**
 * Request state extraction with debouncing.
 */
function requestStateExtraction() {
  if (debounceTimer) {
    clearTimeout(debounceTimer);
  }
  debounceTimer = setTimeout(() => {
    debounceTimer = null;
    handleStateUpdate();
  }, MUTATION_DEBOUNCE_MS);
}

/**
 * Start observing a DOM element for mutations.
 */
function startObserving(target) {
  if (mutationObserver) {
    mutationObserver.disconnect();
  }

  mutationObserver = new MutationObserver(() => {
    requestStateExtraction();
  });

  mutationObserver.observe(target, {
    childList: true,
    subtree: true,
    characterData: true,
    attributes: true,
    attributeFilter: ['class', 'data-idx'],
  });

  log('MutationObserver attached to:', target.tagName, target.className || target.id || '');

  // Trigger an immediate extraction
  requestStateExtraction();
}

/**
 * Find the matchup page container element.
 */
function findMatchupContainer() {
  try {
    // Try specific matchup containers first
    return document.querySelector('.shell-container') ||
           document.querySelector('.page-container') ||
           document.querySelector('#fitt-analytics');
  } catch (_e) {
    return null;
  }
}

/**
 * Poll for the matchup container, then start observing.
 */
function initObserver() {
  const startTime = Date.now();

  const pollId = setInterval(() => {
    const container = findMatchupContainer();
    if (container) {
      clearInterval(pollId);
      log('Found matchup container element');
      startObserving(container);
      return;
    }

    if (Date.now() - startTime >= CONTAINER_TIMEOUT_MS) {
      clearInterval(pollId);
      warn('Matchup container not found after', CONTAINER_TIMEOUT_MS, 'ms. Falling back to document.body');
      startObserving(document.body);
    }
  }, CONTAINER_POLL_MS);
}

/**
 * Periodic polling fallback. Sends state updates at regular intervals
 * in case MutationObserver misses changes (React virtual DOM updates).
 */
function startPeriodicPolling() {
  setInterval(() => {
    requestStateExtraction();
  }, POLL_INTERVAL_MS);
}

// ---------------------------------------------------------------------------
// Message listener: respond to requests from the background script
// ---------------------------------------------------------------------------

function setupMessageListener() {
  try {
    browser.runtime.onMessage.addListener((message) => {
      if (!message || message.source !== 'wyndham-draft-sync-bg') return;

      if (message.type === 'REQUEST_FULL_STATE_SYNC') {
        log('Received REQUEST_FULL_STATE_SYNC from background');
        // Force a fresh send by clearing the fingerprint
        lastFingerprint = '';
        handleStateUpdate();
      }
    });
  } catch (e) {
    warn('Could not set up message listener:', e.message || e);
  }
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

function init() {
  log('Initializing ESPN matchup page scraper');

  // Set up message listener for background script requests
  setupMessageListener();

  // Poll for container and start observing
  initObserver();

  // Start periodic polling fallback
  startPeriodicPolling();

  log('Content script initialized');
}

// Run initialization
init();
