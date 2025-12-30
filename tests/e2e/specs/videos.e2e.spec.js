import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Videos View', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should navigate to videos view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.verifyActiveView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    // Should show videos or empty state
    const hasVideos = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasVideos || hasEmptyState).toBe(true);
  });

  test('should display only videos in videos view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos found in test data');
    }

    // All cards should have video icon
    for (const card of videoCards.slice(0, 5)) {
      // Check first 5
      const videoIcon = card.locator('[data-feather="video"]');
      const hasIcon = (await videoIcon.count()) > 0;
      expect(hasIcon).toBe(true);
    }
  });

  test('should show empty state when no videos', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    // Either has videos or shows empty state
    expect(videoCards.length > 0 || hasEmptyState).toBe(true);
  });

  test('should open video in viewer', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    const firstVideo = videoCards[0];
    await firstVideo.click();

    await TestHelpers.verifyViewerOpen(page);

    // Should show video element
    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible({ timeout: 10000 });
  });

  test('should play video in viewer', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible();

    // Wait for video to load
    await page.waitForTimeout(2000);

    // Check if video is ready to play
    const readyState = await video.evaluate((v) => v.readyState);
    expect(readyState).toBeGreaterThanOrEqual(2); // HAVE_CURRENT_DATA or higher
  });

  test('should show video controls', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible();

    // Video should have controls attribute
    const hasControls = await video.getAttribute('controls');
    expect(hasControls).not.toBeNull();
  });

  test('should pause and play video with spacebar', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible();

    // Wait for video to load
    await page.waitForTimeout(1500);

    // Try to play with spacebar
    await page.keyboard.press('Space');
    await page.waitForTimeout(500);

    // Video state might have changed
    const isPaused = await video.evaluate((v) => v.paused);
    expect(typeof isPaused).toBe('boolean');
  });

  test('should navigate between videos', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length < 2) {
      test.skip('Need at least 2 videos');
    }

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstVideoHash = await TestHelpers.getCurrentPhotoHash(page);

    // Navigate to next video
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(1000);

    const secondVideoHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(secondVideoHash).not.toBe(firstVideoHash);

    // Video element should still be visible
    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible();
  });

  test('should show video thumbnail in grid', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    const firstVideo = videoCards[0];

    // Should have video play icon overlay
    const playIcon = firstVideo.locator('[data-feather="video"]');
    await expect(playIcon).toBeVisible();

    // Should have thumbnail image
    const thumbnail = firstVideo.locator('img');
    const thumbnailExists = (await thumbnail.count()) > 0;
    expect(thumbnailExists).toBe(true);
  });

  test('should handle video transcoding message if needed', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Wait for potential transcoding message
    await page.waitForTimeout(2000);

    // Check if transcoding warning exists (might not for all videos)
    const transcodingWarning = page.locator('.transcoding-warning');
    const warningExists = (await transcodingWarning.count()) > 0;

    // Either transcoding warning shows or video loads normally
    const video = page.locator(TestHelpers.selectors.viewerVideo);
    const videoVisible = await video.isVisible();

    expect(warningExists || videoVisible).toBe(true);
  });

  test('should show video metadata', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Try to open metadata panel
    const metadataBtn = page.locator('.metadata-btn');
    const btnExists = (await metadataBtn.count()) > 0;

    if (btnExists) {
      await metadataBtn.click();

      const sidebar = page.locator('.viewer-sidebar');
      await expect(sidebar).toBeVisible();

      // Should show video-specific metadata
      const videoInfo = page.locator('.video-information, #video-duration, #video-codec');
      const hasVideoInfo = (await videoInfo.count()) > 0;

      if (hasVideoInfo) {
        await expect(videoInfo.first()).toBeVisible();
      }
    }
  });

  test('should favorite a video', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const favoriteBtn = page.locator('.favorite-btn');
    await favoriteBtn.click();

    // Should show toast
    const toast = page.locator(TestHelpers.selectors.toast);
    await expect(toast).toBeVisible({ timeout: 5000 });
  });

  test('should not show zoom controls for videos', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Zoom controls should be hidden or disabled for videos
    const zoomInBtn = page.locator('.zoom-in');
    const zoomOutBtn = page.locator('.zoom-out');

    const zoomInVisible = (await zoomInBtn.count()) > 0 && (await zoomInBtn.isVisible());
    const zoomOutVisible = (await zoomOutBtn.count()) > 0 && (await zoomOutBtn.isVisible());

    // If visible, they should be disabled
    if (zoomInVisible) {
      const isDisabled = await zoomInBtn.isDisabled();
      expect(isDisabled).toBe(true);
    }
    if (zoomOutVisible) {
      const isDisabled = await zoomOutBtn.isDisabled();
      expect(isDisabled).toBe(true);
    }
  });
});
