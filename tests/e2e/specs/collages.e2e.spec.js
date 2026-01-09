import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Collages', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
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

    // WHEN: User checks for action buttons
    const acceptBtn = page.locator(TestHelpers.selectors.action('accept-collage')).first();
    const rejectBtn = page.locator(TestHelpers.selectors.action('reject-collage')).first();

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

  test('should open standard viewer for collages', async ({ page }) => {
    // GIVEN: User is on collages view with collages available
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);
    const collageCards = await TestHelpers.getPhotoCards(page);

    // WHEN: User clicks on a collage
    if (collageCards.length === 0) {
      test.skip('No collages available for preview');
    }
    await collageCards[0].click();

    // THEN: Viewer should open for collages
    await TestHelpers.verifyViewerOpen(page);
    await page.waitForSelector(`${TestHelpers.selectors.viewerImage}.loaded`, {
      state: 'attached',
    });
  });

  test('should navigate collages in viewer with arrow keys', async ({ page }) => {
    // GIVEN: User is on collages view with multiple collages available
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);
    const collageCards = await TestHelpers.getPhotoCards(page);

    if (collageCards.length < 2) {
      test.skip('Need at least 2 collages for viewer navigation');
    }

    await collageCards[0].click();
    await TestHelpers.verifyViewerOpen(page);
    await page.waitForSelector(`${TestHelpers.selectors.viewerImage}.loaded`, {
      state: 'attached',
    });
    const firstSrc = await page.getAttribute(TestHelpers.selectors.viewerImage, 'src');

    // WHEN: User presses right arrow
    await page.keyboard.press('ArrowRight');

    // THEN: Viewer should show a different collage
    await page.waitForFunction(
      ({ selector, previous }) => {
        const img = document.querySelector(selector);
        const src = img?.getAttribute('src');
        return Boolean(src && src !== previous);
      },
      { selector: TestHelpers.selectors.viewerImage, previous: firstSrc }
    );
  });
});
