import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildYearAggregates, getYearAggregate } from '../frontend/src/lib/timeline.js';

test('empty density builds no aggregates', () => {
  assert.deepEqual(buildYearAggregates([]), []);
});

test('aggregates one year with zero-filled months and total', () => {
  const aggregates = buildYearAggregates([
    { year: 1998, month: 3, count: 4 },
    { year: 1998, month: 7, count: 2 },
  ]);
  assert.equal(aggregates.length, 1);
  assert.equal(aggregates[0].year, 1998);
  assert.equal(aggregates[0].total, 6);
  assert.equal(aggregates[0].months.length, 12);
  assert.equal(aggregates[0].months[2].count, 4);
  assert.equal(aggregates[0].months[6].count, 2);
  assert.equal(aggregates[0].months[0].count, 0);
});

test('sparse years sort newest-first', () => {
  const aggregates = buildYearAggregates([
    { year: 1970, month: 1, count: 1 },
    { year: 2024, month: 12, count: 3 },
    { year: 1998, month: 3, count: 2 },
  ]);
  assert.deepEqual(
    aggregates.map((a) => a.year),
    [2024, 1998, 1970]
  );
});

test('getYearAggregate returns null for unknown year', () => {
  const aggregates = buildYearAggregates([{ year: 2005, month: 5, count: 1 }]);
  assert.equal(getYearAggregate(aggregates, 1999), null);
  assert.equal(getYearAggregate(aggregates, 2005).total, 1);
});
