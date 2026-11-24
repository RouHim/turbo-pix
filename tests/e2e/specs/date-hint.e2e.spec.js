import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const scrollMain = (page, top) =>
  page.evaluate((value) => {
    document.querySelector('[data-testid="main-content"]').scrollTo(0, value);
  }, top);

const hint = (page) => page.getByTestId('date-hint');

test.describe('Current-date hint', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
  });

  test('appears only after the content header leaves the glass bar', async ({ page }) => {
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);

    await expect(hint(page)).toHaveAttribute('data-visible', 'false');
    await expect(hint(page)).toHaveCSS('opacity', '0');

    await scrollMain(page, 700);
    await expect(hint(page)).toHaveAttribute('data-visible', 'true');
    await expect(hint(page)).toHaveCSS('opacity', '1');
    await expect(hint(page)).toContainText(/\d{4}/);

    const pill = await page.getByTestId('date-hint-pill').boundingBox();
    expect(Math.round(pill.y)).toBe(72);

    await scrollMain(page, 0);
    await expect(hint(page)).toHaveAttribute('data-visible', 'false');
    await expect(hint(page)).toHaveCSS('opacity', '0');
  });

  test('pauses the rail caption only while the hint is visible', async ({ page }) => {
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    const caption = page.getByTestId('date-caption');
    await expect(caption).toHaveCSS('visibility', 'visible');

    await scrollMain(page, 700);
    await expect(hint(page)).toHaveAttribute('data-visible', 'true');
    await expect(caption).toHaveCSS('visibility', 'hidden');

    await page.getByTestId('date-track').hover();
    await expect(hint(page)).toHaveAttribute('data-visible', 'false');
    await expect(caption).toHaveCSS('visibility', 'visible');
    await expect(page.getByTestId('date-bubble')).toBeVisible();
  });

  test('works on mobile and yields to the drawer and search', async ({ page }) => {
    await TestHelpers.setMobileViewport(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.getByTestId('date-rail')).toHaveCount(0);

    await scrollMain(page, 500);
    await expect(hint(page)).toHaveAttribute('data-visible', 'true');

    await page.getByTestId('sidebar-toggle').click();
    await expect(hint(page)).toHaveAttribute('data-visible', 'false');
    await page.getByTestId('sidebar-toggle').click();
    await expect(hint(page)).toHaveAttribute('data-visible', 'true');

    await page.locator('.mobile-search-btn').click();
    await expect(hint(page)).toHaveAttribute('data-visible', 'false');
    await page.locator('.mobile-search-btn').click();
    await expect(hint(page)).toHaveAttribute('data-visible', 'true');
  });

  test('stays out of non-date sorts', async ({ page }) => {
    await TestHelpers.goto(page, '/?sort=name_asc');
    await TestHelpers.waitForPhotosToLoad(page);
    await scrollMain(page, 700);
    await expect(hint(page)).toHaveAttribute('data-visible', 'false');
  });
});
