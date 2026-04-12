// Shared background core for Wyndham Draft Sync extension.
// Manages WebSocket connection to the Rust backend and relays messages from
// content scripts. Used by both Firefox (background page) and Chrome (offscreen document).

'use strict';

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

// Must match the WebSocket port in ws_server.rs
const WS_URL = 'ws://localhost:9001';
const HEARTBEAT_INTERVAL_MS = 5000;
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;
const ESPN_HOSTNAME = 'fantasy.espn.com';
const ESPN_BASEBALL_PATH_PREFIX = '/baseball/';

const LOG_PREFIX = '[WyndhamDraftSync:BG]';

/**
 * Check if a URL is an ESPN fantasy baseball page that we handle.
 * Matches any fantasy.espn.com/baseball/* path — content scripts
 * control which pages actually inject, so this is intentionally broad.
 */
function isEspnAppUrl(urlStr) {
  if (!urlStr) return false;
  try {
    const parsed = new URL(urlStr);
    return parsed.hostname === ESPN_HOSTNAME &&
           parsed.pathname.startsWith(ESPN_BASEBALL_PATH_PREFIX);
  } catch (e) {
    return false;
  }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let ws = null;
let heartbeatTimer = null;
let reconnectTimer = null;
let reconnectDelay = RECONNECT_BASE_MS;
let isConnected = false;
let intentionalDisconnect = false;

// ---------------------------------------------------------------------------
// Active tab tracking
// ---------------------------------------------------------------------------
// Only messages from the "primary" tab are relayed over the WebSocket.
// When multiple ESPN tabs are open (e.g. draft + matchup), the most recently
// active tab wins. Closing the primary tab falls back to the next most recent.

/** @type {Set<number>} All tab IDs that have sent a content-script message. */
const knownTabs = new Set();

/** @type {number|null} The tab whose messages are currently relayed. */
let primaryTabId = null;

/**
 * Make `tabId` the primary active tab. If the primary tab changes, log it.
 * Returns true if the tab is (now) the primary tab, false if it was already.
 */
function setPrimaryTab(tabId) {
  if (primaryTabId === tabId) return false;
  const prev = primaryTabId;
  primaryTabId = tabId;
  log('Primary tab changed:', prev, '->', tabId);
  return true;
}

/**
 * Remove a tab from tracking. If it was the primary tab, fall back to
 * another known tab (if any). Returns true if any tabs remain tracked.
 */
function removeTab(tabId) {
  knownTabs.delete(tabId);
  if (primaryTabId === tabId) {
    // Fall back to the most recently added remaining tab (Set iteration
    // order is insertion order; pick the last element).
    primaryTabId = null;
    for (const id of knownTabs) {
      primaryTabId = id;
    }
    if (primaryTabId !== null) {
      log('Primary tab closed; falling back to tab', primaryTabId);
    } else {
      log('Primary tab closed; no remaining ESPN tabs');
    }
  }
  return knownTabs.size > 0;
}

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
 * Uses the platform string from the config.
 */
function sendHandshake(config) {
  const handshake = {
    type: 'EXTENSION_CONNECTED',
    payload: {
      platform: config.platform,
      extensionVersion: config.extensionVersion,
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
function scheduleReconnect(config) {
  if (knownTabs.size === 0) {
    log('No active content script tabs; skipping reconnect');
    return;
  }

  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
  }

  log(`Scheduling reconnect in ${reconnectDelay}ms`);

  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connect(config);
  }, reconnectDelay);

  // Exponential backoff: double the delay each time, up to the max
  reconnectDelay = Math.min(reconnectDelay * 2, RECONNECT_MAX_MS);
}

/**
 * Request a FULL_STATE_SYNC from the primary active tab's content script.
 */
function requestFullStateSyncFromContentScript(config) {
  if (primaryTabId === null) {
    log('No primary tab for FULL_STATE_SYNC request');
    return;
  }
  config.sendToContentScript(primaryTabId, {
    source: 'wyndham-draft-sync-bg',
    type: 'REQUEST_FULL_STATE_SYNC',
  }).catch((err) => {
    log('Could not reach content script on tab', primaryTabId, ':', err.message || err);
  });
}

/**
 * Establish a WebSocket connection to the backend.
 */
function connect(config) {
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
    scheduleReconnect(config);
    return;
  }

  ws.onopen = () => {
    log('WebSocket connected');
    isConnected = true;

    // Reset reconnect backoff on successful connection
    reconnectDelay = RECONNECT_BASE_MS;

    // Send handshake
    sendHandshake(config);

    // Start heartbeat
    startHeartbeat();

    // Request a full state snapshot from the content script so the backend
    // can rebuild draft state from scratch rather than applying diffs against
    // a blank slate.
    requestFullStateSyncFromContentScript(config);
  };

  ws.onclose = (event) => {
    log(`WebSocket closed: code=${event.code} reason=${event.reason}`);
    isConnected = false;
    ws = null;
    stopHeartbeat();

    if (intentionalDisconnect) {
      intentionalDisconnect = false;
    } else {
      scheduleReconnect(config);
    }
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
        requestFullStateSyncFromContentScript(config);
      }
    } catch (e) {
      warn('Failed to parse backend message:', e.message || e);
    }
  };
}

