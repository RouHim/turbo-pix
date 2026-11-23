import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  TIMELINE_COLLAPSED_STORAGE_KEY,
  readTimelineCollapsed,
  writeTimelineCollapsed,
} from '../frontend/src/lib/timelinePreference.js';

function fakeStorage(initial = {}) {
  const data = new Map(Object.entries(initial));
  return {
    getItem: (key) => (data.has(key) ? data.get(key) : null),
    setItem: (key, value) => data.set(key, value),
  };
}

test('an absent timeline preference reads as expanded', () => {
  assert.equal(readTimelineCollapsed(fakeStorage()), false);
});

test('every value but the written marker reads as expanded', () => {
  for (const value of ['false', '', '1', '{"collapsed":true}', 'TRUE']) {
    assert.equal(
      readTimelineCollapsed(fakeStorage({ [TIMELINE_COLLAPSED_STORAGE_KEY]: value })),
      false,
      `value ${JSON.stringify(value)} must not count as collapsed`
    );
  }
  assert.equal(
    readTimelineCollapsed(fakeStorage({ [TIMELINE_COLLAPSED_STORAGE_KEY]: 'true' })),
    true
  );
});

test('writing records the marker and round-trips through the reader', () => {
  const storage = fakeStorage();
  writeTimelineCollapsed(storage, true);
  assert.equal(storage.getItem(TIMELINE_COLLAPSED_STORAGE_KEY), 'true');
  assert.equal(readTimelineCollapsed(storage), true);

  writeTimelineCollapsed(storage, false);
  assert.equal(storage.getItem(TIMELINE_COLLAPSED_STORAGE_KEY), 'false');
  assert.equal(readTimelineCollapsed(storage), false);
});

test('a denied or missing storage never throws', () => {
  const denied = {
    getItem() {
      throw new Error('denied');
    },
    setItem() {
      throw new Error('denied');
    },
  };
  assert.equal(readTimelineCollapsed(denied), false);
  assert.doesNotThrow(() => writeTimelineCollapsed(denied, true));
  assert.equal(readTimelineCollapsed(null), false);
  assert.doesNotThrow(() => writeTimelineCollapsed(null, true));
});
