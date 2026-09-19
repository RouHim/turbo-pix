import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Map view', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.stubMapTiles(page);
  });

  test('sidebar navigation opens the map and updates the URL', async ({ page }) => {
    await TestHelpers.goto(page);
    await TestHelpers.waitForSearchReady(page);

    await TestHelpers.navigateToView(page, 'map');

    await expect(page).toHaveURL(/\/map$/);
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await expect(page.locator('#current-view-title')).toHaveText('Map');
  });

  test('direct URL load renders the map identically', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await TestHelpers.verifyActiveView(page, 'map');
  });

  test('shows visible OSM attribution and no viewport in the URL', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const attribution = page.locator('.leaflet-control-attribution');
    await expect(attribution).toBeVisible();
    await expect(attribution).toContainText('OpenStreetMap');

    // FR-017: center/zoom are never serialized into the URL.
    await expect(page).not.toHaveURL(/(zoom|lat|lng|center)=/);
  });

  test('filters with matching photos but no coordinates show the empty state', async ({ page }) => {
    // The seeded videos carry no GPS; the map must explain instead of showing a blank map.
    await TestHelpers.goto(page, '/map?q=type%3Avideo');

    await expect(page.locator('[data-testid="map-empty-state"]')).toBeVisible();
    await expect(page.locator('[data-testid="map-empty-state"]')).toContainText(
      'No photos with location data'
    );
  });

  test('reports the number of matching photos without location data', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const expected = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      return photos.filter((photo) => {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        return typeof latitude !== 'number' || typeof longitude !== 'number';
      }).length;
    });

    await expect(page.locator('[data-testid="map-unlocated-notice"]')).toContainText(
      `${expected} photos without location data`
    );
  });

  test('plots every geo-located location as a marker or cluster', async ({ page }) => {
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();

    const expectedLocations = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
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
    });
    test.skip(expectedLocations === 0, 'No geo-located photos in the test library');

    // Supercluster guarantees every point is represented: a cluster reports how
    // many locations it aggregates (FR-006), so clusters + individual markers
    // must equal the library's unique coordinates (FR-009).
    await expect
      .poll(async () =>
        page.evaluate(() => {
          const clustered = [...document.querySelectorAll('[data-map-cluster]')].reduce(
            (sum, element) => sum + Number(element.getAttribute('data-map-cluster')),
            0
          );
          return clustered + document.querySelectorAll('[data-map-location]').length;
        })
      )
      .toBe(expectedLocations);
  });

  test('cluster click separates the cluster into individual markers', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const cluster = page.locator('[data-map-cluster]').first();
    test.skip((await cluster.count()) === 0, 'Test library has no cluster at the initial zoom');

    const before = await page.locator('[data-map-location]').count();
    await cluster.click();

    await expect
      .poll(async () => page.locator('[data-map-location]').count())
      .toBeGreaterThan(before);
  });

  // Expand clusters until individual location markers render, then open the
  // first one's popup.
  async function openFirstLocationPopup(page) {
    for (let attempt = 0; attempt < 5; attempt += 1) {
      if ((await page.locator('[data-map-location]').count()) > 0) break;
      const cluster = page.locator('[data-map-cluster]').first();
      if ((await cluster.count()) === 0) break;
      await cluster.click();
      await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    }
    const marker = page.locator('[data-map-location]').first();
    await expect(marker).toBeVisible();
    await marker.click();
    return page.locator('.leaflet-popup');
  }

  test('popup shows the place name and opens the viewer on a thumbnail', async ({ page }) => {
    await TestHelpers.goto(page, '/map');
    const popup = await openFirstLocationPopup(page);

    const key = await page.locator('[data-map-location]').first().getAttribute('data-map-location');
    const expectedPlace = await page.evaluate(async (locationKey) => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      const match = photos.find((photo) => {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        return typeof latitude === 'number' && `${latitude},${longitude}` === locationKey;
      });
      const city = match?.metadata?.location?.city;
      if (typeof city === 'string' && city.trim()) return city.trim();
      const [latitude, longitude] = locationKey.split(',').map(Number);
      return `${latitude.toFixed(6)}, ${longitude.toFixed(6)}`;
    }, key);

    await expect(popup).toBeVisible();
    await expect(popup.locator('[data-testid="map-popup-place"]')).toHaveText(expectedPlace);
    await expect(popup.locator('[data-map-popup-photo]').first()).toBeVisible();

    const firstThumbnail = popup.locator('[data-map-popup-photo]').first();
    const hash = await firstThumbnail.getAttribute('data-map-popup-photo');
    await firstThumbnail.click();

    await expect(page.locator('#photo-viewer')).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`photo=${hash}`));

    await page.keyboard.press('Escape');
    await expect(page.locator('#photo-viewer')).toBeHidden();
  });

  test('photos sharing coordinates collapse into one marker with a full, scrollable list', async ({
    page,
  }) => {
    await TestHelpers.goto(page, '/map');

    const expected = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
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
    });
    test.skip(!expected, 'No geo-located photos in the test library');

    const [key, count] = expected;

    for (let attempt = 0; attempt < 6; attempt += 1) {
      if ((await page.locator(`[data-map-location="${key}"]`).count()) > 0) break;
      const cluster = page.locator('[data-map-cluster]').first();
      if ((await cluster.count()) === 0) break;
      await cluster.click();
      await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    }

    const marker = page.locator(`[data-map-location="${key}"]`);
    await expect(marker).toBeVisible();
    await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    await marker.click();

    const popup = page.locator('.leaflet-popup');
    // FR-009/FR-010: every photo of that location is listed (never truncated).
    await expect(popup.locator('[data-map-popup-photo]')).toHaveCount(count);

    if (count > 5) {
      const list = await popup.locator('.map-popup-list').evaluate((element) => ({
        scrollHeight: element.scrollHeight,
        clientHeight: element.clientHeight,
      }));
      expect(list.scrollHeight).toBeGreaterThan(list.clientHeight);
    }
  });
});
