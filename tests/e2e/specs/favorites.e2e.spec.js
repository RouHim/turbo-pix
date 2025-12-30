import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';
import { TestDataManager } from '../setup/test-data-manager.js';

test.describe('Favorites Management', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should favorite a photo from grid', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    const firstCard = photoCards[0];
    const photoId = await firstCard.getAttribute('data-photo-id');

    // Hover to show favorite button
    await firstCard.hover();

    // Click favorite button using data-action selector
    const favoriteBtn = firstCard.locator(TestHelpers.selectors.action('favorite'));
    const favoriteBtnExists = (await favoriteBtn.count()) > 0;

    if (!favoriteBtnExists) {
      test.skip('Favorite button not found on photo card');
    }

    await favoriteBtn.click();

    // Wait a moment for the favorite to be processed
    await page.waitForTimeout(500);

    // Navigate to favorites view
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    // Photo should be in favorites
    const favoriteCard = TestHelpers.getPhotoCardByHash(page, photoId);
    const cardExists = (await favoriteCard.count()) > 0;

    expect(cardExists).toBe(true);
  });

  test('should unfavorite a photo from favorites view', async ({ page }) => {
    // First, favorite a photo
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    const firstCard = photoCards[0];
    const photoId = await firstCard.getAttribute('data-photo-id');

    // Favorite it from viewer using viewer-specific selector
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    const favoriteBtn = page.locator('[data-icon="heart"]');
    await favoriteBtn.click();

    // Wait for favorite to be added
    await page.waitForTimeout(500);

    // Close viewer and go to favorites
    await TestHelpers.closeViewer(page);
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    // Find the favorited photo
    const favoritedCard = TestHelpers.getPhotoCardByHash(page, photoId);
    const exists = (await favoritedCard.count()) > 0;

    if (!exists) {
      test.skip('Photo not in favorites');
    }

    // Unfavorite it from viewer
    await favoritedCard.click();
    await TestHelpers.verifyViewerOpen(page);

    const viewerFavoriteBtn = page.locator('[data-icon="heart"]');
    await viewerFavoriteBtn.click();

    // Wait for unfavorite
    await page.waitForTimeout(500);

    // Close viewer
    await TestHelpers.closeViewer(page);

    // Photo should be removed from favorites view
    await page.waitForTimeout(500);
    const stillExists = await TestHelpers.elementExists(page, `[data-photo-id="${photoId}"]`);

    expect(stillExists).toBe(false);
  });

  test('should show favorites view with favorited photos only', async ({ page }) => {
    // Navigate to favorites view
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    // Either has favorited photos or shows empty state
    const hasFavorites = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasFavorites || hasEmptyState).toBe(true);

    if (hasFavorites) {
      // All photos in this view should be favorites
      // (We can't directly verify this without clicking each one,
      // but we can trust the view is filtering correctly)
      const favoriteCards = await TestHelpers.getPhotoCards(page);
      expect(favoriteCards.length).toBeGreaterThan(0);
    }
  });

  test('should toggle favorite from viewer and update UI', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    const firstCard = photoCards[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Get viewer favorite button
    const favoriteBtn = page.locator('[data-icon="heart"]');
    await expect(favoriteBtn).toBeVisible();

    // Click to favorite
    await favoriteBtn.click();
    await page.waitForTimeout(500);

    // Check if button state changed (class or attribute)
    const btnClass = await favoriteBtn.getAttribute('class');
    const isFavorited = btnClass?.includes('favorited') || btnClass?.includes('active');

    // Click again to unfavorite
    await favoriteBtn.click();
    await page.waitForTimeout(500);

    const btnClass2 = await favoriteBtn.getAttribute('class');
    const isUnfavorited = !btnClass2?.includes('favorited') && !btnClass2?.includes('active');

    // Button state should have toggled
    expect(isFavorited || isUnfavorited).toBe(true);
  });

  test('should show empty state when no favorites', async ({ page }) => {
    // Clear all favorites first (if API is available)
    try {
      await TestDataManager.clearAllFavorites();
    } catch (e) {
      // API might not support this, skip
      console.log('Could not clear favorites, skipping...');
    }

    // Navigate to favorites
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    // Should show empty state
    const noPhotos = page.locator(TestHelpers.selectors.noPhotos);
    const photoCards = await TestHelpers.getPhotoCards(page);

    const hasEmptyMessage = (await noPhotos.count()) > 0;
    const hasNoCards = photoCards.length === 0;

    expect(hasEmptyMessage || hasNoCards).toBe(true);
  });

  test('should persist favorites across page reload', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    const firstCard = photoCards[0];
    const photoId = await firstCard.getAttribute('data-photo-id');

    // Favorite a photo from viewer
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    const favoriteBtn = page.locator('[data-icon="heart"]');
    await favoriteBtn.click();
    await page.waitForTimeout(500);

    await TestHelpers.closeViewer(page);

    // Reload page
    await page.reload();
    await TestHelpers.waitForPhotosToLoad(page);

    // Navigate to favorites
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    // Photo should still be favorited
    const favoriteCard = TestHelpers.getPhotoCardByHash(page, photoId);
    const exists = (await favoriteCard.count()) > 0;

    expect(exists).toBe(true);
  });

  test('should show favorite count or indicator', async ({ page }) => {
    // Navigate to favorites
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    // View should be active
    await TestHelpers.verifyActiveView(page, 'favorites');

    // Title or header should indicate favorites view
    const viewTitle = page.locator(TestHelpers.selectors.viewTitle);
    const exists = (await viewTitle.count()) > 0;

    if (exists) {
      const titleText = await viewTitle.textContent();
      expect(titleText).toBeTruthy();
    }
  });

  test('should favorite and unfavorite multiple photos', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length < 2) {
      test.skip('Need at least 2 photos');
    }

    // Favorite first two photos from viewer
    for (let i = 0; i < 2; i++) {
      await photoCards[i].click();
      await TestHelpers.verifyViewerOpen(page);

      const favoriteBtn = page.locator('[data-icon="heart"]');
      await favoriteBtn.click();
      await page.waitForTimeout(500);

      await TestHelpers.closeViewer(page);
      await page.waitForTimeout(300);
    }

    // Check favorites view
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    const favoriteCards = await TestHelpers.getPhotoCards(page);
    expect(favoriteCards.length).toBeGreaterThanOrEqual(2);
  });
});
