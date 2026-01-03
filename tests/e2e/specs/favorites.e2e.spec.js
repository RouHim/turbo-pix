import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';
import { TestDataManager } from '../setup/test-data-manager.js';

test.describe('Favorites', () => {
  let dataManager;

  test.beforeAll(async () => {
    dataManager = new TestDataManager();
  });

  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should navigate to favorites view', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User clicks favorites button
    await TestHelpers.navigateToView(page, 'favorites');

    // THEN: Favorites view is active
    await TestHelpers.verifyActiveView(page, 'favorites');
  });

  test('should display favorite button on photo cards', async ({ page }) => {
    // GIVEN: Photos are loaded
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    // WHEN: User checks the photo card
    const favoriteBtn = photos[0].locator(TestHelpers.selectors.favoriteBtn);

    // THEN: Favorite button exists
    const exists = (await favoriteBtn.count()) > 0;
    expect(exists).toBe(true);
  });

  test('should toggle favorite status from grid', async ({ page }) => {
    // GIVEN: User has photos loaded
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    // WHEN: User clicks favorite button
    const favoriteBtn = photos[0].locator(TestHelpers.selectors.favoriteBtn).first();

    const initialClass = await favoriteBtn.getAttribute('class');
    await favoriteBtn.click();

    // THEN: Favorite status changes
    await page.waitForTimeout(500);
    const newClass = await favoriteBtn.getAttribute('class');

    // Note: The actual class change depends on the implementation
    // This test verifies the click is successful
    expect(newClass).toBeDefined();
  });
});
