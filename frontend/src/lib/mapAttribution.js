// The credit line every map surface renders.
//
// Both sources are required: OpenStreetMap for the raster tiles (its tile policy
// demands visible attribution) and GeoNames for the place names resolved from
// the embedded dataset (CC BY 4.0). Keeping one builder means the Map view and
// the metadata panel's mini map cannot drift apart.
//
// Imports nothing: callers pass their already-translated labels in, so this
// module stays outside the i18n store and the DOM.

/** Leaflet's ceiling on every map surface; also the point where clusters stop. */
export const MAP_MAX_ZOOM = 19;

/**
 * @param {string} tilesLabel - localized "OpenStreetMap contributors"
 * @param {string} placesLabel - localized "Place data © GeoNames"
 * @returns {string} HTML for Leaflet's attribution control
 */
export function mapAttributionHtml(tilesLabel, placesLabel) {
  return `&copy; <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noopener">${tilesLabel}</a> &middot; <a href="https://www.geonames.org" target="_blank" rel="noopener">${placesLabel}</a>`;
}
