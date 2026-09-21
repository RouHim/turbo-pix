import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

/** Geo-located photo count of a map endpoint response. */
async function expectedPhotos(page, endpoint) {
  return page.evaluate(async (url) => {
    const response = await fetch(url);
    const { photos } = await response.json();
    let located = 0;
    for (const photo of photos) {
      const latitude = photo.metadata?.location?.latitude;
      const longitude = photo.metadata?.location?.longitude;
      if (typeof latitude === 'number' && typeof longitude === 'number') located += 1;
    }
    return located;
  }, endpoint);
}

/**
 * Photos the map currently represents: a cluster announces the photos it
 * aggregates (FR-006) and a marker its location's photo count (FR-009), so
 * summing both element kinds covers every plotted photo exactly once.
 */
async function renderedPhotos(page) {
  return page.evaluate(() => {
    const announced = (selector, attribute) =>
      [...document.querySelectorAll(selector)].reduce(
        (sum, element) => sum + Number(element.getAttribute(attribute)),
        0
      );
    return (
      announced('[data-map-cluster]', 'data-map-cluster') +
      announced('[data-map-location]', 'data-map-location-count')
    );
  });
}

/**
 * Densest location of a map endpoint response as `[key, photoCount]`, derived
 * from the same filter the view is showing. Null when nothing is located.
 */
async function densestLocation(page, endpoint) {
  return page.evaluate(async (url) => {
    const response = await fetch(url);
    const { photos } = await response.json();
    const counts = new Map();
    for (const photo of photos) {
      const latitude = photo.metadata?.location?.latitude;
      const longitude = photo.metadata?.location?.longitude;
      if (typeof latitude !== 'number' || typeof longitude !== 'number') continue;
      const key = `${latitude},${longitude}`;
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1])[0] ?? null;
  }, endpoint);
}

/** Opens the popup for a location key, expanding clusters when needed. */
async function focusLocation(page, key) {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    if ((await page.locator(`[data-map-location="${key}"]`).count()) > 0) break;
    const cluster = page.locator('[data-map-cluster]').first();
    if ((await cluster.count()) === 0) break;
    await cluster.click();
    // The zoom animation pane detaches once the jump completes.
    await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
  }
  return page.locator(`[data-map-location="${key}"]`);
}

/**
 * The year of the newest dated photo. Read as UTC, like the server's
 * `strftime('%Y', taken_at)`: a local-year conversion could name the
 * neighbouring year right after New Year.
 */
async function latestYear(page) {
  return page.evaluate(async () => {
    const response = await fetch('/api/photos/map');
    const { photos } = await response.json();
    const withDate = photos.find((photo) => photo.taken_at);
    return withDate ? new Date(withDate.taken_at).getUTCFullYear() : null;
  });
}

/**
 * The map's own `/api/photos/map` request for a filter, so an assertion runs
 * against the filtered render instead of the one that was already on screen.
 */
function mapRequest(page, params) {
  return page.waitForResponse((response) => {
    const url = new URL(response.url());
    if (url.pathname !== '/api/photos/map') return false;
    return Object.entries(params).every(([key, value]) => url.searchParams.get(key) === value);
  });
}

/**
 * The loading overlay is removed after the loaded photo set has been applied,
 * so waiting for it to go away is what makes a state assertion (marker counts,
 * unlocated notice) read the finished render rather than a transient one.
 */
async function waitForMapLoad(page) {
  await expect(page.locator('[data-testid="map-loading"]')).toHaveCount(0);
}

