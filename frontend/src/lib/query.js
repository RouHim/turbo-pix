// Query-token classification shared by the grid, the search bar, and the map.
// Pure module: no Svelte or api imports, so it stays unit-testable with
// node --test (see tests/map-aggregates.test.js).

/**
 * True for backend filter-prefix queries (type:, location:, is_favorite:),
 * which the search pipeline routes through the regular (non-semantic) path.
 * Keep in sync with SearchBar.performSearch.
 * @param {string} q
 * @returns {boolean}
 */
export function isPrefixQuery(q) {
  return q.startsWith('type:') || q.startsWith('location:') || q.startsWith('is_favorite:');
}
