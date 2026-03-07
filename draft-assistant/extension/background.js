// Background script for Wyndham Draft Sync extension.
// Manages WebSocket connection to the Rust backend and relays messages from
// content scripts.

'use strict';

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const WS_URL = 'ws://localhost:9001';
const HEARTBEAT_INTERVAL_MS = 5000;
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;
const EXTENSION_VERSION = browser.runtime.getManifest().version;

const LOG_PREFIX = '[WyndhamDraftSync:BG]';

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let ws = null;
let heartbeatTimer = null;
let reconnectTimer = null;
let reconnectDelay = RECONNECT_BASE_MS;
let isConnected = false;

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

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
// WebSocket connection management
// ---------------------------------------------------------------------------

/**
 * Send a JSON message over the WebSocket if connected.
 * Returns true if the message was sent, false otherwise.
 */
function wsSend(message) {
  if (!ws || ws.readyState !== WebSocket.OPEN) {
    return false;
  }
  try {
    ws.send(JSON.stringify(message));
    return true;
  } catch (e) {
    warn('Failed to send WebSocket message:', e.message || e);
    return false;
  }
}

/**
 * Send the EXTENSION_CONNECTED handshake message.
 */
function sendHandshake() {
  const handshake = {
    type: 'EXTENSION_CONNECTED',
    payload: {
      platform: 'firefox',
      extensionVersion: EXTENSION_VERSION,
    },
  };
  if (wsSend(handshake)) {
    log('Sent EXTENSION_CONNECTED handshake');
  }
}

/**
 * Send a heartbeat message to keep the connection alive.
 */
function sendHeartbeat() {
  const heartbeat = {
    type: 'EXTENSION_HEARTBEAT',
    payload: {
      timestamp: Date.now(),
    },
  };
  wsSend(heartbeat);
}

/**
 * Start the heartbeat interval.
 */
function startHeartbeat() {
  stopHeartbeat();
  heartbeatTimer = setInterval(sendHeartbeat, HEARTBEAT_INTERVAL_MS);
}

/**
 * Stop the heartbeat interval.
 */
function stopHeartbeat() {
  if (heartbeatTimer) {
    clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }
}

/**
 * Schedule a reconnection attempt with exponential backoff.
 */
function scheduleReconnect() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
  }

  log(`Scheduling reconnect in ${reconnectDelay}ms`);

  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connect();
  }, reconnectDelay);

  // Exponential backoff: double the delay each time, up to the max
  reconnectDelay = Math.min(reconnectDelay * 2, RECONNECT_MAX_MS);
}

/**
 * Establish a WebSocket connection to the backend.
 */
function connect() {
  // Clean up any existing connection
  if (ws) {
    try {
      ws.close();
    } catch (e) {
      // Ignore close errors on stale socket
    }
    ws = null;
  }

  log(`Connecting to ${WS_URL}...`);

  try {
    ws = new WebSocket(WS_URL);
  } catch (e) {
    error('Failed to create WebSocket:', e.message || e);
    scheduleReconnect();
    return;
  }

  ws.onopen = () => {
    log('WebSocket connected');
    isConnected = true;

    // Reset reconnect backoff on successful connection
    reconnectDelay = RECONNECT_BASE_MS;

    // Send handshake
    sendHandshake();

    // Start heartbeat
    startHeartbeat();

    // Request a full state snapshot from the content script so the backend
    // can rebuild draft state from scratch rather than applying diffs against
    // a blank slate. This is critical when resuming a mid-draft session after
    // a disconnect. We use a small delay to allow the handshake to complete.
    requestFullStateSyncFromContentScript();
  };

  ws.onclose = (event) => {
    log(`WebSocket closed: code=${event.code} reason=${event.reason}`);
    isConnected = false;
    ws = null;
    stopHeartbeat();
    scheduleReconnect();
  };

  ws.onerror = (event) => {
    warn('WebSocket error:', event);
    // onclose will also fire after onerror, which handles reconnection
  };

  ws.onmessage = (event) => {
    log('Received from backend:', event.data);
    try {
      const msg = JSON.parse(event.data);
      if (msg.type === 'REQUEST_KEYFRAME') {
        log('Backend requested keyframe — forwarding to content script');
        requestFullStateSyncFromContentScript();
      }
    } catch (e) {
      warn('Failed to parse backend message:', e.message || e);
    }
  };
}

