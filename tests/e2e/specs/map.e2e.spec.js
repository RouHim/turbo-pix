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
});
