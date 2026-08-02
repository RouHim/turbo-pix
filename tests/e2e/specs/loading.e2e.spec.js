import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Startup loading behavior', () => {
  test('loads photos when indexing is active but metadata phase is done', async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);

    await page.route('**/api/indexing/status', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          is_indexing: true,
          is_complete: false,
          started_at: new Date().toISOString(),
          active_phase_id: 'housekeeping',
          phases: [
            {
              id: 'discovering',
              state: 'done',
              kind: 'indeterminate',
              processed: 0,
              total: null,
              errors: 0,
              current_item: null,
            },
            {
              id: 'metadata',
              state: 'done',
              kind: 'determinate',
              processed: 237,
              total: 237,
              errors: 0,
              current_item: null,
            },
            {
              id: 'semantic_vectors',
              state: 'done',
              kind: 'determinate',
              processed: 237,
              total: 237,
              errors: 0,
              current_item: null,
            },
            {
              id: 'collages',
              state: 'done',
              kind: 'indeterminate',
              processed: 0,
              total: null,
              errors: 0,
              current_item: null,
            },
            {
              id: 'housekeeping',
              state: 'active',
              kind: 'indeterminate',
              processed: 0,
              total: null,
              errors: 0,
              current_item: null,
            },
          ],
          photos_indexed: 237,
        }),
      });
    });

    const photosResponse = page.waitForResponse(
      (response) =>
        response.url().includes('/api/photos') &&
        response.request().method() === 'GET' &&
        response.status() === 200
    );

    await TestHelpers.goto(page);
    await photosResponse;
    await TestHelpers.waitForPhotosToLoad(page);

    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);
  });

  test('shows error state on failed load and recovers via Try Again', async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    let failPhotos = true;
    await page.route(
      (url) => url.pathname === '/api/photos',
      async (route) => {
        if (failPhotos) {
          await route.fulfill({
            status: 500,
            contentType: 'application/json',
            body: JSON.stringify({ error: 'forced failure' }),
          });
        } else {
          await route.continue();
        }
      }
    );

    await TestHelpers.goto(page);

    // THEN: error state with Try Again is shown (fails pre-fix: skeleton sticks)
    const errorState = page.locator('.error-state');
    await expect(errorState).toBeVisible({ timeout: 10000 });
    await expect(errorState.getByRole('button', { name: 'Try Again' })).toBeVisible();

    // WHEN: the backend is healthy again and the user retries
    failPhotos = false;
    await errorState.getByRole('button', { name: 'Try Again' }).click();
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: photos load
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);
  });
});
