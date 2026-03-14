// Chrome MV3 service worker.
// Relays messages between content scripts and the offscreen document that
// holds the persistent WebSocket connection.

'use strict';

const LOG_PREFIX = '[WyndhamDraftSync:SW]';
const OFFSCREEN_URL = 'offscreen.html';

let creatingOffscreen = null;

/**
 * Ensure the offscreen document exists. Creates it if needed.
 * Uses a lock to prevent duplicate creation attempts.
 */
async function ensureOffscreen() {
  // Check if offscreen document already exists
  const contexts = await chrome.runtime.getContexts({
    contextTypes: ['OFFSCREEN_DOCUMENT'],
    documentUrls: [chrome.runtime.getURL(OFFSCREEN_URL)],
  });

  if (contexts.length > 0) {
    return;
  }

  // Wait for any in-progress creation
  if (creatingOffscreen) {
    await creatingOffscreen;
    return;
  }

  creatingOffscreen = chrome.offscreen.createDocument({
    url: OFFSCREEN_URL,
    reasons: ['WEB_RTC'],
    justification: 'Persistent WebSocket connection to draft assistant backend',
  });

  await creatingOffscreen;
  creatingOffscreen = null;
  console.log(LOG_PREFIX, 'Offscreen document created');
}

// ---------------------------------------------------------------------------
// Content script message relay → offscreen document
// ---------------------------------------------------------------------------

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  // --- Messages from the offscreen document asking to relay to a tab ---
  if (message && message.target === 'service-worker' && message.action === 'relayToTab') {
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

  // Ensure offscreen document is running, then forward
  ensureOffscreen().then(() => {
    chrome.runtime.sendMessage({
      target: 'offscreen',
      payload: message,
      senderTab: sender.tab ? { id: sender.tab.id, url: sender.tab.url } : null,
    }).catch((err) => {
      console.warn(LOG_PREFIX, 'Failed to forward to offscreen:', err.message || err);
    });
  });
});

// ---------------------------------------------------------------------------
// Tab lifecycle tracking → forward to offscreen document
// ---------------------------------------------------------------------------

chrome.tabs.onRemoved.addListener((tabId) => {
  chrome.runtime.sendMessage({
    target: 'offscreen',
    action: 'tabRemoved',
    tabId: tabId,
  }).catch(() => {
    // Offscreen document may not exist yet — safe to ignore
  });
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.status === 'loading' && changeInfo.url) {
    chrome.runtime.sendMessage({
      target: 'offscreen',
      action: 'tabUpdated',
      tabId: tabId,
      changeInfo: { status: changeInfo.status, url: changeInfo.url },
    }).catch(() => {
      // Offscreen document may not exist yet — safe to ignore
    });
  }
});

console.log(LOG_PREFIX, 'Service worker started');
