import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  EMPTY_DATE_FILTER,
  filterEquals,
  filterFromSelection,
  isDecadeFilter,
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

test('a bare period keeps its end bound even past the model, so the newest year round-trips', () => {
  // The model's newest month is January 2026, so a whole-year period reaches
  // past it. Clamping the end to `model.maxIndex` gave
  // `filterFromSelection` a clipped selection back — `{year: 2026, month: 1}` —
  // which the clamp effect then wrote to the URL: the label read
  // "January 2026 – September 2026", the selection was no longer a single
  // period, and a saved `year = 2026` stopped matching.
  const filter = normalizeDateFilter({ year: 2026, month: null });
  assert.deepEqual(filter, { year: 2026, month: null, to_year: null, to_month: null });
  assert.ok(toMonthIndex(2026, 12) > model.maxIndex, 'the period ends past the model');

  const selection = selectionFromFilter(filter, model);
  assert.deepEqual(selection, {
    startIndex: toMonthIndex(2026, 1),
    endIndex: toMonthIndex(2026, 12),
  });
  assert.deepEqual(filterFromSelection(selection), filter, 'the round-trip is the identity');

  // A period the library has nothing in still clears the filter
  assert.equal(selectionFromFilter(normalizeDateFilter({ year: 2030, month: null }), model), null);
});

test('selectionFromFilter answers null for missing input', () => {
  // The density may not have loaded yet and a filter may arrive without a year;
  // neither may throw or invent a selection.
  assert.equal(selectionFromFilter(undefined, model), null);
  assert.equal(selectionFromFilter(null, model), null);
  const filter = normalizeDateFilter({ year: 2012, month: 3 });
  assert.equal(selectionFromFilter(filter, undefined), null);
  assert.equal(selectionFromFilter(filter, buildTimelineModel([])), null);
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

test('a grid-aligned decade is a period and keeps its own bounds', () => {
  // March 1962 … September 1974: the 1960s reach left of the library.
  const legacyModel = buildTimelineModel([
    { year: 1962, month: 3, count: 1 },
    { year: 1969, month: 12, count: 1 },
    { year: 1974, month: 9, count: 1 },
  ]);

  const decade = normalizeDateFilter({ year: 1960, month: null, to_year: 1969, to_month: null });
  assert.ok(isDecadeFilter(decade));
  assert.deepEqual(selectionFromFilter(decade, legacyModel), {
    startIndex: toMonthIndex(1960, 1),
    endIndex: toMonthIndex(1969, 12),
  });
  assert.deepEqual(
    filterFromSelection(selectionFromFilter(decade, legacyModel)),
    decade,
    'the round trip is the identity, so the clamp effect leaves the URL alone'
  );

  // Only a *grid-aligned* ten-year range is a period; a shifted one keeps
  // clamping to the library.
  const shifted = normalizeDateFilter({ year: 1963, month: null, to_year: 1972, to_month: null });
  assert.ok(!isDecadeFilter(shifted));
  assert.deepEqual(selectionFromFilter(shifted, legacyModel), {
    startIndex: toMonthIndex(1963, 1),
    endIndex: toMonthIndex(1972, 12),
  });

  // …and a range reaching left of the library still clamps to its oldest bucket.
  const early = normalizeDateFilter({ year: 1955, month: 3, to_year: 1972, to_month: 12 });
  assert.ok(!isDecadeFilter(early));
  assert.deepEqual(selectionFromFilter(early, legacyModel), {
    startIndex: legacyModel.minIndex,
    endIndex: toMonthIndex(1972, 12),
  });

  // A decade the library has nothing in still clears the filter.
  const nineties = normalizeDateFilter({ year: 1990, month: null, to_year: 1999, to_month: null });
  assert.ok(isDecadeFilter(nineties));
  assert.equal(selectionFromFilter(nineties, legacyModel), null);

  // A decade-shaped *non* range (an explicit end month) is not a decade period.
  assert.ok(
    !isDecadeFilter(normalizeDateFilter({ year: 1960, month: 3, to_year: 1969, to_month: 12 }))
  );
  assert.ok(!isDecadeFilter(EMPTY_DATE_FILTER));
  assert.ok(!isDecadeFilter({ year: 1961, month: null, to_year: 1970, to_month: null }));
});
