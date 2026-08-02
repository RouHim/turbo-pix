import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Viewer Swipe Gestures', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await TestHelpers.setMobileViewport(page);
  });

  test('swipe left navigates to next photo', async ({ page }) => {
    // GIVEN: Viewer is open on the first photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(1);

    const firstHash = await photos[0].getAttribute('data-photo-id');
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const currentHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(currentHash).toBe(firstHash);

    // WHEN: User swipes left
    await TestHelpers.swipeLeft(page);

    // THEN: Viewer shows the next photo (URL hash updates asynchronously — poll)
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(firstHash);
  });

  test('swipe right navigates to previous photo', async ({ page }) => {
    // GIVEN: Viewer is open on the first photo, then navigated to the second
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(1);

    const firstHash = await photos[0].getAttribute('data-photo-id');
    await page.locator('.photo-card').first().scrollIntoViewIfNeeded();
    await page.locator('.photo-card').first().click();
    await TestHelpers.verifyViewerOpen(page);

    // Navigate to second photo via swipe left
    await TestHelpers.swipeLeft(page);
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(firstHash);

    // WHEN: User swipes right
    await TestHelpers.swipeRight(page);

    // THEN: Viewer shows the previous (first) photo again
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(firstHash);
  });

  test('swipe down past threshold dismisses viewer', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User swipes down past the 150px dismiss threshold
    await TestHelpers.swipeDown(page, { distance: 200 });

    // THEN: Viewer is dismissed (no .active class)
    await expect
      .poll(async () => (await page.locator('#photo-viewer').getAttribute('class')) ?? '')
      .not.toContain('active');
  });

  test('short swipe snaps back to same photo', async ({ page }) => {
    // GIVEN: Viewer is open on a photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(1);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const originalHash = await TestHelpers.getCurrentPhotoHash(page);

    // WHEN: User performs a slow short swipe (40px at 50ms/step → velocity ~0.08 px/ms, below 0.3 threshold)
    await TestHelpers.swipeLeft(page, { distance: 40, stepDelay: 50 });

    // THEN: Viewer stays on the same photo
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(originalHash);
    await TestHelpers.verifyViewerOpen(page);
  });

  test('keyboard navigation still works after swipe module added', async ({ page }) => {
    // GIVEN: Viewer is open on the first photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(1);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);

    // WHEN: User presses ArrowRight
    await page.keyboard.press('ArrowRight');

    // THEN: Next photo is displayed
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(firstHash);

    // WHEN: User presses ArrowLeft
    await page.keyboard.press('ArrowLeft');

    // THEN: Previous photo is displayed again
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(firstHash);
  });

  test('click navigation still works', async ({ page }) => {
    // GIVEN: Viewer is open on the first photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(1);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);

    // WHEN: User clicks the next button
    const nextBtn = page.locator('.viewer-next');
    if (await nextBtn.isVisible()) {
      await nextBtn.click();

      // THEN: Next photo is displayed
      await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(firstHash);

      // WHEN: User clicks the prev button
      const prevBtn = page.locator('.viewer-prev');
      if (await prevBtn.isVisible()) {
        await prevBtn.click();

        // THEN: Back to first photo
        await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(firstHash);
      }
    }

    // WHEN: User clicks the close button
    const closeBtn = page.locator('.viewer-close');
    if (await closeBtn.isVisible()) {
      await closeBtn.click();

      // THEN: Viewer is closed
      await expect
        .poll(async () => (await page.locator('#photo-viewer').getAttribute('class')) ?? '')
        .not.toContain('active');
    }
  });
});
