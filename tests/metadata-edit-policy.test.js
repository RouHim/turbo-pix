/**
 * Metadata-edit policy coverage.
 *
 * `buildMetadataEdit` decides the PATCH payload without rendering anything:
 * untouched fields stay out (the backend would rewrite them), the position
 * travels as a unit, and clearing a previously-set field is refused. These
 * cases pin that policy independently of the modal and its translations.
 *
 * Usage: node --test tests/metadata-edit-policy.test.js  (part of npm run test:unit)
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildMetadataEdit } from '../frontend/src/lib/metadataEditPolicy.js';

/** The form fields as the modal holds them, plus the opening state it installed. */
function form(overrides = {}) {
  return {
    takenAt: '',
    latitude: '',
    longitude: '',
    baseline: {
      takenAt: '',
      latitude: '',
      longitude: '',
      hadTakenAt: false,
      hadLatitude: false,
      hadLongitude: false,
    },
    ...overrides,
  };
}

test('leaves an untouched date out of the payload', () => {
  const result = buildMetadataEdit(
    form({
      takenAt: '2024-05-01T10:30',
      baseline: {
        takenAt: '2024-05-01T10:30',
        latitude: '',
        longitude: '',
        hadTakenAt: true,
        hadLatitude: false,
        hadLongitude: false,
      },
    })
  );

  assert.deepEqual(result, { updates: {} });
});

test('emits a changed date as an ISO instant', () => {
  const result = buildMetadataEdit(
    form({
      takenAt: '2024-06-02T08:15',
      baseline: {
        takenAt: '2024-05-01T10:30',
        latitude: '',
        longitude: '',
        hadTakenAt: true,
        hadLatitude: false,
        hadLongitude: false,
      },
    })
  );

  assert.deepEqual(result, {
    updates: { taken_at: new Date('2024-06-02T08:15').toISOString() },
  });
});

test('leaves an untouched position out of the payload', () => {
  const result = buildMetadataEdit(
    form({
      // The number input hands back a normalized spelling of what the form
      // installed; that is the same value, not an edit.
      latitude: '52.50',
      longitude: '13.40',
      baseline: {
        takenAt: '',
        latitude: '52.5',
        longitude: '13.4',
        hadTakenAt: false,
        hadLatitude: true,
        hadLongitude: true,
      },
    })
  );

  assert.deepEqual(result, { updates: {} });
});

test('emits a changed position as a unit', () => {
  const result = buildMetadataEdit(
    form({
      latitude: '52.6',
      longitude: '13.4',
      baseline: {
        takenAt: '',
        latitude: '52.5',
        longitude: '13.4',
        hadTakenAt: false,
        hadLatitude: true,
        hadLongitude: true,
      },
    })
  );

  assert.deepEqual(result, { updates: { latitude: 52.6, longitude: 13.4 } });
});

test('refuses a half position pair', () => {
  assert.deepEqual(buildMetadataEdit(form({ latitude: '52.5' })), { refusal: 'gps_pair' });
  assert.deepEqual(buildMetadataEdit(form({ longitude: '13.4' })), { refusal: 'gps_pair' });
});

test('refuses out-of-range coordinates', () => {
  assert.deepEqual(buildMetadataEdit(form({ latitude: '95', longitude: '10' })), {
    refusal: 'gps_range',
  });
  assert.deepEqual(buildMetadataEdit(form({ latitude: '52.5', longitude: '181' })), {
    refusal: 'gps_range',
  });
});

test('refuses an unparseable non-empty date', () => {
  assert.deepEqual(buildMetadataEdit(form({ takenAt: 'not-a-date' })), {
    refusal: 'invalid_date',
  });
});

test('refuses clearing a previously-set date, before any other refusal', () => {
  const result = buildMetadataEdit(
    form({
      takenAt: '',
      // Also a half pair, which must not mask the clear refusal.
      latitude: '52.5',
      baseline: {
        takenAt: '2024-05-01T10:30',
        latitude: '',
        longitude: '',
        hadTakenAt: true,
        hadLatitude: false,
        hadLongitude: false,
      },
    })
  );

  assert.deepEqual(result, { refusal: 'clear_unsupported' });
});

test('returns empty updates when nothing changed', () => {
  const result = buildMetadataEdit(
    form({
      takenAt: '2024-05-01T10:30',
      latitude: '52.5',
      longitude: '13.4',
      baseline: {
        takenAt: '2024-05-01T10:30',
        latitude: '52.5',
        longitude: '13.4',
        hadTakenAt: true,
        hadLatitude: true,
        hadLongitude: true,
      },
    })
  );

  assert.deepEqual(result, { updates: {} });
});
