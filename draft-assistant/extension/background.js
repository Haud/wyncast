// Firefox background page entry point.
// Loads the shared background core with Firefox-specific configuration.

'use strict';

const EXTENSION_VERSION = browser.runtime.getManifest().version;

initBackgroundCore({
  platform: 'firefox',
  extensionVersion: EXTENSION_VERSION,
  sendToContentScript: (tabId, message) => browser.tabs.sendMessage(tabId, message),
  onContentScriptMessage: (callback) => {
    browser.runtime.onMessage.addListener(callback);
  },
  onTabRemoved: (callback) => {
    browser.tabs.onRemoved.addListener(callback);
  },
  onTabUpdated: (callback) => {
    browser.tabs.onUpdated.addListener(callback);
  },
});
