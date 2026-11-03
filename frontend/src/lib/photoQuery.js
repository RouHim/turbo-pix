// Route → API filter params, shared by the grid and the map.
// Pure module: no Svelte, api, or i18n imports, so it stays unit-testable with
// node --test (tests/photo-query.test.js).

/**
 * Translates the route's filter fields into the param shape the photo
 * endpoints accept. Both views read the same URL fields; only the query
 * differs: favorites/videos view tokens are grid-only, while the map is its
 * own view and forwards the route query verbatim.
 *
 * Keys without a value are omitted (never `undefined` or `null`), so callers
 * can spread the result straight into API params.
 * @param {{ view?: string, query?: string|null, sort?: string|null,
 *   year?: number|null, month?: number|null, to_year?: number|null,
 *   to_month?: number|null, album?: number|null }} route
 * @param {{ view?: 'grid'|'map' }} [options]
 * @returns {{ query?: string, sort?: string, order?: string, year?: number,
 *   month?: number, toYear?: number, toMonth?: number, album?: number }}
 */
export function routeToPhotoQuery(route, { view = 'grid' } = {}) {
  const params = {};

  if (view === 'grid') {
    if (route.view === 'favorites') params.query = 'is_favorite:true';
    if (route.view === 'videos') params.query = 'type:video';
  } else if (route.query) {
    params.query = route.query;
  }

  if (route.sort) {
    const [field, order] = route.sort.split('_');
    params.sort = field;
    params.order = order || 'desc';
  }

  if (route.year != null) params.year = route.year;
  if (route.month != null) params.month = route.month;
  if (route.to_year != null) params.toYear = route.to_year;
  if (route.to_month != null) params.toMonth = route.to_month;
  if (view === 'map' && route.album != null) params.album = route.album;

  return params;
}
