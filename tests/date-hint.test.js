import assert from 'node:assert/strict';
import test from 'node:test';

import { firstVisibleCardIndex } from '../frontend/src/lib/dateScrubber.js';
import {
  formatHintDate,
  isDateSort,
  isPhotoGridRoute,
  nextHintVisibility,
} from '../frontend/src/lib/dateHint.js';

const card = (bottom, top = 0) => ({ getBoundingClientRect: () => ({ top, bottom }) });

test('nextHintVisibility enters at the header line and leaves after the hysteresis band', () => {
  assert.equal(nextHintVisibility(64, false, 64, 8), true);
  assert.equal(nextHintVisibility(64.1, false, 64, 8), false);
  assert.equal(nextHintVisibility(72, true, 64, 8), true);
  assert.equal(nextHintVisibility(72.1, true, 64, 8), false);
  assert.equal(nextHintVisibility(150, true, 64, 8), false);
  assert.equal(nextHintVisibility(0, true, 64, 8), true);
});

test('isDateSort accepts only the chronological taken_at sorts', () => {
  assert.equal(isDateSort('date_desc'), true);
  assert.equal(isDateSort('date_asc'), true);
  assert.equal(isDateSort('name_asc'), false);
  assert.equal(isDateSort('size_desc'), false);
  assert.equal(isDateSort(null), false);
});

test('isPhotoGridRoute matches the routes that render PhotoGrid', () => {
  for (const view of ['all', 'favorites', 'videos']) {
    assert.equal(isPhotoGridRoute({ view }), true);
  }
  for (const view of ['collages', 'housekeeping', 'map']) {
    assert.equal(isPhotoGridRoute({ view }), false);
  }
  assert.equal(isPhotoGridRoute({ view: 'collages', album: 5 }), false);
  assert.equal(isPhotoGridRoute({ view: 'albums' }), false);
  assert.equal(isPhotoGridRoute({ view: 'albums', album: 5 }), true);
  assert.equal(isPhotoGridRoute({ view: 'albums', query: 'legacy' }), true);
  assert.equal(isPhotoGridRoute({ view: 'people' }), false);
  assert.equal(isPhotoGridRoute({ view: 'people', person: 3 }), true);
  assert.equal(isPhotoGridRoute({ view: 'people', query: 'legacy' }), true);
});

test('formatHintDate renders the authored date and rejects missing values', () => {
  assert.equal(formatHintDate('2024-11-18T12:00:00', 'de'), '18. Nov. 2024');
  assert.equal(formatHintDate('2024-11-18T12:00:00', 'en'), 'Nov 18, 2024');
  assert.equal(formatHintDate(null, 'de'), null);
  assert.equal(formatHintDate('not a date', 'de'), null);
});

test('firstVisibleCardIndex ignores cards below the viewport when bounded', () => {
  const cards = [card(70, 10), card(900, 800), card(300, 200)];
  assert.equal(firstVisibleCardIndex(cards, 64, 700), 0);
  assert.equal(firstVisibleCardIndex([card(900, 800), card(1000, 900)], 64, 700), null);
});

test('firstVisibleCardIndex finds the first card crossing the header line', () => {
  const cards = [card(10), card(64), card(66), card(200)];
  assert.equal(firstVisibleCardIndex(cards, 64), 2);
  assert.equal(firstVisibleCardIndex([card(10), card(64), card(65)], 64), null);
  assert.equal(firstVisibleCardIndex([], 64), null);
  assert.equal(firstVisibleCardIndex([card(10), card(20)], 64), null);
});
