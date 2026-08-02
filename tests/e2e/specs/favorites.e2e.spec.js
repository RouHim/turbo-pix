import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';
import { TestDataManager } from '../setup/test-data-manager.js';

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

  test('should un-favorite from viewer without dropping the next card', async ({ page }) => {
    // Regression: the viewer's toggleFavorite dispatched the grid event
    // BEFORE writing photos[currentIndex]; the synchronous handler splices
    // the SHARED array, so the write landed on the shifted slot and the next
    // card vanished from the Favorites grid (round 17 reorder, fixed in 18).
    // GIVEN: two favorite photos (absolute PUTs — a retry must not toggle
    // already-favorited photos back off)
    const dataManager = new TestDataManager();
    const cards = await TestHelpers.getPhotoCards(page);
    expect(cards.length).toBeGreaterThanOrEqual(2);
    const firstId = await cards[0].getAttribute('data-photo-id');
    const secondId = await cards[1].getAttribute('data-photo-id');
    expect(firstId).toBeTruthy();
    expect(secondId).toBeTruthy();

    await dataManager.addToFavorites(firstId);
    await dataManager.addToFavorites(secondId);
    // Reload so the grid reflects the seeded favorites before navigating
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: In the favorites view, open the first card and un-favorite it
    // from the viewer (click the card BY HASH — sort order must not matter)
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator(TestHelpers.selectors.photoCard(firstId)).click();
    await TestHelpers.verifyViewerOpen(page);

    const unFavResponse = page.waitForResponse(
      (r) => r.url().includes('/favorite') && r.request().method() === 'PUT' && r.status() === 200
    );
    // Scope to the viewer: the grid cards carry the same .favorite-btn class
    await page.locator('#photo-viewer .favorite-btn').click();
    await unFavResponse;

    // THEN: the un-favorited card is removed from the favorites grid AND the
    // next card is still present (the round-18 regression assertion)
    await expect(page.locator(TestHelpers.selectors.photoCard(firstId))).toHaveCount(0);
    await expect(page.locator(TestHelpers.selectors.photoCard(secondId))).toHaveCount(1);

    // AND: restore shared server state — re-favorite the photo (absolute PUT)
    await dataManager.addToFavorites(firstId);
  });
});
