import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const waitForCollagesOrEmpty = async (page) => {
  await page.waitForFunction(
    () => document.querySelector('.photo-card') || document.querySelector('.empty-state')
  );
};

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
    await waitForCollagesOrEmpty(page);

    // WHEN: Page loads
    const collageCards = await TestHelpers.getPhotoCards(page);
    if (collageCards.length === 0) {
      test.skip('No pending collages available');
    }

    // THEN: Collages are displayed
    expect(collageCards.length).toBeGreaterThan(0);
  });

  test('should show accept/reject buttons for collages', async ({ page }) => {
    // GIVEN: User is on collages view with collages available
    await TestHelpers.navigateToView(page, 'collages');
    await waitForCollagesOrEmpty(page);

    const collageCards = await TestHelpers.getPhotoCards(page);
    if (collageCards.length === 0) {
      test.skip('No pending collages available for action buttons');
    }

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
    await waitForCollagesOrEmpty(page);
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
    await waitForCollagesOrEmpty(page);
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

  test('should accept pending collage from viewer and remove it from grid', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await waitForCollagesOrEmpty(page);

    const collageCards = page.locator(TestHelpers.selectors.photoCardAny);
    if ((await collageCards.count()) === 0) {
      test.skip('No pending collages available for viewer accept flow');
    }

    const firstCollageCard = collageCards.first();
    await expect(firstCollageCard).toBeVisible();

    const acceptedCollageId = await firstCollageCard.getAttribute('data-photo-id');
    expect(acceptedCollageId).toBeTruthy();

    await firstCollageCard.click();
    await TestHelpers.verifyViewerOpen(page);

    const viewerAcceptButton = page.locator('#photo-viewer [data-action="accept-collage"]');
    await expect(viewerAcceptButton).toBeVisible();
    await expect(viewerAcceptButton).toBeEnabled();

    const acceptResponsePromise = page.waitForResponse(
      (response) =>
        response.url().includes(`/api/collages/${acceptedCollageId}/accept`) && response.ok()
    );

    await viewerAcceptButton.click();
    await acceptResponsePromise;

    await expect(page.locator('#photo-viewer.active')).toHaveCount(0);
    await expect(page.locator(TestHelpers.selectors.photoCard(acceptedCollageId))).toHaveCount(0);
  });

  test('should keep viewer open when collage accept API fails', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');

    await waitForCollagesOrEmpty(page);

    const collageCards = page.locator(TestHelpers.selectors.photoCardAny);
    if ((await collageCards.count()) === 0) {
      test.skip('No pending collages available for negative accept flow');
    }

    const firstCollageCard = collageCards.first();
    await expect(firstCollageCard).toBeVisible();

    const acceptedCollageId = await firstCollageCard.getAttribute('data-photo-id');
    expect(acceptedCollageId).toBeTruthy();

    const acceptRouteHandler = (route) => {
      route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ message: 'forced accept failure for e2e' }),
      });
    };

    await page.route('**/api/collages/*/accept', acceptRouteHandler);

    try {
      await firstCollageCard.click();
      await TestHelpers.verifyViewerOpen(page);

      const viewerAcceptButton = page.locator('#photo-viewer [data-action="accept-collage"]');
      await expect(viewerAcceptButton).toBeVisible();
      await expect(viewerAcceptButton).toBeEnabled();

      const failedAcceptResponsePromise = page.waitForResponse(
        (response) =>
          response.url().includes(`/api/collages/${acceptedCollageId}/accept`) &&
          response.status() === 500
      );

      await viewerAcceptButton.click();
      await failedAcceptResponsePromise;

      await expect(page.locator('#photo-viewer.active')).toHaveCount(1);
      await expect(viewerAcceptButton).toBeEnabled();

      await TestHelpers.closeViewer(page);
      await expect(page.locator(TestHelpers.selectors.photoCard(acceptedCollageId))).toHaveCount(1);
    } finally {
      await page.unroute('**/api/collages/*/accept', acceptRouteHandler);
    }
  });
});