/**
 * Cleanly disconnect the WebSocket. Used when all content script tabs have
 * closed so we don't hold an idle connection to the backend.
 */
function disconnect() {
  log('Disconnecting WebSocket (no active content script tabs)');
  stopHeartbeat();

  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }

  reconnectDelay = RECONNECT_BASE_MS;

  if (ws) {
    intentionalDisconnect = true;
    try {
      ws.close();
    } catch (e) {
      // Ignore close errors
    }
    ws = null;
    isConnected = false;
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Initialize the background core with platform-specific configuration.
 *
 * @param {Object} config
 * @param {string} config.platform - 'firefox' or 'chrome'
 * @param {string} config.extensionVersion - Extension version string
 * @param {(tabId: number, message: Object) => Promise} config.sendToContentScript
 *   - Send a message to a content script tab
 * @param {(callback: (message, sender) => void) => void} config.onContentScriptMessage
 *   - Register a listener for messages from content scripts
 * @param {(callback: (tabId: number) => void) => void} config.onTabRemoved
 *   - Register a listener for tab removal events
 * @param {(callback: (tabId: number, changeInfo: Object) => void) => void} config.onTabUpdated
 *   - Register a listener for tab update events
 */
// eslint-disable-next-line no-unused-vars
function initBackgroundCore(config) {
  log('Background core starting (platform:', config.platform + ')');

  // --- Message relay from content scripts ---
  config.onContentScriptMessage((message, sender) => {
    // Only process messages from our content script
    if (!message || message.source !== 'wyndham-draft-sync') {
      return;
    }

    const tabId = sender.tab ? sender.tab.id : null;

    // Track content script tabs and connect lazily on first message
    if (tabId !== null) {
      const tabUrl = sender.tab ? sender.tab.url : '';
      if (!isEspnAppUrl(tabUrl)) {
        return;
      }
      const isNew = !knownTabs.has(tabId);
      const wasEmpty = knownTabs.size === 0;
      knownTabs.add(tabId);

      // A newly seen tab becomes primary. This means opening a matchup
      // page while on draft (or vice versa) naturally switches the active
      // source. Subsequent messages from an already-known tab do NOT
      // re-claim primary status, preventing ping-pong between tabs.
      if (isNew) {
        setPrimaryTab(tabId);
      }

      if (wasEmpty) {
        log('First active content script tab detected; connecting');
        connect(config);
      }
    }

    // Only relay messages from the primary tab. Non-primary tabs are
    // tracked but their messages are silently dropped.
    if (tabId !== primaryTabId) {
      log('Dropping', message.type, 'from non-primary tab', tabId,
        '(primary is', primaryTabId + ')');
      return;
    }

    log('Relaying', message.type, 'from tab', tabId);

    // Build a protocol-compliant message with ONLY the fields that
    // protocol.rs ExtensionMessage expects.
    const forwarded = {
      type: message.type,
      timestamp: message.timestamp,
      payload: message.payload,
    };

    // Forward to WebSocket
    if (isConnected) {
      if (!wsSend(forwarded)) {
        warn('WebSocket send failed; message dropped');
      }
    } else {
      warn('WebSocket not connected; dropping message of type:', message.type);
    }
  });

  // --- Tab lifecycle tracking ---
  config.onTabRemoved((tabId) => {
    if (knownTabs.has(tabId)) {
      const hasRemaining = removeTab(tabId);
      log('Tab', tabId, 'closed; active tabs:', knownTabs.size);
      if (!hasRemaining) {
        disconnect();
      }
    }
  });

  config.onTabUpdated((tabId, changeInfo) => {
    if (!knownTabs.has(tabId)) {
      return;
    }
    if (changeInfo.status === 'loading' && changeInfo.url &&
        !isEspnAppUrl(changeInfo.url)) {
      const hasRemaining = removeTab(tabId);
      log('Tab', tabId, 'navigated away from ESPN; active tabs:', knownTabs.size);
      if (!hasRemaining) {
        disconnect();
      }
    }
  });

  log('Background core initialized');
}
