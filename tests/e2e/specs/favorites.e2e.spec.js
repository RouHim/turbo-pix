import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Favorites', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
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

    const photoId = await photos[0].getAttribute('data-photo-id');
    expect(photoId).toBeTruthy();

    const favoriteBtn = photos[0].locator(TestHelpers.selectors.favoriteBtn).first();
    const initialClass = await favoriteBtn.getAttribute('class');

    // WHEN: User clicks favorite button
    const favoriteResponse = page.waitForResponse(
      (r) => r.url().includes('/favorite') && r.request().method() === 'PUT' && r.status() === 200
    );
    await favoriteBtn.click();

    // THEN: Favorite status changes — await the API response: the optimistic
    // class flip alone would pass even if the favorite endpoint were broken
    await favoriteResponse;
    await expect.poll(() => favoriteBtn.getAttribute('class')).not.toBe(initialClass);

    // AND: Toggle back so shared server state is untouched
    const restoreResponse = page.waitForResponse(
      (r) => r.url().includes('/favorite') && r.request().method() === 'PUT' && r.status() === 200
    );
    await favoriteBtn.click();
    await restoreResponse;
    await expect.poll(() => favoriteBtn.getAttribute('class')).toBe(initialClass);
  });
});
