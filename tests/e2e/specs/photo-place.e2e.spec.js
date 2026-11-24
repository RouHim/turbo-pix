import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const BASE_URL = `http://localhost:${process.env.TURBO_PIX_E2E_PORT ?? '18473'}`;

const escapeRegExp = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** A seeded photo whose coordinates resolved to a place name. */
async function photoWithPlace(page) {
  const response = await page.request.get(`${BASE_URL}/api/photos?limit=200`);
  if (!response.ok()) return null;
  const { photos } = await response.json();
  return (
    (photos || []).find(
      (photo) =>
        typeof photo.metadata?.location?.place === 'string' &&
        photo.metadata.location.place.trim() &&
        typeof photo.metadata?.location?.latitude === 'number'
    ) ?? null
  );
}

async function openMetadataPanel(page, hash) {
  await TestHelpers.openViewer(page, hash);
  await page.locator('.metadata-btn').click();
  await page.locator('.viewer-sidebar.show').waitFor();
}

test.describe('Photo place and mini map', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('the location section shows the resolved place with its country', async ({ page }) => {
    // GIVEN: a photo whose coordinates resolved offline to a place
    const photo = await photoWithPlace(page);
    test.skip(!photo, 'No seeded photo resolved to a place name');

    // WHEN: the viewer's metadata panel is opened
    await openMetadataPanel(page, photo.hash_sha256);

    // THEN: the place name is listed in the Location section
    const place = photo.metadata.location.place.trim();
    const placeButton = page.locator('[data-testid="meta-place"]');
    await expect(placeButton).toBeVisible();
    await expect(placeButton).toContainText(place);

    // AND: a resolved country is appended to it
    const countryCode = photo.metadata.location.country_code;
    if (countryCode) {
      await expect(placeButton).toHaveText(new RegExp(`^${escapeRegExp(place)}, .+`));
    }

    // AND: the coordinates stay visible next to it
    await expect(page.locator('#meta-gps')).toContainText(
      photo.metadata.location.latitude.toFixed(6)
    );
  });

  test('the mini map renders a marker and hands off to the focused map view', async ({ page }) => {
    // GIVEN: the metadata panel of a located photo
    const photo = await photoWithPlace(page);
    test.skip(!photo, 'No seeded photo resolved to a place name');
    await openMetadataPanel(page, photo.hash_sha256);

    // WHEN: the panel has scrolled the Location section into view
    const miniMap = page.locator('[data-testid="mini-map"]');
    await miniMap.scrollIntoViewIfNeeded();

    // THEN: the map renders its tiles container, one marker and both credits
    await expect(miniMap).toHaveClass(/leaflet-container/);
    await expect(miniMap.locator('.mini-map-marker')).toHaveCount(1);
    await expect(miniMap.locator('.leaflet-control-attribution')).toContainText(
      'OpenStreetMap contributors'
    );
    await expect(miniMap.locator('.leaflet-control-attribution')).toContainText('GeoNames');

    // WHEN: the user asks for the full map
    await page.locator('[data-testid="mini-map-open"]').click();

    // THEN: the app navigates to the focused map view
    await expect(page).toHaveURL(new RegExp(`/map\\?.*focus=${photo.hash_sha256}`));
    await expect(page.locator('#photo-viewer')).toBeHidden();

    // AND: that photo's location popup is open, naming the place
    const popup = page.locator('.leaflet-popup');
    await expect(popup).toBeVisible();
    await expect(popup.locator('[data-testid="map-popup-place"]')).toContainText(
      photo.metadata.location.place.trim()
    );
  });

  test('the ?focus= handoff still opens the popup with reduced motion', async ({ page }) => {
    // GIVEN: the OS reduce-motion preference, which makes every zoom step
    // synchronous: the popup must open from the render of the result set, not
    // from a chain of zoom retries climbing to the maximum zoom.
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const photo = await photoWithPlace(page);
    test.skip(!photo, 'No seeded photo resolved to a place name');
    await openMetadataPanel(page, photo.hash_sha256);

    // WHEN: the user asks for the full map from the mini map
    await page.locator('[data-testid="mini-map-open"]').click();

    // THEN: the handoff lands on the focused map view and its popup opens
    await expect(page).toHaveURL(new RegExp(`/map\\?.*focus=${photo.hash_sha256}`));
    const popup = page.locator('.leaflet-popup');
    await expect(popup).toBeVisible();
    await expect(popup.locator('[data-testid="map-popup-place"]')).toContainText(
      photo.metadata.location.place.trim()
    );
  });

  test('activating the place name filters the grid by that place', async ({ page }) => {
    // GIVEN: the metadata panel of a located photo
    const photo = await photoWithPlace(page);
    test.skip(!photo, 'No seeded photo resolved to a place name');
    await openMetadataPanel(page, photo.hash_sha256);

    // WHEN: the place name is activated
    const placeButton = page.locator('[data-testid="meta-place"]');
    await placeButton.scrollIntoViewIfNeeded();
    await placeButton.click();

    // THEN: the viewer closes and the grid shows the location: search
    await expect(page.locator('#photo-viewer')).toBeHidden();
    await expect(page).toHaveURL(/q=location%3A/);
    await TestHelpers.waitForPhotosToLoad(page);
    const cards = await TestHelpers.getPhotoCards(page);
    expect(cards.length).toBeGreaterThan(0);
  });

  test('a photo without coordinates shows no location section', async ({ page }) => {
    // GIVEN: a seeded photo the library has no coordinates or place for
    const response = await page.request.get(`${BASE_URL}/api/photos?limit=200`);
    expect(response.ok()).toBeTruthy();
    const { photos } = await response.json();
    const unlocated = (photos || []).find(
      (photo) =>
        photo.metadata?.location?.latitude == null &&
        photo.metadata?.location?.longitude == null &&
        !photo.metadata?.location?.place
    );
    test.skip(!unlocated, 'Every seeded photo carries coordinates');

    // WHEN: its metadata panel is opened
    await openMetadataPanel(page, unlocated.hash_sha256);

    // THEN: no Location section is rendered at all
    await expect(page.locator('#location-section')).toHaveCount(0);
  });
});
