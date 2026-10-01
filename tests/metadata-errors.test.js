/**
 * Metadata refusal-code coverage guard.
 *
 * The metadata PATCH endpoint answers video refusals with an `error_code`;
 * every such code must map to a localized message that exists in BOTH
 * dictionaries, so a new backend refusal can never surface as a raw English
 * string (or an untranslated key) in the editor.
 *
 * The code list is parsed out of the backend's own `video_metadata_rejection`
 * in src/handlers_photo.rs rather than written out here, so adding a refusal
 * code on the server fails this test until the frontend map (and therefore the
 * bundles) catches up.
 *
 * Usage: node --test tests/metadata-errors.test.js  (part of npm run test:unit)
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { METADATA_ERROR_KEYS } from '../frontend/src/lib/metadataErrors.js';

const HANDLERS_PHOTO = new URL('../src/handlers_photo.rs', import.meta.url);

/**
 * Parse the refusal codes out of `fn video_metadata_rejection`: the slice runs
 * from the `fn` keyword to the function's own closing brace — the first `}` at
 * the start of a line — and every lowercase string literal it contains is an
 * `error_code`. Ending the slice at the next `async fn` instead would swallow
 * whatever helper comes after the refusal mapping (a `fn`, so not `async`), and
 * a lowercase literal there would be counted as a code. The count assertion
 * keeps a parse regression loud instead of silently yielding an empty (and thus
 * vacuous) set.
 */
function parseBackendRefusalCodes() {
  const source = readFileSync(HANDLERS_PHOTO, 'utf8');
  const start = source.indexOf('fn video_metadata_rejection');
  assert.ok(start !== -1, 'could not find `fn video_metadata_rejection` in src/handlers_photo.rs');
  const end = source.indexOf('\n}', start);
  assert.ok(end !== -1, 'could not find the closing brace of `fn video_metadata_rejection`');
  return new Set([...source.slice(start, end).matchAll(/"([a-z_]+)"/g)].map((m) => m[1]));
}

/** Resolve a dotted i18n key against a parsed bundle. */
function getPath(obj, path) {
  return path.split('.').reduce((value, part) => (value == null ? undefined : value[part]), obj);
}

test('every backend refusal code has a localized message in both bundles', () => {
  const en = JSON.parse(readFileSync(new URL('../frontend/src/i18n/en.json', import.meta.url)));
  const de = JSON.parse(readFileSync(new URL('../frontend/src/i18n/de.json', import.meta.url)));

  const backendCodes = parseBackendRefusalCodes();
  assert.equal(
    backendCodes.size,
    8,
    'video_metadata_rejection must yield exactly the eight refusal codes'
  );
  assert.deepEqual([...backendCodes].sort(), Object.keys(METADATA_ERROR_KEYS).sort());

  for (const key of Object.values(METADATA_ERROR_KEYS)) {
    assert.equal(typeof getPath(en, key), 'string', `${key} missing in en.json`);
    assert.equal(typeof getPath(de, key), 'string', `${key} missing in de.json`);
  }
});
