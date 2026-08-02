import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Critical User Paths', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should complete photo browsing and viewing workflow', async ({ page }) => {
    // GIVEN: User is on the homepage
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    // WHEN: User opens a photo
    const firstPhoto = photos[0];
    await firstPhoto.click();

    // THEN: Viewer opens successfully
    await TestHelpers.verifyViewerOpen(page);
    await expect(page.locator(TestHelpers.selectors.viewer)).toBeVisible();

    // WHEN: User navigates to next photo
    await page.keyboard.press('ArrowRight');

    // THEN: Next photo is displayed
    const currentHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(currentHash).toBeTruthy();

    // WHEN: User closes the viewer
    await TestHelpers.closeViewer(page);

    // THEN: User is back on the grid
    await expect(page.locator(TestHelpers.selectors.photoCardAny).first()).toBeVisible();
  });

  test('should complete search and view workflow', async ({ page }) => {
    // GIVEN: User is on the homepage
    await expect(page.locator(TestHelpers.selectors.searchInput)).toBeVisible();

    // WHEN: User performs a search
    await TestHelpers.performSearch(page, 'type:video');

    // THEN: Search results are displayed (seeded videos are always indexed)
    await page.waitForResponse(
      (response) =>
        response.url().includes('/api/photos') && response.url().includes('q=type%3Avideo')
    );
    await page.waitForSelector(`${TestHelpers.selectors.photoCardAny}, .empty-state`, {
      state: 'attached',
    });
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    // WHEN: User opens a search result
    await photos[0].click();

    // THEN: Viewer opens with the photo
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User clears the search
    await TestHelpers.closeViewer(page);
    await TestHelpers.clearSearch(page);

    // THEN: All photos are shown again
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should complete video discovery and playback workflow', async ({ page }) => {
    // GIVEN: User navigates to videos view
    const videosResponse = page.waitForResponse(
      (response) =>
        response.url().includes('/api/photos') && response.url().includes('q=type%3Avideo')
    );
    await TestHelpers.navigateToView(page, 'videos');
    await videosResponse;
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: Video cards are loaded
    const videoCards = await TestHelpers.getPhotoCards(page);
    expect(videoCards.length).toBeGreaterThan(0);

    // THEN: Videos are displayed
    expect(videoCards.length).toBeGreaterThan(0);

    // WHEN: User opens a video
    await videoCards[0].click();

    // THEN: Video viewer opens
    await TestHelpers.verifyViewerOpen(page);

    // THEN: Video element is present (seeded videos are always indexed)
    await expect(page.locator(TestHelpers.selectors.viewerVideo)).toBeVisible();

    // WHEN: User closes the viewer
    await TestHelpers.closeViewer(page);

    // THEN: User is back on videos grid
    await expect(page.locator(TestHelpers.selectors.photoCardAny).first()).toBeVisible();
  });
});
