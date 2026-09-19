// Map view data helpers. Pure functions plus the semantic-search result-set
// loader the Map view shares with the grid's search semantics. Imports only the
// pure query-token module, so the whole file stays unit-testable with
// node --test (tests/map-aggregates.test.js) — the api client is injected.
import { isPrefixQuery } from './query.js';

/** Server cap for /api/search/semantic (src/handlers_search.rs MAX_LIMIT). */
const SEMANTIC_PAGE_SIZE = 200;

/** Bound parallel /api/photos/{hash} hydration so a semantic page cannot
 *  flood the server (the grid hydrates one page at a time; the map needs the
 *  whole result set). */
const HYDRATE_CONCURRENCY = 8;

/**
 * Extracts validated coordinates from a photo's metadata.
 * Photos with only one coordinate, non-numeric values, or out-of-range values
 * are never plotted (the library rejects such data at write time; legacy rows
 * are skipped here).
 * @param {{ metadata?: { location?: { latitude?: unknown, longitude?: unknown } } } | null} photo
 * @returns {{ latitude: number, longitude: number } | null}
 */
export function getPhotoCoordinates(photo) {
  const latitude = photo?.metadata?.location?.latitude;
  const longitude = photo?.metadata?.location?.longitude;
  if (typeof latitude !== 'number' || typeof longitude !== 'number') return null;
  if (!Number.isFinite(latitude) || !Number.isFinite(longitude)) return null;
  if (latitude < -90 || latitude > 90 || longitude < -180 || longitude > 180) return null;
  return { latitude, longitude };
}

/**
 * Groups photos by identical coordinates into map locations, preserving the
 * input (sorted) order inside each location and across locations (FR-009/FR-010).
 * @param {Array} photos
 * @returns {Array<{ key: string, latitude: number, longitude: number, photos: Array }>}
 */
export function groupPhotosByLocation(photos) {
  const locations = new Map();
  for (const photo of photos ?? []) {
    const coordinates = getPhotoCoordinates(photo);
    if (!coordinates) continue;
    const key = `${coordinates.latitude},${coordinates.longitude}`;
    const location = locations.get(key);
    if (location) {
      location.photos.push(photo);
    } else {
      locations.set(key, { key, ...coordinates, photos: [photo] });
    }
  }
  return [...locations.values()];
}

/**
 * Resolved place name for a location: the first photo that carries one.
 * @returns {string | null}
 */
export function getLocationLabel(location) {
  for (const photo of location?.photos ?? []) {
    const city = photo?.metadata?.location?.city;
    if (typeof city === 'string' && city.trim()) return city.trim();
  }
  return null;
}

/**
 * Coordinate fallback for locations without a resolved place name (FR-012).
 * Matches the metadata panel's six-decimal convention.
 */
export function formatCoordinates({ latitude, longitude }) {
  return `${latitude.toFixed(6)}, ${longitude.toFixed(6)}`;
}

/**
 * Route state → /api/photos/map params, mirroring PhotoGrid.buildFilters:
 * favorites/videos view tokens never apply (the map is its own view), the
 * route query travels verbatim, sort/order split like the grid.
 */
export function buildMapFilters(route) {
  const filters = {
    query: route.query || null,
    year: route.year ?? undefined,
    month: route.month ?? undefined,
  };
  if (route.sort) {
    const [field, order] = route.sort.split('_');
    filters.sort = field;
    filters.order = order || 'desc';
  }
  return filters;
}

/**
 * Non-prefix queries run through CLIP semantic search, exactly like the grid
 * (prefix queries type:/location:/is_favorite: stay regular).
 */
export function isSemanticQuery(query) {
  return Boolean(query) && !isPrefixQuery(query);
}

/**
 * Loads the complete semantic result set (all pages, hydrated to full photo
 * rows) so the map's viewer navigation matches what the grid shows for the
 * same query. Stale photos (deleted between search and hydration) are skipped.
 * @param {{ semanticSearch: Function, getPhoto: Function }} client - api client
 * @param {string} query
 * @param {{ signal?: AbortSignal }} [options]
 * @returns {Promise<Array>}
 */
export async function fetchSemanticPhotoSet(client, query, { signal } = {}) {
  const cleanQuery = query.startsWith('@') ? query.substring(1).trim() : query;
  const photos = [];

  for (let offset = 0; ; offset += SEMANTIC_PAGE_SIZE) {
    const page = await client.semanticSearch(cleanQuery, SEMANTIC_PAGE_SIZE, offset, { signal });
    const hashes = (page?.results ?? []).map((result) => result.hash);
    if (hashes.length === 0) break;

    for (let index = 0; index < hashes.length; index += HYDRATE_CONCURRENCY) {
      const chunk = hashes.slice(index, index + HYDRATE_CONCURRENCY);
      const hydrated = await Promise.all(
        chunk.map((hash) => client.getPhoto(hash, { signal }).catch(() => null))
      );
      photos.push(...hydrated.filter((photo) => photo !== null));
    }

    if (hashes.length < SEMANTIC_PAGE_SIZE) break;
  }

  return photos;
}