test.describe('Map filters', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.stubMapTiles(page);
  });

  test('year filter plots exactly the geo-located subset of that year', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const year = await latestYear(page);
    test.skip(year === null, 'No dated photos in the test library');

    const filteredEndpoint = `/api/photos/map?year=${year}`;
    const expected = await expectedPhotos(page, filteredEndpoint);
    const densest = await densestLocation(page, filteredEndpoint);

    await page.evaluate((value) => {
      window.history.pushState({}, '', `/map?year=${value}`);
      window.dispatchEvent(new PopStateEvent('popstate'));
    }, year);
    await expect(page).toHaveURL(new RegExp(`/map\\?year=${year}`));

    // Filtered photos are a subset of the fitted initial viewport, so the
    // rendered representation must match the filtered set exactly (SC-003).
    await expect.poll(() => renderedPhotos(page)).toBe(expected);

    // Scoping, not just parity: the densest location's marker carries exactly
    // the filtered photos at that coordinate. Playwright retries the attribute
    // until the filtered reload has replaced the pre-filter render.
    if (densest) {
      const [key, count] = densest;
      const marker = await focusLocation(page, key);
      await expect(marker).toBeVisible();
      await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    }
  });

  test('location: search plots exactly the matching geo-located subset', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const city = await page.evaluate(async () => {
      const response = await fetch('/api/photos?limit=100');
      const { photos } = await response.json();
      return photos.find((photo) => photo.metadata?.location?.city)?.metadata.location.city ?? null;
    });
    test.skip(!city, 'No resolved city in the seeded photos');

    const query = `location:${city}`;
    const endpoint = `/api/photos/map?q=${encodeURIComponent(query)}`;
    const expected = await expectedPhotos(page, endpoint);
    const densest = await densestLocation(page, endpoint);
    expect(expected).toBeGreaterThan(0);

    // Wait for the map's own filtered request, so everything below reads the
    // filtered render instead of the unfiltered one still on screen.
    const filteredLoad = mapRequest(page, { q: query });
    await TestHelpers.performSearch(page, query);
    await filteredLoad;
    await waitForMapLoad(page);

    await expect.poll(() => renderedPhotos(page)).toBe(expected);
    // The unlocated notice renders only while the map is neither loading nor
    // errored, so a failed filtered load would leave it hidden and the check
    // below passing. Assert the load itself first.
    await expect(page.locator('[data-testid="map-error"]')).toHaveCount(0);
    // Teeth: every photo the city matches carries coordinates, while the
    // unfiltered render shows this notice for the unlocated videos, receipt,
    // and camera-EXIF fixture (`sample_with_exif.jpg` has no GPS tags).
    // A dropped query leaves the notice in place and fails here.
    await expect(page.locator('[data-testid="map-unlocated-notice"]')).toHaveCount(0);
    // Scoping, not just parity: the densest location's marker carries exactly
    // the filtered photos at its coordinate, not the library's photos there.
    if (densest) {
      const [key, count] = densest;
      const marker = await focusLocation(page, key);
      await expect(marker).toBeVisible();
      await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    }
  });

  test('semantic search plots the geo-located subset of the CLIP result set', async ({ page }) => {
    // CLIP text encoding plus the sqlite-vec scan is CPU-bound on the E2E box:
    // measured 13-18s per query, and this test drives the map's query plus a
    // hydration pass. That is far beyond Playwright's 30s global budget, so the
    // budget is raised here (all waits below stay selector/response-driven).
    test.setTimeout(180_000);
    await TestHelpers.goto(page, '/map');

    // 'car' is what the seeded media actually show (test-data/car.jpg). Wait
    // for the map's own CLIP query and derive the expectation from its
    // response, so the parity assertion is about the same result set the map
    // plots — not about a second, separately-timed query.
    const semanticLoad = page.waitForResponse((response) =>
      response.url().includes('/api/search/semantic')
    );
    await TestHelpers.performSearch(page, 'car');
    const search = await (await semanticLoad).json();

    // lib/map.js keeps paging until a page comes back short, so a short first
    // page proves this response is the whole result set (no paging to mirror).
    const hashes = (search.results ?? []).map((entry) => entry.hash);
    expect(hashes.length).toBeLessThan(200);

    const photos = await Promise.all(
      hashes.map(async (hash) => {
        const response = await page.request.get(`/api/photos/${hash}`);
        return response.ok() ? response.json() : null;
      })
    );
    const locationCounts = new Map();
    for (const photo of photos) {
      const latitude = photo?.metadata?.location?.latitude;
      const longitude = photo?.metadata?.location?.longitude;
      if (typeof latitude === 'number' && typeof longitude === 'number') {
        const key = `${latitude},${longitude}`;
        locationCounts.set(key, (locationCounts.get(key) ?? 0) + 1);
      }
    }
    // FR-006: the map announces photos, so parity counts the geo-located CLIP
    // hits themselves, not the locations they sit on.
    const expected = [...locationCounts.values()].reduce((sum, count) => sum + count, 0);
    const densest = [...locationCounts.entries()].sort((a, b) => b[1] - a[1])[0] ?? null;

    // Teeth: the CLIP hits are the car photos, which all carry coordinates,
    // while the unfiltered render shows the unlocated notice for the videos and
    // the receipt. Waiting for the map's own semantic load to finish and then
    // asserting the notice is gone fails if the semantic path ever plots the
    // whole library instead of its result set.
    await waitForMapLoad(page);
    // The unlocated notice renders only while the map is neither loading nor
    // errored, so a failed semantic load would leave it hidden and the check
    // below passing while the previous markers stay on screen.
    await expect(page.locator('[data-testid="map-error"]')).toHaveCount(0);
    await expect(page.locator('[data-testid="map-unlocated-notice"]')).toHaveCount(0);

    // Results can lie outside the fitted viewport (the map deliberately keeps
    // the user's viewport on filter changes), so zoom out to the world first.
    for (let step = 0; step < 4; step += 1) {
      await page.locator('.leaflet-control-zoom-out').click();
      await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    }

    if (expected === 0) {
      // A query the fixtures score below the CLIP threshold (0.615) still has a
      // defined map result: the empty state, never stale markers.
      await expect(page.locator('[data-testid="map-empty-state"]')).toBeVisible({ timeout: 30000 });
    } else {
      await expect.poll(() => renderedPhotos(page), { timeout: 30000 }).toBe(expected);
      // Scoping: the densest location's marker carries exactly the CLIP set's
      // photos at that coordinate, not the library's photos there.
      if (densest) {
        const [key, count] = densest;
        const marker = await focusLocation(page, key);
        await expect(marker).toBeVisible();
        await expect(marker).toHaveAttribute('data-map-location-count', String(count));
      }
    }
  });

  test('album scoping plots exactly the album members', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    // One hash per distinct coordinate, so the album spans as many locations as
    // the library has (never more than two: the assertion stays about scoping,
    // not about library size).
    const distinct = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      const seen = new Map();
      for (const photo of photos) {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        if (typeof latitude !== 'number' || typeof longitude !== 'number') continue;
        const key = `${latitude},${longitude}`;
        if (!seen.has(key)) seen.set(key, photo.hash_sha256);
      }
      return [...seen.values()];
    });
    test.skip(distinct.length === 0, 'Need geo-located photos to build an album from');

    const createResponse = await page.request.post('/api/albums', {
      data: { name: 'Map E2E Album', initial_hashes: distinct.slice(0, 2) },
    });
    expect(createResponse.ok()).toBe(true);
    const album = await createResponse.json();

    try {
      // The album's own location set, from the same filter the map applies.
      const albumLocations = await page.evaluate(async (albumId) => {
        const response = await fetch(`/api/photos/map?album=${albumId}`);
        const { photos } = await response.json();
        const counts = new Map();
        for (const photo of photos) {
          const latitude = photo.metadata?.location?.latitude;
          const longitude = photo.metadata?.location?.longitude;
          if (typeof latitude !== 'number' || typeof longitude !== 'number') continue;
          const key = `${latitude},${longitude}`;
          counts.set(key, (counts.get(key) ?? 0) + 1);
        }
        return [...counts.entries()];
      }, album.id);
      expect(albumLocations.length).toBeGreaterThan(0);
      const albumPhotos = albumLocations.reduce((sum, [, count]) => sum + count, 0);

      await TestHelpers.goto(page, `/map?album=${album.id}`);
      await expect.poll(() => renderedPhotos(page)).toBe(albumPhotos);

      // Scoping, not just parity: the marker carries only the album's members
      // at that coordinate, not every library photo sitting there.
      const [key, count] = albumLocations[0];
      const marker = await focusLocation(page, key);
      await expect(marker).toBeVisible();
      await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    } finally {
      await page.request.delete(`/api/albums/${album.id}`);
    }
  });

  test('videos with coordinates are plotted and open in the viewer', async ({ page }) => {
    // Videos cannot take EXIF writes, so seed the indexed row directly (the
    // same DB-seeding pattern the collage/housekeeping specs use). The
    // coordinate is cleared again in the `finally`: the whole run shares one
    // server and one database, so a leaked coordinate would break the
    // `?q=type:video` empty-state assertion of the map shell spec.
    TestHelpers.setPhotoLocationInDb('test_video.mp4', 52.52, 13.405);
    try {
      const listing = await page.request.get('/api/photos?q=type:video&limit=100');
      const { photos } = await listing.json();
      const video = photos.find((photo) => photo.filename === 'test_video.mp4');
      expect(video, 'test_video.mp4 must be seeded and indexed').toBeTruthy();

      await TestHelpers.goto(page, '/map');

      const marker = await focusLocation(page, '52.52,13.405');
      await expect(marker).toBeVisible();
      await marker.click();

      const item = page.locator('.leaflet-popup [data-map-popup-photo]').first();
      await expect(item).toHaveAttribute('data-map-popup-photo', video.hash_sha256);
      await item.click();

      await expect(page.locator('#photo-viewer')).toBeVisible();
      await expect(page).toHaveURL(new RegExp(`photo=${video.hash_sha256}`));
      // FR-016: video items play through the existing viewer behavior.
      await expect(page.locator('#viewer-video')).toBeVisible({ timeout: 30000 });
    } finally {
      TestHelpers.clearPhotoLocationInDb('test_video.mp4');
    }
  });

  test('back/forward restores the map with the same filter state', async ({ page }) => {
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();

    // A real filter first: the router listens for popstate, so this is the same
    // mechanism a user's filter change goes through.
    const year = await latestYear(page);
    test.skip(year === null, 'No dated photos in the test library');
    const endpoint = `/api/photos/map?year=${year}`;
    const expected = await expectedPhotos(page, endpoint);
    const densest = await densestLocation(page, endpoint);

    const filteredLoad = mapRequest(page, { year: String(year) });
    await page.evaluate((value) => {
      window.history.pushState({}, '', `/map?year=${value}`);
      window.dispatchEvent(new PopStateEvent('popstate'));
    }, year);
    await expect(page).toHaveURL(new RegExp(`/map\\?year=${year}`));
    await filteredLoad;
    await waitForMapLoad(page);
    await expect.poll(() => renderedPhotos(page)).toBe(expected);

    // Leaving the map keeps the filter in the history entry (the router
    // serializes the whole state), so Back has to restore it.
    await TestHelpers.navigateToView(page, 'videos');
    await expect(page).toHaveURL(/\/videos/);
    await page.goBack();

    await expect(page).toHaveURL(new RegExp(`/map\\?year=${year}`));
    await TestHelpers.verifyActiveView(page, 'map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    // The map remounts on the way back, so its markers have to be re-plotted
    // from the restored filter — a filterless return renders the unfiltered
    // photo count, which this poll rejects whenever the two differ. The densest
    // location's own photo count carries the regression when both sets happen
    // to hold the same number of photos.
    await expect.poll(() => renderedPhotos(page), { timeout: 15000 }).toBe(expected);
    if (densest) {
      const [key, count] = densest;
      const marker = await focusLocation(page, key);
      await expect(marker).toBeVisible();
      await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    }
  });
});