/**
 * Request a FULL_STATE_SYNC from the active ESPN draft tab content script.
 *
 * Sends a REQUEST_FULL_STATE_SYNC message to any active ESPN draft tab so
 * the content script will respond with a FULL_STATE_SYNC message (which is
 * then forwarded to the backend via WebSocket). This is called whenever the
 * WebSocket connects or reconnects so the backend can rebuild from the full
 * current state rather than starting from a blank slate.
 */
function requestFullStateSyncFromContentScript() {
  browser.tabs.query({ url: '*://fantasy.espn.com/baseball/draft*' }).then((tabs) => {
    if (!tabs || tabs.length === 0) {
      log('No active ESPN draft tab found for FULL_STATE_SYNC request');
      return;
    }
    // Send to all matching ESPN draft tabs (usually just one)
    tabs.forEach((tab) => {
      browser.tabs.sendMessage(tab.id, {
        source: 'wyndham-draft-sync-bg',
        type: 'REQUEST_FULL_STATE_SYNC',
      }).catch((err) => {
        // Content script may not be loaded yet (e.g. page still loading)
        log('Could not reach content script on tab', tab.id, ':', err.message || err);
      });
    });
  }).catch((err) => {
    warn('Failed to query tabs for FULL_STATE_SYNC request:', err.message || err);
  });
}

// ---------------------------------------------------------------------------
// ESPN Fantasy API fetch
// ---------------------------------------------------------------------------

/**
 * Fetch ESPN Fantasy API data from the background script.
 * Extensions bypass CORS, so this works without page-context injection.
 * Returns parsed JSON or null on failure.
 */
async function fetchEspnApi(tabUrl) {
  try {
    const url = new URL(tabUrl);
    const leagueId = url.searchParams.get('leagueId');
    if (!leagueId) {
      log('No leagueId in tab URL:', tabUrl);
      return null;
    }
    const year = new Date().getFullYear();
    const apiUrl = 'https://lm-api-reads.fantasy.espn.com/apis/v3/games/flb/seasons/' + year + '/segments/0/leagues/' + leagueId + '?view=mDraftDetail&view=mTeam&view=mRoster';

    log('Fetching ESPN API:', apiUrl);
    const resp = await fetch(apiUrl, { credentials: 'include' });
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    const data = await resp.json();
    log('ESPN API response received, top-level keys:', Object.keys(data));
    return data;
  } catch (e) {
    warn('ESPN API fetch failed:', e.message || e);
    return null;
  }
}

/**
 * Parse the ESPN Fantasy API response into team roster data.
 *
 * The API response format is a best guess — this function parses VERY
 * defensively with null checks at every level. If any expected field is
 * missing, logs a warning and returns null (falling back to DOM scraping).
 *
 * @param {Object} apiData - Parsed JSON from the ESPN Fantasy API
 * @returns {Array|null} Array of team roster objects, or null on parse failure
 */
