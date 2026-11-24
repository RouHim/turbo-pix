// Pure helpers for the current-date hint shown while the content header is out
// of view. No Svelte or DOM imports; node --test only.

/**
 * Hysteresis around the fixed header line: the hint enters when the content
 * header reaches the line and leaves only after it is back below the band.
 */
export function nextHintVisibility(headerBottom, wasVisible, headerHeight, hysteresis) {
  return wasVisible ? headerBottom <= headerHeight + hysteresis : headerBottom <= headerHeight;
}

/** The hint is a chronological affordance; other sorts have no stable date. */
export function isDateSort(sort) {
  return sort === 'date_desc' || sort === 'date_asc';
}

/**
 * Routes that render PhotoGrid. Collages and housekeeping reuse the
 * `#photo-grid` id and `.photo-card` class for their own cards, so the DOM
 * alone cannot gate the hint; this mirrors App's PhotoGrid branch.
 */
export function isPhotoGridRoute(route) {
  if (route.view === 'all' || route.view === 'favorites' || route.view === 'videos') return true;
  if (route.view === 'albums') return route.album != null || route.query != null;
  if (route.view === 'people') return route.person != null || route.query != null;
  return false;
}

/** Long localized date of the top visible photo, or null when it has none. */
export function formatHintDate(takenAt, locale = 'en') {
  if (!takenAt) return null;
  const date = new Date(takenAt);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleDateString(locale, { dateStyle: 'long' });
}
