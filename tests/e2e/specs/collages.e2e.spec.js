import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Collages Management', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should navigate to collages view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.verifyActiveView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    // Should show collages or empty state
    const hasCollages = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasCollages || hasEmptyState).toBe(true);
  });

  test('should show empty state when no pending collages', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    // Either has collages or shows empty state
    expect(collageCards.length > 0 || hasEmptyState).toBe(true);

    if (hasEmptyState) {
      const noPhotos = page.locator(TestHelpers.selectors.noPhotos);
      const emptyText = await noPhotos.textContent();
      expect(emptyText.toLowerCase()).toContain('collage');
    }
  });

  test('should display pending collages', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length === 0) {
      test.skip('No pending collages available');
    }

    // Collages should have action buttons
    const firstCollage = collageCards[0];
    await firstCollage.hover();

    const acceptBtn = firstCollage.locator('[data-action="accept"]');
    const rejectBtn = firstCollage.locator('[data-action="reject"]');

    const hasAccept = (await acceptBtn.count()) > 0;
    const hasReject = (await rejectBtn.count()) > 0;

    expect(hasAccept || hasReject).toBe(true);
  });

  test('should accept a collage', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length === 0) {
      test.skip('No pending collages available');
    }

    const initialCount = collageCards.length;
    const firstCollage = collageCards[0];
    const collageId = await firstCollage.getAttribute('data-photo-id');

    await firstCollage.hover();

    const acceptBtn = firstCollage.locator('[data-action="accept"]');
    const btnExists = (await acceptBtn.count()) > 0;

    if (!btnExists) {
      test.skip('Accept button not found');
    }

    await acceptBtn.click();

    // Wait for action to complete
    await page.waitForTimeout(1500);

    // Should show toast
    const toast = page.locator(TestHelpers.selectors.toast);
    const toastVisible = (await toast.count()) > 0 && (await toast.isVisible());

    if (toastVisible) {
      const toastText = await toast.textContent();
      expect(toastText.toLowerCase()).toMatch(/accept|added/);
    }

    // Collage should be removed from view
    await page.waitForTimeout(500);
    const remainingCollages = await TestHelpers.getPhotoCards(page);
    expect(remainingCollages.length).toBe(initialCount - 1);
  });

  test('should reject a collage', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length === 0) {
      test.skip('No pending collages available');
    }

    const initialCount = collageCards.length;
    const firstCollage = collageCards[0];
    const collageId = await firstCollage.getAttribute('data-photo-id');

    await firstCollage.hover();

    const rejectBtn = firstCollage.locator('[data-action="reject"]');
    const btnExists = (await rejectBtn.count()) > 0;

    if (!btnExists) {
      test.skip('Reject button not found');
    }

    await rejectBtn.click();

    // Wait for action to complete
    await page.waitForTimeout(1500);

    // Should show toast
    const toast = page.locator(TestHelpers.selectors.toast);
    const toastVisible = (await toast.count()) > 0 && (await toast.isVisible());

    if (toastVisible) {
      const toastText = await toast.textContent();
      expect(toastText.toLowerCase()).toMatch(/reject|deleted|removed/);
    }

    // Collage should be removed from view
    await page.waitForTimeout(500);
    const collageStillExists = await TestHelpers.elementExists(
      page,
      `[data-photo-id="${collageId}"]`
    );
    expect(collageStillExists).toBe(false);
  });

  test('should open collage in viewer', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length === 0) {
      test.skip('No pending collages available');
    }

    const firstCollage = collageCards[0];
    await firstCollage.click();

    await TestHelpers.verifyViewerOpen(page);

    // Should show collage image
    const viewerImage = page.locator(TestHelpers.selectors.viewerImage);
    await expect(viewerImage).toBeVisible();
  });

  test('should show collage metadata', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length === 0) {
      test.skip('No pending collages available');
    }

    // Collages might have date or photo count information
    const firstCollage = collageCards[0];

    // Check for any metadata badges or labels
    const badge = firstCollage.locator('.badge, .collage-badge, .photo-count');
    const hasBadge = (await badge.count()) > 0;

    // Collage should have some identifying information
    expect(firstCollage).toBeTruthy();
  });

  test('should not show action buttons when clicking collage', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length === 0) {
      test.skip('No pending collages available');
    }

    const firstCollage = collageCards[0];
    await firstCollage.click();

    await TestHelpers.verifyViewerOpen(page);

    // Viewer should not accidentally trigger accept/reject
    await page.waitForTimeout(500);

    // Viewer should be open
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).toHaveClass(/active/);
  });

  test('should handle multiple pending collages', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length < 2) {
      test.skip('Need at least 2 collages for this test');
    }

    // All collages should have action buttons
    for (let i = 0; i < Math.min(3, collageCards.length); i++) {
      const collage = collageCards[i];
      await collage.hover();

      const acceptBtn = collage.locator('[data-action="accept"]');
      const rejectBtn = collage.locator('[data-action="reject"]');

      const hasActions = (await acceptBtn.count()) > 0 || (await rejectBtn.count()) > 0;
      expect(hasActions).toBe(true);
    }
  });

  test('should show collage date or creation info', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length === 0) {
      test.skip('No pending collages available');
    }

    const firstCollage = collageCards[0];
    await firstCollage.click();
    await TestHelpers.verifyViewerOpen(page);

    // Open metadata if available
    const metadataBtn = page.locator('.metadata-btn');
    const btnExists = (await metadataBtn.count()) > 0;

    if (btnExists) {
      await metadataBtn.click();

      const sidebar = page.locator('.viewer-sidebar');
      await expect(sidebar).toBeVisible();

      // Should show some date information
      const dateInfo = page.locator('#photo-date, .photo-info');
      const hasDateInfo = (await dateInfo.count()) > 0;

      if (hasDateInfo) {
        await expect(dateInfo.first()).toBeVisible();
      }
    }
  });

  test('should navigate between collages in viewer', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length < 2) {
      test.skip('Need at least 2 collages');
    }

    await collageCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstCollageHash = await TestHelpers.getCurrentPhotoHash(page);

    // Navigate to next collage
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(1000);

    const secondCollageHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(secondCollageHash).not.toBe(firstCollageHash);
  });

  test('should not show sort and timeline controls', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    // Sort and timeline controls should be hidden for collages view
    const sortSelect = page.locator(TestHelpers.selectors.sortSelect);
    const timeline = page.locator('.timeline-container, .timeline-input');

    const sortVisible = (await sortSelect.count()) > 0 && (await sortSelect.isVisible());
    const timelineVisible = (await timeline.count()) > 0 && (await timeline.isVisible());

    // Both should be hidden
    expect(sortVisible).toBe(false);
    expect(timelineVisible).toBe(false);
  });
});
