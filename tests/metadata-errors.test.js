/**
 * Metadata refusal-code coverage guard.
 *
 * The metadata PATCH endpoint answers video refusals with an `error_code`;
 * every such code must map to a localized message that exists in BOTH
 * dictionaries, so a new backend refusal can never surface as a raw English
 * string (or an untranslated key) in the editor.
 *
 * Usage: node --test tests/metadata-errors.test.js  (part of npm run test:unit)
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { METADATA_ERROR_KEYS } from '../frontend/src/lib/metadataErrors.js';

const expectedCodes = [
  'invalid_date',
  'invalid_coordinates',
  'unsupported_container',
  'no_location_carrier',
  'no_writable_slot',
  'unrepresentable_value',
  'file_read_only',
  'file_missing',
];

/** Resolve a dotted i18n key against a parsed bundle. */
function getPath(obj, path) {
  return path.split('.').reduce((value, part) => (value == null ? undefined : value[part]), obj);
}

test('every backend refusal code has a localized message in both bundles', () => {
  const en = JSON.parse(readFileSync(new URL('../frontend/src/i18n/en.json', import.meta.url)));
  const de = JSON.parse(readFileSync(new URL('../frontend/src/i18n/de.json', import.meta.url)));

  assert.deepEqual(Object.keys(METADATA_ERROR_KEYS).sort(), [...expectedCodes].sort());

  for (const key of Object.values(METADATA_ERROR_KEYS)) {
    assert.equal(typeof getPath(en, key), 'string', `${key} missing in en.json`);
    assert.equal(typeof getPath(de, key), 'string', `${key} missing in de.json`);
  }
});
