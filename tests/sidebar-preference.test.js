import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  SIDEBAR_COLLAPSED_STORAGE_KEY,
  readSidebarCollapsed,
  writeSidebarCollapsed,
} from '../frontend/src/lib/sidebarPreference.js';

function fakeStorage(initial = {}) {
  const data = new Map(Object.entries(initial));
  return {
    getItem: (key) => (data.has(key) ? data.get(key) : null),
    setItem: (key, value) => data.set(key, value),
  };
}

test('an absent preference reads as expanded', () => {
  assert.equal(readSidebarCollapsed(fakeStorage()), false);
});

test('every value but the written marker reads as expanded', () => {
  for (const value of ['false', '', '1', '{"collapsed":true}', 'TRUE']) {
    assert.equal(
      readSidebarCollapsed(fakeStorage({ [SIDEBAR_COLLAPSED_STORAGE_KEY]: value })),
      false,
      `value ${JSON.stringify(value)} must not count as collapsed`
    );
  }
  assert.equal(
    readSidebarCollapsed(fakeStorage({ [SIDEBAR_COLLAPSED_STORAGE_KEY]: 'true' })),
    true
  );
});

test('writing records the marker and round-trips through the reader', () => {
  const storage = fakeStorage();
  writeSidebarCollapsed(storage, true);
  assert.equal(storage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY), 'true');
  assert.equal(readSidebarCollapsed(storage), true);

  writeSidebarCollapsed(storage, false);
  assert.equal(storage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY), 'false');
  assert.equal(readSidebarCollapsed(storage), false);
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
  assert.equal(readSidebarCollapsed(denied), false);
  assert.doesNotThrow(() => writeSidebarCollapsed(denied, true));
  assert.equal(readSidebarCollapsed(null), false);
  assert.doesNotThrow(() => writeSidebarCollapsed(null, true));
});
