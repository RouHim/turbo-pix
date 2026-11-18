import assert from 'node:assert/strict';
import test from 'node:test';

import {
  anchorOffset,
  bottomTrim,
  decadeStarts,
  earlierPage,
  monthCount,
  monthOfPhoto,
  monthsWithPhotos,
  sliceDensity,
  snapMonth,
  snapMonthInDirection,
} from '../frontend/src/lib/dateScrubber.js';
import { buildTimelineModel, toMonthIndex } from '../frontend/src/lib/timeline.js';

const may1998 = toMonthIndex(1998, 5);
const apr1998 = toMonthIndex(1998, 4);
const mar1998 = toMonthIndex(1998, 3);
const jan1999 = toMonthIndex(1999, 1);

const density = [
  { year: 1998, month: 3, count: 2 },
  { year: 1998, month: 4, count: 3 },
  { year: 1998, month: 5, count: 1 },
  { year: 1999, month: 1, count: 4 },
];
const model = buildTimelineModel(density);

test('monthOfPhoto parses ISO dates and rejects absent or malformed values', () => {
  assert.equal(monthOfPhoto({ taken_at: '1998-03-15T12:00:00Z' }), mar1998);
  assert.equal(monthOfPhoto({ taken_at: '1998-03-15T12:00:00+02:00' }), mar1998);
  assert.equal(monthOfPhoto({ taken_at: '1998-13-01T00:00:00Z' }), null);
  assert.equal(monthOfPhoto({ taken_at: 'not a date' }), null);
  assert.equal(monthOfPhoto({ taken_at: null }), null);
  assert.equal(monthOfPhoto(null), null);
});

test('monthCount reads the dense model and tolerates out-of-range months', () => {
  assert.equal(monthCount(model, apr1998), 3);
  assert.equal(monthCount(model, toMonthIndex(1998, 4)), 3);
  assert.equal(monthCount(model, null), 0);
  assert.equal(monthCount(buildTimelineModel([]), apr1998), 0);
});

test('monthsWithPhotos orders populated months by the track direction', () => {
  assert.deepEqual(monthsWithPhotos(model, true), [jan1999, may1998, apr1998, mar1998]);
  assert.deepEqual(monthsWithPhotos(model, false), [mar1998, apr1998, may1998, jan1999]);
});

test('sliceDensity returns the library density without a filter', () => {
  assert.equal(sliceDensity(density, { year: null }), density);
});

test('sliceDensity scopes to a year, a month or a range', () => {
  assert.deepEqual(
    sliceDensity(density, { year: 1998, month: null, to_year: null, to_month: null }),
    density.slice(0, 3)
  );
  assert.deepEqual(sliceDensity(density, { year: 1998, month: 4, to_year: null, to_month: null }), [
    density[1],
  ]);
  assert.deepEqual(sliceDensity(density, { year: 1998, month: 4, to_year: 1999, to_month: 1 }), [
    density[1],
    density[2],
    density[3],
  ]);
  assert.deepEqual(sliceDensity(density, { year: 1998, month: 4, to_year: 1998, to_month: null }), [
    density[1],
    density[2],
  ]);
  assert.deepEqual(
    sliceDensity(density, { year: 2005, month: null, to_year: null, to_month: null }),
    []
  );
});

test('snapMonth picks the nearest populated month and prefers newer on a tie', () => {
  const sparse = buildTimelineModel([
    { year: 1998, month: 3, count: 2 },
    { year: 1998, month: 6, count: 1 },
  ]);
  assert.equal(snapMonth(sparse, toMonthIndex(1998, 4)), toMonthIndex(1998, 3));
  assert.equal(snapMonth(sparse, toMonthIndex(1998, 5)), toMonthIndex(1998, 6));
  assert.equal(snapMonth(sparse, toMonthIndex(1998, 7)), toMonthIndex(1998, 6));
  assert.equal(snapMonth(sparse, toMonthIndex(1997, 12)), toMonthIndex(1998, 3));

  const tie = buildTimelineModel([
    { year: 1998, month: 3, count: 2 },
    { year: 1998, month: 5, count: 1 },
  ]);
  assert.equal(snapMonth(tie, toMonthIndex(1998, 4)), toMonthIndex(1998, 5));
  assert.equal(snapMonth(buildTimelineModel([]), mar1998), null);
});

