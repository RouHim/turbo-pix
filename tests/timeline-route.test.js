import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  EMPTY_DATE_FILTER,
  filterEquals,
  filterFromSelection,
  normalizeDateFilter,
  selectionFromFilter,
} from '../frontend/src/lib/timelineRoute.js';
import { buildTimelineModel, toMonthIndex } from '../frontend/src/lib/timeline.js';

const model = buildTimelineModel([
  { year: 2012, month: 3, count: 1 },
  { year: 2012, month: 8, count: 1 },
  { year: 2015, month: 8, count: 1 },
  { year: 2026, month: 1, count: 1 },
]);

test('a cleared filter stays cleared and drops orphan months', () => {
  assert.deepEqual(
    normalizeDateFilter({ year: null, month: 5, to_year: 2012, to_month: 3 }),
    EMPTY_DATE_FILTER
  );
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 13, to_year: null, to_month: null }), {
    year: 2012,
    month: null,
    to_year: null,
    to_month: null,
  });
});

test('single periods survive canonicalisation unchanged', () => {
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 3 }), {
    year: 2012,
    month: 3,
    to_year: null,
    to_month: null,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: null }), {
    year: 2012,
    month: null,
    to_year: null,
    to_month: null,
  });
});

test('ranges canonicalise to whole-year bounds and never carry a redundant end', () => {
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 1, to_year: 2012, to_month: 12 }), {
    year: 2012,
    month: null,
    to_year: null,
    to_month: null,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 3, to_year: 2012, to_month: 8 }), {
    year: 2012,
    month: 3,
    to_year: 2012,
    to_month: 8,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 1, to_year: 2015, to_month: 12 }), {
    year: 2012,
    month: null,
    to_year: 2015,
    to_month: null,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 3, to_year: 2015, to_month: 12 }), {
    year: 2012,
    month: 3,
    to_year: 2015,
    to_month: null,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 3, to_year: 2015, to_month: null }), {
    year: 2012,
    month: 3,
    to_year: 2015,
    to_month: null,
  });
});

test('reversed restored bounds normalise to ascending order', () => {
  assert.deepEqual(normalizeDateFilter({ year: 2015, month: 8, to_year: 2012, to_month: 3 }), {
    year: 2012,
    month: 3,
    to_year: 2015,
    to_month: 8,
  });
  assert.deepEqual(
    normalizeDateFilter({ year: 2015, month: null, to_year: 2012, to_month: null }),
    {
      year: 2012,
      month: null,
      to_year: 2015,
      to_month: null,
    }
  );
});

test('filter to selection clamps to the library and clears without overlap', () => {
  const range = normalizeDateFilter({ year: 2012, month: 3, to_year: 2015, to_month: 8 });
  assert.deepEqual(selectionFromFilter(range, model), {
    startIndex: toMonthIndex(2012, 3),
    endIndex: toMonthIndex(2015, 8),
  });

  const year = normalizeDateFilter({ year: 2012, month: null });
  assert.deepEqual(selectionFromFilter(year, model), {
    startIndex: toMonthIndex(2012, 1),
    endIndex: toMonthIndex(2012, 12),
  });

  const lost = normalizeDateFilter({ year: 1900, month: 4, to_year: 1901, to_month: 6 });
  assert.equal(selectionFromFilter(lost, model), null, 'no overlap clears the filter');

  const partial = normalizeDateFilter({ year: 1900, month: 4, to_year: 2012, to_month: 8 });
  assert.deepEqual(selectionFromFilter(partial, model), {
    startIndex: model.minIndex,
    endIndex: toMonthIndex(2012, 8),
  });

  assert.equal(selectionFromFilter(EMPTY_DATE_FILTER, model), null);
  assert.equal(selectionFromFilter(null, model), null);
});

test('selection to filter is canonical and round-trips', () => {
  const selection = { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2015, 8) };
  const filter = filterFromSelection(selection);
  assert.deepEqual(filter, { year: 2012, month: 3, to_year: 2015, to_month: 8 });
  assert.deepEqual(selectionFromFilter(filter, model), selection);

  assert.deepEqual(filterFromSelection(null), EMPTY_DATE_FILTER);
  assert.deepEqual(
    filterFromSelection({ startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2012, 3) }),
    {
      year: 2012,
      month: 3,
      to_year: null,
      to_month: null,
    }
  );
  assert.deepEqual(
    filterFromSelection({ startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) }),
    {
      year: 2012,
      month: null,
      to_year: null,
      to_month: null,
    }
  );
  assert.deepEqual(
    filterFromSelection({ startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2015, 12) }),
    {
      year: 2012,
      month: null,
      to_year: 2015,
      to_month: null,
    }
  );
});

test('filterEquality is structural', () => {
  assert.ok(
    filterEquals(EMPTY_DATE_FILTER, { year: null, month: null, to_year: null, to_month: null })
  );
  assert.ok(
    !filterEquals(EMPTY_DATE_FILTER, { year: 2012, month: null, to_year: null, to_month: null })
  );
  assert.ok(
    !filterEquals(
      { year: 2012, month: 3, to_year: 2015, to_month: 8 },
      { year: 2012, month: 3, to_year: 2015, to_month: 9 }
    )
  );
});