function parseApiTeamRosters(apiData) {
  try {
    if (!apiData || typeof apiData !== 'object') {
      warn('ESPN API data is null or not an object');
      return null;
    }

    // Log top-level keys for debugging
    log('ESPN API response top-level keys:', Object.keys(apiData));

    const teams = apiData.teams;
    if (!Array.isArray(teams) || teams.length === 0) {
      warn('ESPN API: no teams array found');
      return null;
    }

    // Build price map from draftDetail.picks (used by both approaches)
    const priceMap = new Map();
    const draftDetail = apiData.draftDetail;
    if (draftDetail && Array.isArray(draftDetail.picks)) {
      for (const pick of draftDetail.picks) {
        if (pick && pick.playerId != null && pick.bidAmount != null) {
          priceMap.set(pick.playerId, pick.bidAmount);
        }
      }
    }
    log('ESPN API: built price map with', priceMap.size, 'entries from draftDetail.picks');

    // Build team info map from teams[]
    const teamInfoMap = new Map(); // teamId -> teamName
    for (const team of teams) {
      if (!team) continue;
      let teamName = '';
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
        teamInfoMap.set(team.id, teamName);
      }
    }

    // --- Primary approach: build from teams[].roster.entries ---
    const result = [];
    let totalPlayers = 0;

    for (const team of teams) {
      if (!team) continue;
      const teamId = team.id != null ? team.id : 0;
      const teamName = teamInfoMap.get(teamId) || ('Team ' + teamId);
      const players = [];

      if (team.roster && Array.isArray(team.roster.entries)) {
        for (const entry of team.roster.entries) {
          if (!entry) continue;
          const playerId = entry.playerId != null ? String(entry.playerId) : '';
          const lineupSlotId = entry.lineupSlotId != null ? entry.lineupSlotId : 16;

          let playerName = '';
          let eligibleSlots = [];
          const ppe = entry.playerPoolEntry;
          if (ppe && ppe.player) {
            const player = ppe.player;
            playerName = player.fullName || ((player.firstName || '') + ' ' + (player.lastName || '')).trim();
            eligibleSlots = Array.isArray(player.eligibleSlots) ? player.eligibleSlots : [];
          }

          const numericId = entry.playerId != null ? entry.playerId : parseInt(playerId, 10);
          const price = priceMap.has(numericId) ? priceMap.get(numericId) : 0;

          if (playerName) {
            players.push({ playerId, playerName, lineupSlotId, eligibleSlots, price });
          }
        }
      }

      result.push({ teamId, teamName, players });
      totalPlayers += players.length;
    }

    if (totalPlayers > 0) {
      log('ESPN API: built rosters from roster.entries —', totalPlayers, 'players across', result.length, 'teams');
      return result;
    }

    // --- Fallback: build from draftDetail.picks ---
    log('ESPN API: roster.entries produced 0 players, falling back to draftDetail.picks');

    if (!draftDetail || !Array.isArray(draftDetail.picks)) {
      warn('ESPN API: no draftDetail.picks found either, cannot build rosters');
      return null;
    }

    log('ESPN API draftDetail has', draftDetail.picks.length, 'picks');
    if (draftDetail.picks.length > 0) {
      log('ESPN API first pick keys:', Object.keys(draftDetail.picks[0]));
    }

    // Build player info map from teams[].roster.entries[].playerPoolEntry.player
    // (even if entries had no players, some might have player info without lineupSlotId)
    const playerInfoMap = new Map(); // playerId (number) -> {name, eligibleSlots}
    for (const team of teams) {
      if (!team || !team.roster || !Array.isArray(team.roster.entries)) continue;
      for (const entry of team.roster.entries) {
        if (!entry || entry.playerId == null) continue;
        const ppe = entry.playerPoolEntry;
        if (ppe && ppe.player) {
          const player = ppe.player;
          const name = player.fullName || ((player.firstName || '') + ' ' + (player.lastName || '')).trim();
          const eligibleSlots = Array.isArray(player.eligibleSlots) ? player.eligibleSlots : [];
          if (name) {
            playerInfoMap.set(entry.playerId, { name, eligibleSlots });
          }
        }
      }
    }
    log('ESPN API: built player info map with', playerInfoMap.size, 'entries from roster data');

    // Use draftDetail.picks, group by teamId
    const teamPlayersMap = new Map(); // teamId -> Array of player objects
    for (const pick of draftDetail.picks) {
      if (!pick || pick.playerId == null || pick.teamId == null) continue;

      const info = playerInfoMap.get(pick.playerId);
      const playerName = info ? info.name : '';
      const eligibleSlots = info ? info.eligibleSlots : [];
      const lineupSlotId = pick.lineupSlotId != null ? pick.lineupSlotId : 16;

      if (!playerName) {
        warn('ESPN API: pick for playerId', pick.playerId, 'has no player info in roster data, skipping');
        continue;
      }

      if (!teamPlayersMap.has(pick.teamId)) {
        teamPlayersMap.set(pick.teamId, []);
      }
      teamPlayersMap.get(pick.teamId).push({
        playerId: String(pick.playerId),
        playerName: playerName,
        lineupSlotId: lineupSlotId,
        eligibleSlots: eligibleSlots,
        price: pick.bidAmount != null ? pick.bidAmount : 0,
      });
    }

    // Build fallback result, including teams that have no picks yet
    const fallbackResult = [];
    for (const [fbTeamId, fbTeamName] of teamInfoMap) {
      const players = teamPlayersMap.get(fbTeamId) || [];
      fallbackResult.push({
        teamId: fbTeamId,
        teamName: fbTeamName,
        players: players,
      });
    }

    const fallbackTotalPlayers = fallbackResult.reduce((sum, t) => sum + t.players.length, 0);
    if (fallbackTotalPlayers === 0) {
      warn('ESPN API: draftDetail.picks also produced 0 players across all teams, returning null');
      return null;
    }

    log('ESPN API: built rosters from draftDetail.picks (fallback) —', fallbackTotalPlayers, 'players across', fallbackResult.length, 'teams');
    return fallbackResult;
  } catch (e) {
    error('Failed to parse ESPN API team rosters:', e);
    return null;
  }
}

