import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Keyboard Shortcuts', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should navigate photos with arrow keys', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(1);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);

    // WHEN: User presses right arrow
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(500);

    // THEN: Next photo is displayed
    const secondHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(secondHash).not.toBe(firstHash);

    // WHEN: User presses left arrow
    await page.keyboard.press('ArrowLeft');
    await page.waitForTimeout(500);

    // THEN: Previous photo is displayed
    const backToFirst = await TestHelpers.getCurrentPhotoHash(page);
    expect(backToFirst).toBe(firstHash);
  });

  test('should close viewer with Escape key', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User presses Escape
    await page.keyboard.press('Escape');

    // THEN: Viewer closes
    await page.waitForSelector(TestHelpers.selectors.viewer, { state: 'hidden' });
    await expect(page.locator(TestHelpers.selectors.viewer)).not.toBeVisible();
  });

  test('should submit search with Enter key', async ({ page }) => {
    // GIVEN: User has entered search text
    const searchInput = page.locator(TestHelpers.selectors.searchInput);
    await searchInput.fill('test query');

    // WHEN: User presses Enter while focused on input
    await searchInput.press('Enter');

    // THEN: Search is submitted and URL is updated
    await page.waitForTimeout(1000);

    const url = new URL(page.url());
    expect(url.searchParams.get('q')).toBe('test query');
  });

  test('should clear search with Escape key', async ({ page }) => {
    // GIVEN: User has entered search text
    await page.fill(TestHelpers.selectors.searchInput, 'test');

    // WHEN: User presses Escape
    await page.keyboard.press('Escape');

    // THEN: Search input is cleared
    const searchValue = await page.locator(TestHelpers.selectors.searchInput).inputValue();
    expect(searchValue).toBe('');
  });
});
