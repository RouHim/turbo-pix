import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  MONTHS_PER_YEAR,
  MONTHS_PER_DECADE,
  buildTimelineModel,
  clampIndexToModel,
  clampSelectionToModel,
  countInRange,
  formatPeriodName,
  formatSelectionLabel,
  fromMonthIndex,
  normalizeSelection,
  selectionEquals,
  toMonthIndex,
} from '../frontend/src/lib/timeline.js';

const format = {
  allDates: 'All Dates',
  decadeLabel: (year) => `${year}s`,
  rangeTemplate: (start, end) => `${start} – ${end}`,
  monthName: (month) =>
    [
      'January',
      'February',
      'March',
      'April',
      'May',
      'June',
      'July',
      'August',
      'September',
      'October',
      'November',
      'December',
    ][month - 1],
};

test('month index round-trips', () => {
  assert.equal(toMonthIndex(1998, 3), 1998 * 12 + 2);
  assert.deepEqual(fromMonthIndex(toMonthIndex(1998, 3)), { year: 1998, month: 3 });
  assert.equal(MONTHS_PER_YEAR, 12);
  assert.equal(MONTHS_PER_DECADE, 120);
});

test('model spans the populated range and zero-fills the gaps', () => {
  const model = buildTimelineModel([
    { year: 2012, month: 3, count: 4 },
    { year: 2015, month: 8, count: 2 },
    { year: 2015, month: 8, count: 1 },
  ]);
  assert.equal(model.minIndex, toMonthIndex(2012, 3));
  assert.equal(model.maxIndex, toMonthIndex(2015, 8));
  assert.equal(model.length, model.maxIndex - model.minIndex + 1);
  assert.equal(model.total, 7);
  assert.equal(countInRange(model, toMonthIndex(2012, 3), toMonthIndex(2012, 3)), 4);
  assert.equal(countInRange(model, model.minIndex, model.maxIndex), 7);
  assert.equal(countInRange(model, toMonthIndex(2013, 1), toMonthIndex(2015, 7)), 0);
  assert.deepEqual(model.years, [2015, 2012]);
});

test('empty density yields an empty model', () => {
  const model = buildTimelineModel([]);
  assert.equal(model.total, 0);
  assert.deepEqual(model.years, []);
  assert.equal(countInRange(model, 0, 100), 0);
});

test('normalizeSelection orders bounds and preserves null', () => {
  assert.deepEqual(normalizeSelection(5, 2), { startIndex: 2, endIndex: 5 });
  assert.deepEqual(normalizeSelection(2, 5), { startIndex: 2, endIndex: 5 });
  assert.equal(normalizeSelection(null, 5), null);
  assert.ok(selectionEquals({ startIndex: 2, endIndex: 5 }, { startIndex: 2, endIndex: 5 }));
  assert.ok(!selectionEquals({ startIndex: 2, endIndex: 5 }, { startIndex: 2, endIndex: 6 }));
  assert.ok(selectionEquals(null, null));
});

test('clampSelectionToModel narrows to the overlap and clears without one', () => {
  const model = buildTimelineModel([
    { year: 2012, month: 3, count: 1 },
    { year: 2015, month: 8, count: 1 },
  ]);
  assert.deepEqual(
    clampSelectionToModel(
      { startIndex: toMonthIndex(1900, 1), endIndex: toMonthIndex(2013, 5) },
      model
    ),
    { startIndex: model.minIndex, endIndex: toMonthIndex(2013, 5) }
  );
  assert.equal(
    clampSelectionToModel(
      { startIndex: toMonthIndex(1900, 1), endIndex: toMonthIndex(1901, 5) },
      model
    ),
    null
  );
  assert.equal(clampSelectionToModel(null, model), null);
  assert.equal(clampIndexToModel(model, toMonthIndex(1900, 1)), model.minIndex);
  assert.equal(clampIndexToModel(model, toMonthIndex(2100, 1)), model.maxIndex);
});

test('selection labels name the period or both bounds', () => {
  const march2012 = { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2012, 3) };
  assert.equal(formatSelectionLabel(null, format), 'All Dates');
  assert.equal(formatSelectionLabel(march2012, format), 'March 2012');

  const year2012 = { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) };
  assert.equal(formatSelectionLabel(year2012, format), '2012');

  const years = { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2015, 12) };
  assert.equal(formatSelectionLabel(years, format), '2012 – 2015');

  const range = { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2015, 8) };
  assert.equal(formatSelectionLabel(range, format), 'March 2012 – August 2015');

  const partial = { startIndex: toMonthIndex(2011, 12), endIndex: toMonthIndex(2012, 3) };
  assert.equal(formatSelectionLabel(partial, format), 'December 2011 – March 2012');
});

test('period labels name the month and year', () => {
  assert.equal(formatPeriodName(toMonthIndex(1998, 3), format.monthName), 'March 1998');
  assert.equal(formatPeriodName(toMonthIndex(2012, 12), format.monthName), 'December 2012');
});

test('a grid-aligned decade is labelled as the decade', () => {
  const sixties = { startIndex: toMonthIndex(1960, 1), endIndex: toMonthIndex(1969, 12) };
  assert.equal(formatSelectionLabel(sixties, format), '1960s');

  // Clipped at the library's first bucket (March 1962) it is no longer a
  // decade-shaped selection and keeps the explicit range wording.
  const clipped = { startIndex: toMonthIndex(1962, 3), endIndex: toMonthIndex(1969, 12) };
  assert.equal(formatSelectionLabel(clipped, format), 'March 1962 – December 1969');

  // A ten-year span that is not grid-aligned is an ordinary range.
  const shifted = { startIndex: toMonthIndex(1963, 1), endIndex: toMonthIndex(1972, 12) };
  assert.equal(formatSelectionLabel(shifted, format), '1963 – 1972');

  // Single periods and whole years are unaffected.
  assert.equal(
    formatSelectionLabel(
      { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) },
      format
    ),
    '2012'
  );
  assert.equal(
    formatSelectionLabel(
      { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2012, 3) },
      format
    ),
    'March 2012'
  );
});
