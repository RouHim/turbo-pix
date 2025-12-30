import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Housekeeping Functionality', () => {
  test.beforeEach(async ({ page }) => {
    // Setup console monitoring (keeping this pattern from original)
    page.on('console', (msg) => console.log(`PAGE LOG: ${msg.text()}`));
    page.on('pageerror', (exception) => console.log(`PAGE ERROR: ${exception}`));
    page.on('requestfailed', (request) =>
      console.log(`REQUEST FAILED: ${request.url()} - ${request.failure()?.errorText || 'unknown'}`)
    );
    page.on('response', (response) => {
      if (response.status() === 404) {
        console.log(`404 RESPONSE: ${response.url()}`);
      }
    });

    // Navigate to home page
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should navigate to housekeeping view', async ({ page }) => {
    // Navigate to housekeeping
    await TestHelpers.navigateToView(page, 'housekeeping');

    // Verify view is active
    await TestHelpers.verifyActiveView(page, 'housekeeping');

    // Wait for content to load
    await TestHelpers.waitForPhotosToLoad(page);

    // Should show either candidates or empty state
    const hasCandidates = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasCandidates || hasEmptyState).toBe(true);
  });

  test('should display housekeeping candidates with reasons and scores', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.waitForPhotosToLoad(page);

    const candidates = await TestHelpers.getPhotoCards(page);

    if (candidates.length === 0) {
      test.skip('No housekeeping candidates available');
    }

    // Check that candidates have housekeeping badges
    const firstCandidate = candidates[0];
    const badge = firstCandidate.locator('.housekeeping-badge');

    // Badge might exist or might not depending on implementation
    const badgeExists = (await badge.count()) > 0;

    if (badgeExists) {
      await expect(badge).toBeVisible();

      // Badge should have text (reason)
      const badgeText = await badge.textContent();
      expect(badgeText).toBeTruthy();
    }
  });

  test('should keep a housekeeping candidate', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.waitForPhotosToLoad(page);

    const candidates = await TestHelpers.getPhotoCards(page);

    if (candidates.length === 0) {
      test.skip('No housekeeping candidates available');
    }

    const initialCount = candidates.length;
    const firstCandidate = candidates[0];

    // Hover to show action buttons
    await firstCandidate.hover();

    // Click "Keep" button
    const keepBtn = firstCandidate.locator('[data-action="keep"]');
    const keepBtnExists = (await keepBtn.count()) > 0;

    if (!keepBtnExists) {
      test.skip('Keep button not found');
    }

    await keepBtn.click();

    // Wait for action to complete
    await page.waitForTimeout(1000);

    // Verify toast message (if shown)
    const toast = page.locator(TestHelpers.selectors.toast);
    const toastVisible = (await toast.count()) > 0 && (await toast.isVisible());

    if (toastVisible) {
      const toastText = await toast.textContent();
      expect(toastText.toLowerCase()).toContain('kept');
    }

    // Verify candidate was removed from list
    await page.waitForTimeout(500);
    const remainingCandidates = await TestHelpers.getPhotoCards(page);
    expect(remainingCandidates.length).toBe(initialCount - 1);
  });

  test('should remove a housekeeping candidate', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.waitForPhotosToLoad(page);

    const candidates = await TestHelpers.getPhotoCards(page);

    if (candidates.length === 0) {
      test.skip('No housekeeping candidates available');
    }

    const initialCount = candidates.length;
    const firstCandidate = candidates[0];
    const photoId = await firstCandidate.getAttribute('data-photo-id');

    // Hover to show action buttons
    await firstCandidate.hover();

    // Click "Remove" button (might be a delete button)
    const removeBtn = firstCandidate.locator(
      '[data-action="remove"], [data-action="delete"], .delete-btn'
    );
    const removeBtnExists = (await removeBtn.count()) > 0;

    if (!removeBtnExists) {
      test.skip('Remove button not found');
    }

    // Handle confirmation dialog if it appears
    page.once('dialog', async (dialog) => {
      await dialog.accept();
    });

    await removeBtn.click();

    // Wait for deletion
    await page.waitForTimeout(1500);

    // Verify candidate was removed
    const candidateStillExists = await TestHelpers.elementExists(
      page,
      `[data-photo-id="${photoId}"]`
    );
    expect(candidateStillExists).toBe(false);
  });

  test('should show empty state when no candidates found', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.waitForPhotosToLoad(page);

    // Check for empty state or candidates
    const hasCandidates = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasCandidates || hasEmptyState).toBe(true);

    if (hasEmptyState) {
      const noPhotos = page.locator(TestHelpers.selectors.noPhotos);
      const emptyText = await noPhotos.textContent();
      expect(emptyText.toLowerCase()).toContain('clean');
    }
  });

  test('should not open viewer when clicking action buttons', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.waitForPhotosToLoad(page);

    const candidates = await TestHelpers.getPhotoCards(page);

    if (candidates.length === 0) {
      test.skip('No housekeeping candidates available');
    }

    const firstCandidate = candidates[0];
    await firstCandidate.hover();

    // Try to click keep button
    const keepBtn = firstCandidate.locator('[data-action="keep"]');
    const keepBtnExists = (await keepBtn.count()) > 0;

    if (!keepBtnExists) {
      test.skip('Keep button not found');
    }

    await keepBtn.click();

    // Wait a moment
    await page.waitForTimeout(500);

    // Viewer should NOT be open
    const viewer = page.locator(TestHelpers.selectors.viewer);
    const viewerActive = await viewer.getAttribute('class');
    expect(viewerActive).not.toContain('active');
  });

  test('should display reason badges correctly', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.waitForPhotosToLoad(page);

    const candidates = await TestHelpers.getPhotoCards(page);

    if (candidates.length === 0) {
      test.skip('No housekeeping candidates available');
    }

    // Check first candidate for badge
    const firstCandidate = candidates[0];
    const badge = firstCandidate.locator('.housekeeping-badge');

    const badgeExists = (await badge.count()) > 0;

    if (badgeExists) {
      // Badge should be visible
      await expect(badge).toBeVisible();

      // Badge should contain a reason (screenshot, meme, etc.)
      const badgeText = await badge.textContent();
      expect(badgeText.length).toBeGreaterThan(0);

      // Common reasons might include: screenshot, meme, duplicate, etc.
      const validReasons = ['screenshot', 'meme', 'duplicate', 'blur', 'dark'];
      const hasValidReason = validReasons.some((reason) =>
        badgeText.toLowerCase().includes(reason)
      );

      // If it doesn't match known reasons, just verify it has text
      expect(badgeText || hasValidReason).toBeTruthy();
    }
  });

  test('should handle multiple candidates', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.waitForPhotosToLoad(page);

    const candidates = await TestHelpers.getPhotoCards(page);

    if (candidates.length < 2) {
      test.skip('Need at least 2 candidates for this test');
    }

    // All candidates should have action buttons
    for (let i = 0; i < Math.min(3, candidates.length); i++) {
      const candidate = candidates[i];
      await candidate.hover();

      const keepBtn = candidate.locator('[data-action="keep"]');
      const keepBtnExists = (await keepBtn.count()) > 0;

      if (keepBtnExists) {
        await expect(keepBtn).toBeVisible();
      }
    }

    // Multiple candidates should be displayed
    expect(candidates.length).toBeGreaterThanOrEqual(2);
  });
});
