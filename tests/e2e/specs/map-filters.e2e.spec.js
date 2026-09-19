import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const BASE_URL = 'http://localhost:18473';

/** Unique-coordinate count of a map endpoint response. */
async function expectedLocations(page, endpoint) {
  return page.evaluate(async (url) => {
    const response = await fetch(url);
    const { photos } = await response.json();
    const keys = new Set();
    for (const photo of photos) {
      const latitude = photo.metadata?.location?.latitude;
      const longitude = photo.metadata?.location?.longitude;
      if (typeof latitude === 'number' && typeof longitude === 'number') {
        keys.add(`${latitude},${longitude}`);
      }
    }
    return keys.size;
  }, endpoint);
}

/** Locations the map currently represents: cluster counts sum + single markers. */
async function renderedLocations(page) {
  return page.evaluate(() => {
    const clustered = [...document.querySelectorAll('[data-map-cluster]')].reduce(
      (sum, element) => sum + Number(element.getAttribute('data-map-cluster')),
      0
    );
    return clustered + document.querySelectorAll('[data-map-location]').length;
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

test.describe('Map filters', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.stubMapTiles(page);
  });

  test('year filter plots exactly the geo-located subset of that year', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const year = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      const withDate = photos.find((photo) => photo.taken_at);
      // UTC like the server's strftime('%Y', taken_at): a local-year conversion
      // could name the neighbouring year right after New Year.
      return withDate ? new Date(withDate.taken_at).getUTCFullYear() : null;
    });
    test.skip(year === null, 'No dated photos in the test library');

    const filteredEndpoint = `/api/photos/map?year=${year}`;
    const expected = await expectedLocations(page, filteredEndpoint);
    const densest = await densestLocation(page, filteredEndpoint);

    await page.evaluate((value) => {
      window.history.pushState({}, '', `/map?year=${value}`);
      window.dispatchEvent(new PopStateEvent('popstate'));
    }, year);
    await expect(page).toHaveURL(new RegExp(`/map\\?year=${year}`));

    // Filtered locations are a subset of the fitted initial viewport, so the
    // rendered representation must match the filtered set exactly (SC-003).
    await expect.poll(() => renderedLocations(page)).toBe(expected);

    // Photo-level parity: the marker carries exactly the filtered photos at
    // that coordinate. A location count alone can coincide with the unfiltered
    // one (a one-location library always renders one marker); a photo count
    // cannot. Playwright retries the attribute until the filtered reload has
    // replaced the pre-filter render.
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

    const expected = await expectedLocations(
      page,
      `/api/photos/map?q=${encodeURIComponent(`location:${city}`)}`
    );
    expect(expected).toBeGreaterThan(0);

    await TestHelpers.performSearch(page, `location:${city}`);

    await expect.poll(() => renderedLocations(page)).toBe(expected);
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

    const photos = await Promise.all(
      (search.results ?? []).map(async (entry) => {
        const response = await page.request.get(`${BASE_URL}/api/photos/${entry.hash}`);
        return response.ok() ? response.json() : null;
      })
    );
    const keys = new Set();
    for (const photo of photos) {
      const latitude = photo?.metadata?.location?.latitude;
      const longitude = photo?.metadata?.location?.longitude;
      if (typeof latitude === 'number' && typeof longitude === 'number') {
        keys.add(`${latitude},${longitude}`);
      }
    }
    const expected = keys.size;

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
      await expect.poll(() => renderedLocations(page), { timeout: 30000 }).toBe(expected);
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

    const createResponse = await page.request.post(`${BASE_URL}/api/albums`, {
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

      await TestHelpers.goto(page, `/map?album=${album.id}`);
      await expect.poll(() => renderedLocations(page)).toBe(albumLocations.length);

      // Scoping, not just parity: the marker carries only the album's members
      // at that coordinate, not every library photo sitting there.
      const [key, count] = albumLocations[0];
      const marker = await focusLocation(page, key);
      await expect(marker).toBeVisible();
      await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    } finally {
      await page.request.delete(`${BASE_URL}/api/albums/${album.id}`);
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
      const listing = await page.request.get(`${BASE_URL}/api/photos?q=type:video&limit=100`);
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

    await TestHelpers.navigateToView(page, 'videos');
    await page.goBack();

    await expect(page).toHaveURL(/\/map$/);
    await TestHelpers.verifyActiveView(page, 'map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
  });
});
