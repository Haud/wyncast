// Chrome MV3 service worker.
// Relays messages between content scripts and the offscreen document that
// holds the persistent WebSocket connection.

'use strict';

const LOG_PREFIX = '[WyndhamDraftSync:SW]';
const OFFSCREEN_URL = 'offscreen.html';

let creatingOffscreen = null;
let offscreenReady = false;

// Track known draft tab IDs so we can gate tab lifecycle messages
// and validate relayToTab targets without an IPC round-trip.
const knownDraftTabs = new Set();

/**
 * Ensure the offscreen document exists. Creates it if needed.
 * Uses a lock to prevent duplicate creation attempts.
 * Caches the result so subsequent calls skip the IPC check.
 */
async function ensureOffscreen() {
  if (offscreenReady) {
    return;
  }

  // Check if offscreen document already exists
  const contexts = await chrome.runtime.getContexts({
    contextTypes: ['OFFSCREEN_DOCUMENT'],
    documentUrls: [chrome.runtime.getURL(OFFSCREEN_URL)],
  });

  if (contexts.length > 0) {
    offscreenReady = true;
    return;
  }

  // Wait for any in-progress creation, then verify it succeeded
  if (creatingOffscreen) {
    await creatingOffscreen;
    // Re-check: the creation we waited on may have failed
    const recheck = await chrome.runtime.getContexts({
      contextTypes: ['OFFSCREEN_DOCUMENT'],
      documentUrls: [chrome.runtime.getURL(OFFSCREEN_URL)],
    });
    if (recheck.length > 0) {
      offscreenReady = true;
    }
    return;
  }

  creatingOffscreen = chrome.offscreen.createDocument({
    url: OFFSCREEN_URL,
    reasons: ['WEBSOCKET'],
    justification: 'Persistent WebSocket connection to draft assistant backend',
  });

  try {
    await creatingOffscreen;
    offscreenReady = true;
    console.log(LOG_PREFIX, 'Offscreen document created');
  } catch (err) {
    console.error(LOG_PREFIX, 'Failed to create offscreen document:', err.message || err);
  } finally {
    creatingOffscreen = null;
  }
}

// ---------------------------------------------------------------------------
// Content script message relay → offscreen document
// ---------------------------------------------------------------------------

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  // --- Messages from the offscreen document asking to relay to a tab ---
  if (message && message.target === 'service-worker' && message.action === 'relayToTab') {
    // Validate tabId is a tracked draft tab
    if (typeof message.tabId !== 'number' || !knownDraftTabs.has(message.tabId)) {
      console.warn(LOG_PREFIX, 'relayToTab rejected: tab', message.tabId, 'is not a known draft tab');
      sendResponse({ ok: false });
      return;
    }
    chrome.tabs.sendMessage(message.tabId, message.message).catch((err) => {
      console.log(LOG_PREFIX, 'Could not relay to tab', message.tabId, ':', err.message || err);
    });
    sendResponse({ ok: true });
    return;
  }

  // --- Messages from content scripts → forward to offscreen document ---
  if (!message || message.source !== 'wyndham-draft-sync') {
    return;
  }

  // Track this tab as a known draft tab (service worker mirrors the set
  // maintained by background-core.js in the offscreen document)
  if (sender.tab && sender.tab.id != null) {
    knownDraftTabs.add(sender.tab.id);
  }

  // Ensure offscreen document is running, then forward
  ensureOffscreen().then(() => {
    chrome.runtime.sendMessage({
      target: 'offscreen',
      payload: message,
      senderTab: sender.tab ? { id: sender.tab.id, url: sender.tab.url } : null,
    }).catch((err) => {
      console.warn(LOG_PREFIX, 'Failed to forward to offscreen:', err.message || err);
    });
  }).catch((err) => {
    console.error(LOG_PREFIX, 'ensureOffscreen failed:', err.message || err);
  });

  // Return true to keep the message channel open for the async path
  return true;
});

// ---------------------------------------------------------------------------
// Tab lifecycle tracking → forward to offscreen document
// ---------------------------------------------------------------------------

chrome.tabs.onRemoved.addListener((tabId) => {
  if (!knownDraftTabs.has(tabId)) {
    return;
  }
  knownDraftTabs.delete(tabId);
  chrome.runtime.sendMessage({
    target: 'offscreen',
    action: 'tabRemoved',
    tabId: tabId,
  }).catch(() => {
    // Offscreen document may not exist yet — safe to ignore
  });
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (!knownDraftTabs.has(tabId)) {
    return;
  }
  if (changeInfo.status === 'loading' && changeInfo.url) {
    chrome.runtime.sendMessage({
      target: 'offscreen',
      action: 'tabUpdated',
      tabId: tabId,
      changeInfo: { status: changeInfo.status, url: changeInfo.url },
    }).catch(() => {
      // Offscreen document may not exist yet — safe to ignore
    });
    // If navigated away from draft, remove from known set
    try {
      const parsed = new URL(changeInfo.url);
      if (parsed.hostname !== 'fantasy.espn.com' ||
          !parsed.pathname.startsWith('/baseball/draft')) {
        knownDraftTabs.delete(tabId);
      }
    } catch (e) {
      // Invalid URL — remove from known set to be safe
      knownDraftTabs.delete(tabId);
    }
  }
});

console.log(LOG_PREFIX, 'Service worker started');
