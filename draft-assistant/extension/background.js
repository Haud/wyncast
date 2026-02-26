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
    // The backend might send messages to the extension in the future.
    // For now, just log them.
    log('Received from backend:', event.data);
  };
}

// ---------------------------------------------------------------------------
// Message relay from content scripts
// ---------------------------------------------------------------------------

/**
 * Handle messages from content scripts.
 * Constructs a protocol-compliant message and forwards to the backend.
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

  // Log tab context for debugging (not forwarded to backend)
  const tabId = sender.tab ? sender.tab.id : null;
  log('Relaying', message.type, 'from tab', tabId);

  // Build a protocol-compliant message with ONLY the fields that
  // protocol.rs ExtensionMessage expects. Do not add extra fields like
  // tabId or relayTimestamp -- serde will reject unknown fields.
  const forwarded = {
    type: message.type,
    timestamp: message.timestamp,
    payload: message.payload,
  };

  // Forward to WebSocket
  if (isConnected) {
    if (wsSend(forwarded)) {
      // Message sent successfully
    } else {
      warn('WebSocket send failed; message dropped');
    }
  } else {
    warn('WebSocket not connected; dropping message of type:', message.type);
  }
});

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

log('Background script starting');
connect();
