import { test, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';

import {
  VERIFIED_CODECS_STORAGE_KEY,
  codecTokenFor,
  markCodecVerified,
  verifiedCodecs,
} from '../frontend/src/lib/video/playbackVerification.js';

// Captured by descriptor, never read: Node's localStorage getter warns when the
// flag that backs it is absent, and the suite's output must stay clean.
const localStorageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');

function fakeStorage(initial = {}) {
  const data = new Map(Object.entries(initial));
  return {
    getItem: (key) => (data.has(key) ? data.get(key) : null),
    setItem: (key, value) => data.set(key, String(value)),
    removeItem: (key) => data.delete(key),
  };
}

beforeEach(() => {
  globalThis.localStorage = fakeStorage();
});
afterEach(() => {
  if (localStorageDescriptor) {
    Object.defineProperty(globalThis, 'localStorage', localStorageDescriptor);
  } else {
    delete globalThis.localStorage;
  }
});

test('a codec and bit depth map to the token the declaration uses', () => {
  assert.equal(codecTokenFor('h264', 8), 'h264-8');
  assert.equal(codecTokenFor('h264', 10), 'h264-10');
  assert.equal(codecTokenFor('h264', null), 'h264-8');
  assert.equal(codecTokenFor('hevc', 10), 'hevc');
  assert.equal(codecTokenFor('av1', 8), 'av1');
  assert.equal(codecTokenFor('vp9', null), 'vp9');
  assert.equal(codecTokenFor('vp8', null), 'vp8');
  assert.equal(codecTokenFor('mpeg4', 8), null);
  assert.equal(codecTokenFor('', 8), null);
  assert.equal(codecTokenFor(null, 8), null);
  assert.equal(codecTokenFor(undefined, undefined), null);
});

test('a verification is written once and read back from storage', () => {
  assert.deepEqual(verifiedCodecs(), []);
  assert.deepEqual(markCodecVerified('hevc'), ['hevc']);
  assert.deepEqual(markCodecVerified('hevc'), ['hevc'], 'duplicates are not stored');
  assert.deepEqual(markCodecVerified('h264-10'), ['hevc', 'h264-10']);
  // What an earlier page load wrote is what the next one reads.
  assert.deepEqual(JSON.parse(globalThis.localStorage.getItem(VERIFIED_CODECS_STORAGE_KEY)), [
    'hevc',
    'h264-10',
  ]);
  assert.deepEqual(verifiedCodecs(), ['hevc', 'h264-10']);
});

test('tokens the declaration cannot express are refused', () => {
  assert.deepEqual(markCodecVerified(null), []);
  assert.deepEqual(markCodecVerified('mpeg4'), []);
  assert.deepEqual(markCodecVerified(''), []);
  assert.deepEqual(verifiedCodecs(), []);
});

test('malformed or unreadable storage reads as empty and never throws', () => {
  globalThis.localStorage = fakeStorage({ [VERIFIED_CODECS_STORAGE_KEY]: 'not json' });
  assert.deepEqual(verifiedCodecs(), []);
  globalThis.localStorage = fakeStorage({ [VERIFIED_CODECS_STORAGE_KEY]: '{"hevc":true}' });
  assert.deepEqual(verifiedCodecs(), []);

  globalThis.localStorage = {
    getItem: () => {
      throw new Error('denied');
    },
    setItem: () => {
      throw new Error('denied');
    },
  };
  assert.deepEqual(verifiedCodecs(), []);
  assert.deepEqual(markCodecVerified('hevc'), ['hevc'], 'the answer survives a denied write');
});

test('a document that denies the storage PROPERTY reads as empty and never throws', () => {
  // `typeof` does not suppress an exception thrown by a property getter: in a
  // document whose storage access is denied, reading `window.localStorage`
  // throws a `SecurityError`, so the property read itself has to sit inside
  // the guard. The methods of a readable store throwing is the case above.
  Object.defineProperty(globalThis, 'localStorage', {
    get() {
      throw new Error('SecurityError: access to this document is denied');
    },
    configurable: true,
  });
  assert.deepEqual(verifiedCodecs(), []);
  assert.deepEqual(markCodecVerified('hevc'), ['hevc'], 'the answer survives the property throw');
});
