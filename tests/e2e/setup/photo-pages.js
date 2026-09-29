/**
 * Paged listing helpers for the E2E harness.
 *
 * The server clamps `limit` to `MAX_PAGE_SIZE` (100, `src/handlers_photo.rs`)
 * WITHOUT telling the caller, so `?limit=200` silently answers the first page
 * only. The shared library grows as specs accept collages, and a truncated
 * listing would make global setup PATCH (or count) only the first page and then
 * fail with a misleading error — so every listing a helper depends on is walked
 * page by page instead of asked for in one request.
 */
export const MAX_PAGE_SIZE = 100;

/**
 * Every row of a photo listing, walked until the server says there is no next
 * page.
 *
 * `getJson(path)` receives a path WITH its query and resolves to the parsed
 * body; turning a non-OK response into the caller's own error stays the
 * caller's job, so each site keeps its own message. `path` may already carry a
 * query (`/api/photos?q=type:video`) or name another listing
 * (`/api/albums/<id>/photos`); it is preserved.
 */
export async function fetchAllPhotos(getJson, path = '/api/photos') {
  const photos = [];
  for (let page = 1; ; page += 1) {
    const separator = path.includes('?') ? '&' : '?';
    const body = await getJson(`${path}${separator}page=${page}&limit=${MAX_PAGE_SIZE}`);
    const batch = body.photos || [];
    photos.push(...batch);
    if (!body.has_next || batch.length === 0) {
      return photos;
    }
  }
}