// ---------------------------------------------------------------------------
// Message relay from content scripts
// ---------------------------------------------------------------------------

/**
 * Forward a content script message to the WebSocket backend.
 */
function forwardToWebSocket(message) {
  const forwarded = {
    type: message.type,
    timestamp: message.timestamp,
    payload: message.payload,
  };

  if (isConnected) {
    if (wsSend(forwarded)) {
      // Message sent successfully
    } else {
      warn('WebSocket send failed; message dropped');
    }
  } else {
    warn('WebSocket not connected; dropping message of type:', message.type);
  }
}

/**
 * Enrich a FULL_STATE_SYNC message with ESPN API roster data,
 * then forward to the WebSocket backend.
 */
async function enrichAndForward(message, tabUrl) {
  try {
    const apiData = await fetchEspnApi(tabUrl);
    if (apiData) {
      const teamRosters = parseApiTeamRosters(apiData);
      if (teamRosters && teamRosters.length > 0) {
        const totalPlayers = teamRosters.reduce((sum, t) => sum + t.players.length, 0);
        if (totalPlayers > 0) {
          message.payload.teamRosters = teamRosters;
          log('Enriched FULL_STATE_SYNC with', totalPlayers, 'players from', teamRosters.length, 'teams via API');
        } else {
          log('API returned teams but 0 players — not enriching');
        }
      }
    }
  } catch (e) {
    warn('API enrichment failed, forwarding DOM-only data:', e.message || e);
  }

  // Forward (enriched or not) to WebSocket
  forwardToWebSocket(message);
}

/**
 * Handle messages from content scripts.
 * For FULL_STATE_SYNC messages, enriches with ESPN API data before forwarding.
 * Other message types are forwarded immediately.
 *
 * IMPORTANT: The Rust backend deserializes ExtensionMessage using
 * #[serde(tag = "type")] (internally-tagged enum). Only fields defined in
 * protocol.rs are allowed at the top level -- extra fields will cause
 * deserialization to fail silently.
 */
browser.runtime.onMessage.addListener((message, sender) => {
  // Only process messages from our content script
  if (!message || message.source !== 'wyndham-draft-sync') {
    return;
  }

  const tabId = sender.tab ? sender.tab.id : null;
  log('Received', message.type, 'from tab', tabId);

  if (message.type === 'FULL_STATE_SYNC') {
    // Enrich FULL_STATE_SYNC with API data before forwarding
    const tabUrl = sender.tab ? sender.tab.url : '';
    enrichAndForward(message, tabUrl);
    return; // Don't forward yet — enrichAndForward handles it
  }

  // Forward other message types immediately
  forwardToWebSocket(message);
});

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

log('Background script starting');
connect();
