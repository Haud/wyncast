// Tiny browser API polyfill.
// Chrome MV3 chrome.* APIs already return Promises, so we just alias the namespace.
'use strict';

if (typeof globalThis.browser === 'undefined') {
  globalThis.browser = chrome;
}
