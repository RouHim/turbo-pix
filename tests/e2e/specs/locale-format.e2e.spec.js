import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const CONFIG_DE = {
  default_locale: 'de',
  tile_url: null,
  face_recognition_enabled: false,
};

/**
 * The suite's server runs with the default `en` locale. Mocking only the
 * config response proves the locale actually reaches the UI without a second
 * server instance.
 */
test.describe('locale-driven date formatting', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.route('**/api/config', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(CONFIG_DE),
      });
    });
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('renders German dates on cards and in the viewer', async ({ page }) => {
    const firstCard = page.locator(TestHelpers.selectors.photoCardAny).first();
    await expect(firstCard.locator('.photo-card-meta')).toHaveText(/^\d{1,2}\. \p{L}+\.? \d{4}$/u);

    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);
    await expect(page.locator('[data-viewer-date]')).toHaveText(
      /^\d{1,2}\. \p{L}+\.? \d{4}, \d{2}:\d{2}$/u
    );
  });

  test('renders German month names in the mobile timeline dropdown', async ({ page }) => {
    await TestHelpers.setMobileViewport(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('#timeline-month-select')).toBeVisible({ timeout: 10000 });
    const months = await page.locator('#timeline-month-select option').allTextContents();
    expect(months).toContain('Januar');
    expect(months).toContain('Dezember');
  });
});
