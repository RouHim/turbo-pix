// Map view data helpers. Pure functions plus the semantic-search result-set
// loader the Map view shares with the grid's search semantics. Imports only the
// pure query-token module, so the whole file stays unit-testable with
// node --test (tests/map-aggregates.test.js) — the api client is injected.
import { isPrefixQuery } from './query.js';

/**
 * Server cap for /api/search/semantic: the backend clamps every request to
 * `MAX_LIMIT` in src/handlers_search.rs, so this page size must never exceed
 * it. No module is shared between Rust and JS, so the copy is pinned by
 * tests/map-page-size.test.js, which parses `MAX_LIMIT` out of the Rust source
 * and fails `npm run test:unit` when the two drift. Change both together.
 */
export const SEMANTIC_PAGE_SIZE = 200;

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
    const place = photo?.metadata?.location?.place;
    if (typeof place === 'string' && place.trim()) return place.trim();
  }
  return null;
}

/**
 * Localized country name for an ISO-3166 alpha-2 code.
 *
 * Rendered by the browser's own region data, so the panel, the map and an
 * exported label follow the UI language without a second copy of every country
 * name in the translation files. Malformed codes, codes the runtime does not
 * know (it answers every one of those with the same "unknown region" string)
 * and runtimes without `Intl.DisplayNames` fall back to the code itself.
 *
 * @param {unknown} code
 * @param {string} [locale]
 * @returns {string | null}
 */
export function countryName(code, locale = 'en') {
  const normalized = typeof code === 'string' ? code.trim().toUpperCase() : '';
  if (!normalized) return null;
  if (!/^[A-Z]{2}$/.test(normalized)) return normalized;
  try {
    const names = regionDisplayNames(locale);
    const resolved = names.of(normalized);
    if (!resolved || resolved === names.of('ZZ')) return normalized;
    return resolved;
  } catch {
    return normalized;
  }
}

/** One `Intl.DisplayNames` per locale; construction is the expensive part. */
const regionNamesByLocale = new Map();

function regionDisplayNames(locale) {
  let names = regionNamesByLocale.get(locale);
  if (!names) {
    names = new Intl.DisplayNames([locale], { type: 'region' });
    regionNamesByLocale.set(locale, names);
  }
  return names;
}

/**
 * The panel label for a photo's place: the resolved `place`, with the country
 * name appended when the dataset carries a `country_code`.
 *
 * @param {{ metadata?: { location?: object } } | null} photo
 * @param {string} [locale]
 * @returns {string | null}
 */
export function formatPlaceLabel(photo, locale = 'en') {
  const location = photo?.metadata?.location;
  const place = typeof location?.place === 'string' ? location.place.trim() : '';
  if (!place) return null;
  const country = countryName(location?.country_code, locale);
  return country ? `${place}, ${country}` : place;
}

/**
 * Coordinate fallback for locations without a resolved place name (FR-012).
 * Matches the metadata panel's six-decimal convention.
 */
export function formatCoordinates({ latitude, longitude }) {
  return `${latitude.toFixed(6)}, ${longitude.toFixed(6)}`;
}

/**
 * Moves a longitude into the copy of the viewport Leaflet actually shows.
 * Markers are placed via `latLngToLayerPoint`/`project`, which never wrap
 * longitude — only the tile URL wraps — so `bounds` is the raw visible
 * window, e.g. [-155.35, 182.15] for a 1920px window at zoom 3 centred on
 * Berlin. Panning is unbounded, so the window can sit more than one world away
 * from the canonical longitude: pick the `±360°` copy nearest the window
 * centre, and use it only when it falls inside the window, so a photo can
 * never be pushed off-screen by a seam. Returns `lng` unchanged when no copy
 * is visible.
 * @param {number} lng - photo longitude, between -180 and 180
 * @param {number} west - bounds.getWest()
 * @param {number} east - bounds.getEast()
 * @returns {number}
 */
export function wrapLongitudeForView(lng, west, east) {
  const shifted = lng + 360 * Math.round(((west + east) / 2 - lng) / 360);
  return shifted >= west && shifted <= east ? shifted : lng;
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
