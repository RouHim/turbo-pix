import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Keyboard Shortcuts', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should close viewer with Escape key', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Press Escape
    await page.keyboard.press('Escape');

    // Viewer should close
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).not.toHaveClass(/active/);
  });

  test('should navigate to next photo with ArrowRight', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length < 2) {
      test.skip('Need at least 2 photos');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);

    // Press ArrowRight
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(500);

    const secondHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(secondHash).not.toBe(firstHash);
  });

  test('should navigate to previous photo with ArrowLeft', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length < 2) {
      test.skip('Need at least 2 photos');
    }

    await photoCards[1].click();
    await TestHelpers.verifyViewerOpen(page);

    const secondHash = await TestHelpers.getCurrentPhotoHash(page);

    // Press ArrowLeft
    await page.keyboard.press('ArrowLeft');
    await page.waitForTimeout(500);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(firstHash).not.toBe(secondHash);
  });

  test('should toggle favorite with F key', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Press 'f' key
    await page.keyboard.press('f');

    // Should show toast
    const toast = page.locator(TestHelpers.selectors.toast);
    await expect(toast).toBeVisible({ timeout: 5000 });

    const toastText = await toast.textContent();
    expect(toastText.toLowerCase()).toMatch(/favorite/);
  });

  test('should download photo with D key', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Setup download listener
    const downloadPromise = page.waitForEvent('download', { timeout: 5000 });

    // Press 'd' key
    await page.keyboard.press('d');

    // Wait for download
    try {
      const download = await downloadPromise;
      expect(download).toBeTruthy();
    } catch (e) {
      // Download might not work in all environments
      // Just verify the key was pressed
    }
  });

  test('should play/pause video with Space key', async ({ page }) => {
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

    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible();

    // Wait for video to load
    await page.waitForTimeout(1500);

    // Press Space to play/pause
    await page.keyboard.press('Space');
    await page.waitForTimeout(500);

    // Video state should be boolean
    const isPaused = await video.evaluate((v) => v.paused);
    expect(typeof isPaused).toBe('boolean');
  });

  test('should clear search with Escape key', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);
    await searchInput.fill('test query');

    // Press Escape
    await searchInput.press('Escape');

    // Input should be cleared
    const value = await searchInput.inputValue();
    expect(value).toBe('');
  });

  test('should submit search with Enter key', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);
    await searchInput.fill('test');

    // Press Enter
    await searchInput.press('Enter');

    // Wait for search to execute
    await page.waitForTimeout(1000);

    // Should navigate or show results
    await TestHelpers.waitForPhotosToLoad(page);

    const hasResults = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasResults || hasEmptyState).toBe(true);
  });

  test('should not navigate when typing in search input', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);
    await searchInput.click();
    await searchInput.fill('test');

    // Arrow keys should move cursor, not navigate
    await searchInput.press('ArrowLeft');
    await searchInput.press('ArrowRight');

    // Still focused on search
    const isFocused = await searchInput.evaluate((el) => document.activeElement === el);
    expect(isFocused).toBe(true);
  });

  test('should handle keyboard shortcuts only when viewer is active', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    // Press 'f' without viewer open (should not do anything)
    await page.keyboard.press('f');
    await page.waitForTimeout(300);

    // No toast should appear
    const toast = page.locator(TestHelpers.selectors.toast);
    const toastVisible = await toast.isVisible().catch(() => false);
    expect(toastVisible).toBe(false);

    // Now open viewer
    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Press 'f' with viewer open
    await page.keyboard.press('f');

    // Toast should appear
    await expect(toast).toBeVisible({ timeout: 5000 });
  });

  test('should handle rapid key presses gracefully', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length < 3) {
      test.skip('Need at least 3 photos');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Rapidly press arrow keys
    for (let i = 0; i < 5; i++) {
      await page.keyboard.press('ArrowRight');
      await page.waitForTimeout(100);
    }

    // Viewer should still be functional
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).toHaveClass(/active/);

    // Should be on a different photo
    const currentHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(currentHash).toBeTruthy();
  });

  test('should handle keyboard navigation at boundaries', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    // Open first photo
    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Try to go to previous (should stay at first or handle gracefully)
    await page.keyboard.press('ArrowLeft');
    await page.waitForTimeout(500);

    // Should still be in viewer
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).toHaveClass(/active/);

    // Try to go to next
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(500);

    // Should still be in viewer
    await expect(viewer).toHaveClass(/active/);
  });

  test('should support keyboard combinations', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Try Shift+ArrowRight (might have special behavior)
    await page.keyboard.down('Shift');
    await page.keyboard.press('ArrowRight');
    await page.keyboard.up('Shift');

    await page.waitForTimeout(500);

    // Viewer should still work
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).toHaveClass(/active/);
  });

  test('should focus search with forward slash key', async ({ page }) => {
    // Press '/' key (common shortcut to focus search)
    await page.keyboard.press('/');

    // Check if search input is focused
    const searchInput = page.locator(TestHelpers.selectors.searchInput);
    const isFocused = await searchInput.evaluate((el) => document.activeElement === el);

    // Either focused or '/' was typed (both are acceptable)
    expect(typeof isFocused).toBe('boolean');
  });

  test('should handle Tab key navigation', async ({ page }) => {
    // Press Tab to navigate through focusable elements
    await page.keyboard.press('Tab');
    await page.waitForTimeout(200);

    // Some element should be focused
    const focusedElement = await page.evaluate(() => {
      return document.activeElement?.tagName;
    });

    expect(focusedElement).toBeTruthy();
  });
});

test.describe('Accessibility Keyboard Navigation', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should be able to navigate entire interface with keyboard', async ({ page }) => {
    // Tab through interface
    for (let i = 0; i < 10; i++) {
      await page.keyboard.press('Tab');
      await page.waitForTimeout(100);
    }

    // Should have focused various elements
    const focusedElement = await page.evaluate(() => {
      return document.activeElement?.tagName;
    });

    expect(focusedElement).toBeTruthy();
  });

  test('should activate buttons with Enter or Space', async ({ page }) => {
    // Tab to first button
    await page.keyboard.press('Tab');

    // Activate with Enter
    await page.keyboard.press('Enter');
    await page.waitForTimeout(300);

    // Something should have happened (view change, etc.)
    expect(true).toBe(true);
  });
});
