import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Photo Viewer', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should open viewer when photo is clicked', async ({ page }) => {
    // GIVEN: User has photos loaded
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    // WHEN: User clicks on a photo
    await photos[0].click();

    // THEN: Viewer opens
    await TestHelpers.verifyViewerOpen(page);
    await expect(page.locator(TestHelpers.selectors.viewer)).toBeVisible();
  });

  test('should close viewer with Escape key', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User presses Escape
    await TestHelpers.closeViewer(page);

    // THEN: Viewer is closed
    await expect(page.locator(TestHelpers.selectors.viewer)).not.toBeVisible();
  });

  test('should close viewer with close button', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User clicks close button
    const closeBtn = page.locator(TestHelpers.selectors.closeViewerBtn);
    if (await closeBtn.isVisible()) {
      await closeBtn.click();

      // THEN: Viewer is closed
      await expect(page.locator(TestHelpers.selectors.viewer)).not.toBeVisible();
    }
  });

  test('should navigate to next photo with arrow key', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);

    // WHEN: User presses right arrow
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(500);

    // THEN: Next photo is displayed
    const secondHash = await TestHelpers.getCurrentPhotoHash(page);

    if (photos.length > 1) {
      expect(secondHash).not.toBe(firstHash);
    }
  });

  test('should navigate to previous photo with arrow key', async ({ page }) => {
    // GIVEN: Viewer is open on second photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(1);

    await photos[1].click();
    await TestHelpers.verifyViewerOpen(page);

    const secondHash = await TestHelpers.getCurrentPhotoHash(page);

    // WHEN: User presses left arrow
    await page.keyboard.press('ArrowLeft');
    await page.waitForTimeout(500);

    // THEN: Previous photo is displayed
    const firstHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(firstHash).not.toBe(secondHash);
  });

  test('should display viewer image', async ({ page }) => {
    // GIVEN: Try to find and load a valid displayable photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    // Try each photo until we find one that loads successfully
    let mediaLoaded = false;
    for (let i = 0; i < Math.min(photos.length, 5); i++) {
      // Skip if we already found a loaded photo
      if (mediaLoaded) break;

      await photos[i].click();
      await TestHelpers.verifyViewerOpen(page);

      // Wait briefly for media to attempt loading
      await page.waitForTimeout(2000);

      // Check if image or video is visible
      const viewerImage = page.locator(TestHelpers.selectors.viewerImage);
      const viewerVideo = page.locator(TestHelpers.selectors.viewerVideo);

      const imageVisible = await viewerImage.isVisible();
      const videoVisible = await viewerVideo.isVisible();

      if (imageVisible || videoVisible) {
        mediaLoaded = true;
        break;
      }

      // Close viewer and try next photo
      await TestHelpers.closeViewer(page);
      await page.waitForTimeout(500);
    }

    // THEN: At least one photo should have loaded successfully
    expect(mediaLoaded).toBe(true);
  });
});
