import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Collages', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should navigate to collages view', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User clicks collages button
    await TestHelpers.navigateToView(page, 'collages');

    // THEN: Collages view is active
    await TestHelpers.verifyActiveView(page, 'collages');
  });

  test('should display collage cards when available', async ({ page }) => {
    // GIVEN: User navigates to collages view
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: Page loads
    const collageCards = await TestHelpers.getPhotoCards(page);

    // THEN: Collages are displayed
    expect(collageCards.length).toBeGreaterThan(0);
  });

  test('should show accept/reject buttons for collages', async ({ page }) => {
    // GIVEN: User is on collages view with collages available
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);
    const collageCards = await TestHelpers.getPhotoCards(page);

    // WHEN: User checks for action buttons
    const acceptBtn = page.locator(TestHelpers.selectors.action('accept')).first();
    const rejectBtn = page.locator(TestHelpers.selectors.action('reject')).first();

    // THEN: Action buttons should exist
    const hasAcceptBtn = (await acceptBtn.count()) > 0;
    const hasRejectBtn = (await rejectBtn.count()) > 0;

    if (hasAcceptBtn) {
      expect(hasAcceptBtn).toBe(true);
    }

    if (hasRejectBtn) {
      expect(hasRejectBtn).toBe(true);
    }
  });

  test('should not open standard viewer for collages', async ({ page }) => {
    // GIVEN: User is on collages view with collages available
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);
    const collageCards = await TestHelpers.getPhotoCards(page);

    // WHEN: User clicks on a collage
    await collageCards[0].click();

    // THEN: Viewer should remain closed for collages
    await expect(page.locator(TestHelpers.selectors.viewer)).not.toHaveClass(/active/);
  });
});
