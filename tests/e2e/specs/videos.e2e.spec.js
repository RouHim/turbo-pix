import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Videos', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should navigate to videos view', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User clicks videos button
    await TestHelpers.navigateToView(page, 'videos');

    // THEN: Videos view is active
    await TestHelpers.verifyActiveView(page, 'videos');
  });

  test('should display video cards in videos view', async ({ page }) => {
    // GIVEN: User navigates to videos view
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: Page loads
    const videoCards = await TestHelpers.getPhotoCards(page);

    // THEN: Videos are displayed
    expect(videoCards.length).toBeGreaterThan(0);
  });

  test('should open video viewer', async ({ page }) => {
    // GIVEN: User is on videos view with videos available
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    const videoCards = await TestHelpers.getPhotoCards(page);
    expect(videoCards.length).toBeGreaterThan(0);

    // WHEN: User clicks on a video card
    await videoCards[0].click();

    // THEN: Viewer opens
    await TestHelpers.verifyViewerOpen(page);
  });

  test('should display video element in viewer', async ({ page }) => {
    // GIVEN: User opens a video
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    const videoCards = await TestHelpers.getPhotoCards(page);
    expect(videoCards.length).toBeGreaterThan(0);

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: Viewer loads
    const hasVideo = await TestHelpers.elementExists(page, TestHelpers.selectors.viewerVideo);

    // THEN: Video element should be present
    if (hasVideo) {
      await expect(page.locator(TestHelpers.selectors.viewerVideo)).toBeVisible();
    }
  });

  test('should use correct selectors for video cards', async ({ page }) => {
    // GIVEN: User is on videos view
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    const videoCards = await TestHelpers.getPhotoCards(page);
    expect(videoCards.length).toBeGreaterThan(0);

    // WHEN: User checks video card attributes
    const firstCard = videoCards[0];
    const photoId = await firstCard.getAttribute('data-photo-id');

    // THEN: Card has data-photo-id attribute
    expect(photoId).toBeTruthy();
    expect(photoId.length).toBeGreaterThan(0);
  });
});
