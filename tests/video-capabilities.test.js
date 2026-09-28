import { test, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';

import { videoCodecSupport } from '../frontend/src/lib/video/capabilities.js';

const realDocument = globalThis.document;
// Captured by descriptor, never by value: modern Node defines `navigator` as a
// getter-only global (a strict-mode assignment throws) and reading the
// flag-backed `localStorage` getter warns when no storage file is configured.
const navigatorDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
const localStorageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');

function setNavigator(value) {
  Object.defineProperty(globalThis, 'navigator', {
    value,
    writable: true,
    configurable: true,
    enumerable: true,
  });
}

/** A stand-in element whose canPlayType answers from `answers` (default: "no"). */
function fakeDocument(answers = {}) {
  return { createElement: () => ({ canPlayType: (mime) => answers[mime] ?? '' }) };
}

/** A persisting stand-in for localStorage, the way a real one behaves. */
function fakeStorage() {
  const data = new Map();
  return {
    getItem: (key) => (data.has(key) ? data.get(key) : null),
    setItem: (key, value) => data.set(key, String(value)),
    removeItem: (key) => data.delete(key),
  };
}

beforeEach(() => {
  globalThis.document = fakeDocument();
  globalThis.localStorage = fakeStorage();
  videoCodecSupport.clearCache();
});
afterEach(() => {
  globalThis.document = realDocument;
  if (navigatorDescriptor) Object.defineProperty(globalThis, 'navigator', navigatorDescriptor);
  else delete globalThis.navigator;
  if (localStorageDescriptor)
    Object.defineProperty(globalThis, 'localStorage', localStorageDescriptor);
  else delete globalThis.localStorage;
  videoCodecSupport.clearCache();
});

test('HEVC is declared from the browser answer, never from the user agent', () => {
  setNavigator({ userAgent: 'Mozilla/5.0 Firefox/141.0' });
  globalThis.document = fakeDocument({
    'video/mp4; codecs="hvc1.1.6.L93.B0"': 'maybe',
    'video/mp4; codecs="avc1.42E01E, mp4a.40.2"': 'probably',
  });
  const declared = videoCodecSupport.getClientCodecsString().split(',');
  assert.ok(
    declared.includes('hevc'),
    `a Firefox answering "maybe" must declare HEVC: ${declared}`
  );
  assert.ok(declared.includes('h264-8'));
});

test('a playback-proven codec is declared even when the probe says no', () => {
  globalThis.document = fakeDocument({});
  assert.equal(videoCodecSupport.getClientCodecsString(), '', 'nothing is guessed');
  videoCodecSupport.recordVerifiedCodec('hevc');
  assert.equal(videoCodecSupport.getClientCodecsString(), 'hevc');
  videoCodecSupport.clearCache();
  assert.equal(videoCodecSupport.getClientCodecsString(), 'hevc', 'storage backs the claim');
});

test('audio capability answers from the same probes the declaration uses', () => {
  globalThis.document = fakeDocument({
    'audio/mp4; codecs="mp4a.40.2"': 'probably',
    'audio/mp4; codecs="ac-3"': '',
  });
  assert.equal(videoCodecSupport.canPlayAudioCodec('aac'), true);
  assert.equal(videoCodecSupport.canPlayAudioCodec('ac3'), false);
  assert.equal(videoCodecSupport.canPlayAudioCodec(null), true);
  assert.equal(videoCodecSupport.canPlayAudioCodec(''), true);
  assert.equal(videoCodecSupport.canPlayAudioCodec('truehd'), false);
});
