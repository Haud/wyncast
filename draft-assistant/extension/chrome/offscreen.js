// Chrome offscreen document entry point.
// Holds the persistent WebSocket connection (service workers can't).
// Communicates with the service worker via chrome.runtime messaging.

'use strict';

const EXTENSION_VERSION = chrome.runtime.getManifest().version;

// Single message listener that dispatches to the appropriate callback.
// Avoids registering multiple independent onMessage listeners.
let contentScriptCallback = null;
let tabRemovedCallback = null;
let tabUpdatedCallback = null;

chrome.runtime.onMessage.addListener((message) => {
  if (!message || message.target !== 'offscreen') {
    return;
  }

  // Content script message (has payload, no action)
  if (message.payload && !message.action) {
    if (contentScriptCallback) {
      const sender = { tab: message.senderTab || null };
      contentScriptCallback(message.payload, sender);
    }
    return;
  }

  // Tab lifecycle events
  if (message.action === 'tabRemoved' && message.tabId != null) {
    if (tabRemovedCallback) {
      tabRemovedCallback(message.tabId);
    }
    return;
  }

  if (message.action === 'tabUpdated' && message.tabId != null && message.changeInfo) {
    if (tabUpdatedCallback) {
      tabUpdatedCallback(message.tabId, message.changeInfo);
    }
  }
});

initBackgroundCore({
  platform: 'chrome',
  extensionVersion: EXTENSION_VERSION,

  // Offscreen documents can't access chrome.tabs, so ask the service worker
  // to relay messages to content script tabs.
  sendToContentScript: (tabId, message) => {
    return chrome.runtime.sendMessage({
      target: 'service-worker',
      action: 'relayToTab',
      tabId: tabId,
      message: message,
    });
  },

  // Register callback for content script messages forwarded by the service worker.
  onContentScriptMessage: (callback) => {
    contentScriptCallback = callback;
  },

  // Register callback for tab removal events forwarded by the service worker.
  onTabRemoved: (callback) => {
    tabRemovedCallback = callback;
  },

  // Register callback for tab update events forwarded by the service worker.
  onTabUpdated: (callback) => {
    tabUpdatedCallback = callback;
  },
});
