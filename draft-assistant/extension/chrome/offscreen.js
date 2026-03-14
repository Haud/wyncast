// Chrome offscreen document entry point.
// Holds the persistent WebSocket connection (service workers can't).
// Communicates with the service worker via chrome.runtime messaging.

'use strict';

const EXTENSION_VERSION = chrome.runtime.getManifest().version;

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

  // Listen for messages forwarded from the service worker.
  // The service worker tags forwarded content script messages with
  // target: 'offscreen'.
  onContentScriptMessage: (callback) => {
    chrome.runtime.onMessage.addListener((message, sender) => {
      if (message && message.target === 'offscreen' && message.payload) {
        // Reconstruct a sender-like object with the original tab info
        const fakeSender = { tab: message.senderTab || null };
        callback(message.payload, fakeSender);
      }
    });
  },

  // Tab lifecycle events are forwarded from the service worker as
  // target: 'offscreen' messages with action: 'tabRemoved' / 'tabUpdated'.
  onTabRemoved: (callback) => {
    chrome.runtime.onMessage.addListener((message) => {
      if (message && message.target === 'offscreen' && message.action === 'tabRemoved') {
        callback(message.tabId);
      }
    });
  },

  onTabUpdated: (callback) => {
    chrome.runtime.onMessage.addListener((message) => {
      if (message && message.target === 'offscreen' && message.action === 'tabUpdated') {
        callback(message.tabId, message.changeInfo);
      }
    });
  },
});
