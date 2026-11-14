/**
 * Semantic-search page-size drift guard.
 *
 * The map loads every page of /api/search/semantic, so its page size has to
 * match the backend's clamp. Rust and JS share no module, so the JS copy
 * (frontend/src/lib/map.js SEMANTIC_PAGE_SIZE) is checked against the Rust
 * source of truth (src/handlers_search.rs MAX_LIMIT) by parsing it, the way
 * tests/i18n-integrity.test.js parses frontend sources.
 *
 * Usage: node --test tests/map-page-size.test.js  (part of npm run test:unit)
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { SEMANTIC_PAGE_SIZE } from '../frontend/src/lib/map.js';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const HANDLERS_SEARCH_RS = join(ROOT, 'src', 'handlers_search.rs');
const MAP_JS = join(ROOT, 'frontend', 'src', 'lib', 'map.js');

/** Reads the backend's MAX_LIMIT; a missing constant is a broken guard, not a pass. */
function readBackendMaxLimit() {
  const source = readFileSync(HANDLERS_SEARCH_RS, 'utf8');
  const match = source.match(/const\s+MAX_LIMIT\s*:\s*usize\s*=\s*(\d+)\s*;/);
  assert.ok(
    match,
    `Could not find \`const MAX_LIMIT: usize = <n>;\` in ${HANDLERS_SEARCH_RS}; the map page-size guard needs it`
  );
  return Number(match[1]);
}

test('map semantic page size matches the backend MAX_LIMIT', () => {
  // GIVEN the backend clamps /api/search/semantic to MAX_LIMIT
  const maxLimit = readBackendMaxLimit();

  // THEN the map asks for exactly that page size: a larger request would be
  // clamped (short page, early stop), a smaller one is a silent drift
  assert.equal(
    SEMANTIC_PAGE_SIZE,
    maxLimit,
    `frontend/src/lib/map.js SEMANTIC_PAGE_SIZE (${SEMANTIC_PAGE_SIZE}) drifted from ` +
      `src/handlers_search.rs MAX_LIMIT (${maxLimit})`
  );
});

test('map pages semantic search with the guarded constant', () => {
  // GIVEN the loader sends the page size to the client
  const source = readFileSync(MAP_JS, 'utf8');

  // THEN the guarded constant is the value actually sent — a hardcoded literal
  // would leave the drift check above green while paging past the cap
  assert.match(
    source,
    /semanticSearch\(\s*cleanQuery,\s*SEMANTIC_PAGE_SIZE\s*,/,
    'frontend/src/lib/map.js must pass SEMANTIC_PAGE_SIZE to client.semanticSearch()'
  );
});
