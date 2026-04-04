import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const BASE_URL = 'http://localhost:18473';

test.describe('Location Search', () => {
  let cityName = null;

  test.beforeAll(async () => {
    const response = await fetch(`${BASE_URL}/api/photos?limit=50`);
    if (response.ok) {
      const data = await response.json();
      const photos = data.photos || [];
      const photoWithCity = photos.find(
        (photo) => photo.filename?.startsWith('cluster_') && photo.metadata?.location?.city
      );
      if (photoWithCity) {
        cityName = photoWithCity.metadata.location.city;
        console.log(`Discovered city name from seeded photos: "${cityName}"`);
      } else {
        console.warn('No seeded photos have a city name — geo_resolution may not have completed');
      }
    }
  });

  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForSearchReady(page);
  });

  test('API: seeded photos have city names after indexing', async ({ page }) => {
    // GIVEN: Server running with geo_resolution complete
    // WHEN: Fetch /api/photos?limit=50
    const response = await page.request.get(`${BASE_URL}/api/photos?limit=50`);
    expect(response.ok()).toBe(true);

    const data = await response.json();
    const photos = data.photos || [];

    // THEN: At least one cluster_*.jpg has metadata.location.city that is non-null
    const photosWithCity = photos.filter(
      (photo) => photo.filename?.startsWith('cluster_') && photo.metadata?.location?.city
    );
    expect(photosWithCity.length).toBeGreaterThan(0);
  });

  test('UI: location: prefix search returns photos', async ({ page }) => {
    // GIVEN: city name known from beforeAll (or skip if null)
    test.skip(!cityName, 'No city name found in seeded photos — geo_resolution incomplete');

    // WHEN: User types `location:<city>` in search input and clicks search
    await TestHelpers.performSearch(page, `location:${cityName}`);

    // THEN: At least one photo card appears in results
    await TestHelpers.waitForPhotosToLoad(page);
    const cards = await page.locator('[data-photo-id]').all();
    expect(cards.length).toBeGreaterThan(0);
  });

  test('UI: nonexistent city returns empty results', async ({ page }) => {
    // GIVEN: User on homepage
    // WHEN: User types `location:Atlantis` and searches
    await TestHelpers.performSearch(page, 'location:Atlantis');

    // THEN: No [data-photo-id] elements visible
    await page.waitForTimeout(2000);
    const cards = await page.locator('[data-photo-id]').all();
    expect(cards.length).toBe(0);
  });

  test('UI: search hint tooltip appears on focus', async ({ page }) => {
    // GIVEN: User on homepage
    // WHEN: User clicks on the search input
    await page.click(TestHelpers.selectors.searchInput);

    // THEN: Element with [data-search-hint="true"] is visible
    const hint = page.locator('[data-search-hint="true"]');
    await expect(hint).toBeVisible();

    // AND: It contains text "location:"
    await expect(hint).toContainText('location:');
  });

  test('UI: search hint disappears when typing', async ({ page }) => {
    // GIVEN: Search hint is visible (after click)
    await page.click(TestHelpers.selectors.searchInput);
    const hint = page.locator('[data-search-hint="true"]');
    await expect(hint).toBeVisible();

    // WHEN: User types into search input
    await page.locator(TestHelpers.selectors.searchInput).pressSequentially('hello');

    // THEN: [data-search-hint="true"] has opacity 0 (hidden via inline style)
    await expect(hint).toHaveCSS('opacity', '0');
  });

  test('Network: location: prefix routes to text search, NOT semantic', async ({ page }) => {
    // GIVEN: User on homepage
    const requests = [];
    page.on('request', (request) => {
      requests.push(request.url());
    });

    // WHEN: User types "location:Berlin" and submits search
    await TestHelpers.performSearch(page, 'location:Berlin');

    await page.waitForTimeout(2000);

    // THEN: A request to /api/photos is made (containing "location:Berlin")
    const textSearchRequest = requests.find(
      (url) => url.includes('/api/photos') && url.includes('location')
    );
    expect(textSearchRequest).toBeDefined();

    // AND: NO request to /api/search/semantic is made
    const semanticRequest = requests.find((url) => url.includes('/api/search/semantic'));
    expect(semanticRequest).toBeUndefined();
  });
});