test('anchorOffset counts newer photos in descending order', () => {
  assert.equal(anchorOffset(model, may1998, { descending: true }), 4);
  assert.equal(anchorOffset(model, apr1998, { descending: true }), 5);
  assert.equal(anchorOffset(model, mar1998, { descending: true }), 8);
  assert.equal(anchorOffset(model, may1998, { descending: true, totalPhotos: 12 }), 4);
});

test('anchorOffset adds undated photos ahead of the listing in ascending order', () => {
  assert.equal(anchorOffset(model, apr1998, { descending: false }), 2);
  assert.equal(anchorOffset(model, mar1998, { descending: false }), 0);
  assert.equal(anchorOffset(model, apr1998, { descending: false, totalPhotos: 12 }), 4);
});

test('decadeStarts picks the oldest populated month per decade', () => {
  const months = [
    toMonthIndex(2025, 12),
    toMonthIndex(2025, 1),
    toMonthIndex(2019, 7),
    toMonthIndex(2012, 3),
    toMonthIndex(2004, 11),
    toMonthIndex(1998, 3),
  ];
  assert.deepEqual(decadeStarts(months), [
    { decade: 2020, month: toMonthIndex(2025, 1) },
    { decade: 2010, month: toMonthIndex(2012, 3) },
    { decade: 2000, month: toMonthIndex(2004, 11) },
    { decade: 1990, month: toMonthIndex(1998, 3) },
  ]);
  assert.deepEqual(decadeStarts([]), []);
});

test('decadeStarts keeps each decade start for an oldest-first list', () => {
  const months = [
    toMonthIndex(2012, 3),
    toMonthIndex(2014, 8),
    toMonthIndex(2019, 7),
    toMonthIndex(2021, 2),
    toMonthIndex(2025, 1),
  ];
  assert.deepEqual(decadeStarts(months), [
    { decade: 2010, month: toMonthIndex(2012, 3) },
    { decade: 2020, month: toMonthIndex(2021, 2) },
  ]);
  // A single-decade range is labelled at its start, not at its end.
  assert.deepEqual(decadeStarts([toMonthIndex(2012, 3)]), [
    { decade: 2010, month: toMonthIndex(2012, 3) },
  ]);
});

test('snapMonthInDirection follows the direction of travel across gaps', () => {
  const sparse = buildTimelineModel([
    { year: 1998, month: 3, count: 2 },
    { year: 1998, month: 6, count: 1 },
    { year: 1998, month: 9, count: 3 },
  ]);
  assert.equal(snapMonthInDirection(sparse, toMonthIndex(1998, 4), 1), toMonthIndex(1998, 6));
  assert.equal(snapMonthInDirection(sparse, toMonthIndex(1998, 4), -1), toMonthIndex(1998, 3));
  assert.equal(snapMonthInDirection(sparse, toMonthIndex(1998, 7), -1), toMonthIndex(1998, 6));
  assert.equal(snapMonthInDirection(sparse, toMonthIndex(1998, 6), 1), toMonthIndex(1998, 6));
  // A target beyond the track clamps to its populated end, so the caller
  // simply settles on the same month (no-op at the edge).
  assert.equal(snapMonthInDirection(sparse, toMonthIndex(1998, 10), 1), toMonthIndex(1998, 9));
  assert.equal(snapMonthInDirection(buildTimelineModel([]), mar1998, 1), null);
});

test('earlierPage resolves the page and the prefix length', () => {
  assert.equal(earlierPage(0, 50), null);
  assert.deepEqual(earlierPage(5, 50), { page: 1, take: 5 });
  assert.deepEqual(earlierPage(50, 50), { page: 1, take: 50 });
  assert.deepEqual(earlierPage(51, 50), { page: 2, take: 1 });
  assert.deepEqual(earlierPage(120, 50), { page: 3, take: 20 });
});

test('bottomTrim drops a whole trailing page only when something remains', () => {
  assert.deepEqual(bottomTrim(50, 100, 50), { drop: 50, page: 3 });
  assert.deepEqual(bottomTrim(50, 80, 50), { drop: 30, page: 3 });
  assert.deepEqual(bottomTrim(0, 350, 50), { drop: 50, page: 7 });
  // A one-page window must never be emptied.
  assert.equal(bottomTrim(100, 50, 50), null);
  assert.equal(bottomTrim(100, 30, 50), null);
  assert.equal(bottomTrim(0, 0, 50), null);
});
