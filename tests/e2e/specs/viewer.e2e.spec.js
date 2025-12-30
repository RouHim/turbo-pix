import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';
import { TestDataManager } from '../setup/test-data-manager.js';

test.describe('Photo Viewer', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should open viewer when clicking a photo', async ({ page }) => {
    // Get first photo card
    const photoCards = await TestHelpers.getPhotoCards(page);
    expect(photoCards.length).toBeGreaterThan(0);

    const firstCard = photoCards[0];
    await firstCard.click();

    // Verify viewer opened
    await TestHelpers.verifyViewerOpen(page);

    // Verify viewer image or video is loaded
    const hasImage = await TestHelpers.elementExists(page, TestHelpers.selectors.viewerImage);
    const hasVideo = await TestHelpers.elementExists(page, TestHelpers.selectors.viewerVideo);

    expect(hasImage || hasVideo).toBe(true);
  });

  test('should close viewer with close button', async ({ page }) => {
    // Open viewer
    const firstCard = (await TestHelpers.getPhotoCards(page))[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Click close button
    const closeBtn = page.locator('.viewer-close');
    await closeBtn.click();

    // Verify viewer closed
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).not.toHaveClass(/active/);
  });

  test('should close viewer with Escape key', async ({ page }) => {
    // Open viewer
    const firstCard = (await TestHelpers.getPhotoCards(page))[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Press Escape
    await TestHelpers.closeViewer(page);

    // Verify viewer closed
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).not.toHaveClass(/active/);
  });

  test('should close viewer by clicking overlay', async ({ page }) => {
    // Open viewer
    const firstCard = (await TestHelpers.getPhotoCards(page))[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Click overlay (outside the viewer content)
    const overlay = page.locator('.viewer-overlay');
    await overlay.click({ position: { x: 10, y: 10 } });

    // Verify viewer closed
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).not.toHaveClass(/active/);
  });

  test('should navigate to next photo with next button', async ({ page }) => {
    // Get photo count
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length < 2) {
      test.skip('Need at least 2 photos for this test');
    }

    // Open first photo
    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstPhotoHash = await TestHelpers.getCurrentPhotoHash(page);

    // Click next button
    const nextBtn = page.locator('.viewer-next');
    await nextBtn.click();

    // Wait for photo to change
    await page.waitForTimeout(500);

    const secondPhotoHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(secondPhotoHash).not.toBe(firstPhotoHash);
  });

  test('should navigate to previous photo with prev button', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length < 2) {
      test.skip('Need at least 2 photos for this test');
    }

    // Open second photo
    await photoCards[1].click();
    await TestHelpers.verifyViewerOpen(page);

    const secondPhotoHash = await TestHelpers.getCurrentPhotoHash(page);

    // Click previous button
    const prevBtn = page.locator('.viewer-prev');
    await prevBtn.click();

    // Wait for photo to change
    await page.waitForTimeout(500);

    const firstPhotoHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(firstPhotoHash).not.toBe(secondPhotoHash);
  });

  test('should navigate with arrow keys', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length < 2) {
      test.skip('Need at least 2 photos for this test');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstPhotoHash = await TestHelpers.getCurrentPhotoHash(page);

    // Navigate right with arrow key
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(500);

    const secondPhotoHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(secondPhotoHash).not.toBe(firstPhotoHash);

    // Navigate left with arrow key
    await page.keyboard.press('ArrowLeft');
    await page.waitForTimeout(500);

    const backToFirstHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(backToFirstHash).toBe(firstPhotoHash);
  });

  test('should update URL with photo hash', async ({ page }) => {
    const firstCard = (await TestHelpers.getPhotoCards(page))[0];
    const photoId = await firstCard.getAttribute('data-photo-id');

    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Verify URL contains photo hash
    expect(page.url()).toContain(`photo=${photoId}`);
  });

  test('should toggle favorite from viewer', async ({ page }) => {
    const firstCard = (await TestHelpers.getPhotoCards(page))[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Click favorite button
    const favoriteBtn = page.locator('.favorite-btn');
    await favoriteBtn.click();

    // Wait for toast (either "Added" or "Removed")
    const toast = page.locator(TestHelpers.selectors.toast);
    await expect(toast).toBeVisible({ timeout: 5000 });
    const toastText = await toast.textContent();
    expect(toastText).toMatch(/(Added|Removed).*favorite/i);
  });

  test('should toggle favorite with keyboard shortcut (f)', async ({ page }) => {
    const firstCard = (await TestHelpers.getPhotoCards(page))[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Press 'f' key
    await page.keyboard.press('f');

    // Wait for toast
    const toast = page.locator(TestHelpers.selectors.toast);
    await expect(toast).toBeVisible({ timeout: 5000 });
  });

  test('should download photo with download button', async ({ page }) => {
    const firstCard = (await TestHelpers.getPhotoCards(page))[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Setup download listener
    const downloadPromise = page.waitForEvent('download');

    // Click download button
    const downloadBtn = page.locator('.download-btn');
    await downloadBtn.click();

    // Wait for download to start
    const download = await downloadPromise;
    expect(download).toBeTruthy();

    // Verify toast message
    await TestHelpers.waitForToast(page, 'download started');
  });

  test('should show zoom controls for images', async ({ page }) => {
    // Find an image (not video)
    const photoCards = await TestHelpers.getPhotoCards(page);
    let imageCard = null;

    for (const card of photoCards) {
      const hasVideoIcon = await card.locator('[data-feather="video"]').count();
      if (hasVideoIcon === 0) {
        imageCard = card;
        break;
      }
    }

    if (!imageCard) {
      test.skip('No image found, only videos');
    }

    await imageCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Verify zoom controls are visible
    const zoomInBtn = page.locator('.zoom-in');
    const zoomOutBtn = page.locator('.zoom-out');
    const zoomFitBtn = page.locator('.zoom-fit');

    await expect(zoomInBtn).toBeVisible();
    await expect(zoomOutBtn).toBeVisible();
    await expect(zoomFitBtn).toBeVisible();
  });

  test('should zoom in and out on images', async ({ page }) => {
    // Find an image
    const photoCards = await TestHelpers.getPhotoCards(page);
    let imageCard = null;

    for (const card of photoCards) {
      const hasVideoIcon = await card.locator('[data-feather="video"]').count();
      if (hasVideoIcon === 0) {
        imageCard = card;
        break;
      }
    }

    if (!imageCard) {
      test.skip('No image found');
    }

    await imageCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Zoom in
    const zoomInBtn = page.locator('.zoom-in');
    await zoomInBtn.click();
    await page.waitForTimeout(300);

    // Zoom out
    const zoomOutBtn = page.locator('.zoom-out');
    await zoomOutBtn.click();
    await page.waitForTimeout(300);

    // Fit to screen
    const zoomFitBtn = page.locator('.zoom-fit');
    await zoomFitBtn.click();
    await page.waitForTimeout(300);

    // No errors should occur
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).toHaveClass(/active/);
  });

  test('should rotate image left', async ({ page }) => {
    // Find a non-RAW image
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    const firstCard = photoCards[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Click rotate left button
    const rotateLeftBtn = page.locator('.rotate-left-btn');
    const isDisabled = await rotateLeftBtn.isDisabled();

    if (!isDisabled) {
      await rotateLeftBtn.click();

      // Wait for rotation to complete (shows toast)
      const toast = page.locator(TestHelpers.selectors.toast);
      await expect(toast).toBeVisible({ timeout: 10000 });
      await expect(toast).toContainText(/rotat/i);
    } else {
      test.skip('Rotation disabled for this photo (RAW or video)');
    }
  });

  test('should rotate image right', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    const firstCard = photoCards[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Click rotate right button
    const rotateRightBtn = page.locator('.rotate-right-btn');
    const isDisabled = await rotateRightBtn.isDisabled();

    if (!isDisabled) {
      await rotateRightBtn.click();

      // Wait for rotation to complete
      const toast = page.locator(TestHelpers.selectors.toast);
      await expect(toast).toBeVisible({ timeout: 10000 });
      await expect(toast).toContainText(/rotat/i);
    } else {
      test.skip('Rotation disabled for this photo');
    }
  });

  test('should delete photo with confirmation', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    const initialCount = photoCards.length;

    if (initialCount === 0) {
      test.skip('No photos available');
    }

    const firstCard = photoCards[0];
    const photoId = await firstCard.getAttribute('data-photo-id');

    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Click delete button
    const deleteBtn = page.locator('.delete-photo-btn');
    await deleteBtn.click();

    // Confirm deletion in dialog
    page.on('dialog', async (dialog) => {
      expect(dialog.message()).toContain('delete');
      await dialog.accept();
    });

    // Wait for deletion to complete
    await page.waitForTimeout(2000);

    // Verify photo was removed from grid
    const photoStillExists = await TestHelpers.elementExists(page, `[data-photo-id="${photoId}"]`);
    expect(photoStillExists).toBe(false);
  });

  test('should toggle fullscreen mode', async ({ page }) => {
    const firstCard = (await TestHelpers.getPhotoCards(page))[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Click fullscreen button
    const fullscreenBtn = page.locator('.fullscreen-btn');
    await expect(fullscreenBtn).toBeVisible();
    await fullscreenBtn.click();

    // Note: Actual fullscreen API testing is tricky in headless mode
    // Just verify the button works without errors
    await page.waitForTimeout(500);
  });

  test('should show metadata panel when button clicked', async ({ page }) => {
    const firstCard = (await TestHelpers.getPhotoCards(page))[0];
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Check if metadata button exists
    const metadataBtn = page.locator('.metadata-btn');
    const metadataBtnExists = (await metadataBtn.count()) > 0;

    if (metadataBtnExists) {
      await metadataBtn.click();

      // Verify sidebar appears
      const sidebar = page.locator('.viewer-sidebar');
      await expect(sidebar).toBeVisible();
    }
  });

  test('should play video when opened', async ({ page }) => {
    // Find a video
    const photoCards = await TestHelpers.getPhotoCards(page);
    let videoCard = null;

    for (const card of photoCards) {
      const hasVideoIcon = await card.locator('[data-feather="video"]').count();
      if (hasVideoIcon > 0) {
        videoCard = card;
        break;
      }
    }

    if (!videoCard) {
      test.skip('No videos found');
    }

    await videoCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // Verify video element is visible
    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible();

    // Wait for video to load
    await page.waitForTimeout(1000);

    // Video should be playable
    const canPlay = await video.evaluate((v) => v.readyState >= 2);
    expect(canPlay).toBe(true);
  });
});
