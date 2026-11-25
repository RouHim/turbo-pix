import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  formatDate,
  formatDateTime,
  formatLongDate,
  formatMonthName,
  formatWeekdayDate,
} from '../frontend/src/lib/datetime.js';

test('formatDate renders a medium date per locale', () => {
  const value = new Date(2026, 10, 4, 13, 0);

  assert.equal(formatDate(value, 'en'), 'Nov 4, 2026');
  assert.equal(formatDate(value, 'de'), '04.11.2026');
});

test('formatDateTime adds the short local time per locale', () => {
  const value = new Date(2026, 10, 4, 13, 0);

  assert.equal(formatDateTime(value, 'en'), 'Nov 4, 2026, 1:00 PM');
  assert.equal(formatDateTime(value, 'de'), '04.11.2026, 13:00');
});

test('formatWeekdayDate keeps a calendar date on its own UTC day', () => {
  const previousTz = process.env.TZ;
  process.env.TZ = 'America/New_York';
  try {
    assert.equal(formatWeekdayDate('2026-11-04', 'en'), 'Wednesday, November 4, 2026');
    assert.equal(formatWeekdayDate('2026-11-04', 'de'), 'Mittwoch, 4. November 2026');
  } finally {
    if (previousTz === undefined) delete process.env.TZ;
    else process.env.TZ = previousTz;
  }
});

test('formatMonthName renders long and short month names', () => {
  assert.equal(formatMonthName(1, 'en'), 'January');
  assert.equal(formatMonthName(3, 'de'), 'März');
  assert.equal(formatMonthName(3, 'de', true), 'Mär');
  assert.equal(formatMonthName(0, 'en'), '');
  assert.equal(formatMonthName(13, 'en'), '');
});

test('formatters reject missing, malformed and impossible values', () => {
  assert.equal(formatDate(null, 'en'), '');
  assert.equal(formatDate('not-a-date', 'en'), '');
  assert.equal(formatDateTime(undefined, 'de'), '');
  assert.equal(formatWeekdayDate('2026-02-30', 'en'), '');
  assert.equal(formatWeekdayDate('04.11.2026', 'de'), '');
});

test('formatLongDate renders the long date used by the date hint', () => {
  const value = new Date(2024, 2, 14, 12, 0);

  assert.equal(formatLongDate(value, 'de'), '14. März 2024');
  assert.equal(formatLongDate(value, 'en'), 'March 14, 2024');
  assert.equal(formatLongDate('not-a-date', 'en'), '');
});
