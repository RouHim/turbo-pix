import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Navigation', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  // The per-view specs (favorites, videos, collages, housekeeping) each own the
  // claim "this sidebar entry activates its view"; url-routing owns "a direct
  // load activates the all view". What none of them cover is the walk itself —
  // one session switching through every entry and back — so that is the only
  // case kept here.
  test('should switch views through the sidebar walk', async ({ page }) => {
    // GIVEN: User is on the homepage with photos loaded and the all view active
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);
    await TestHelpers.verifyActiveView(page, 'all');

    // WHEN: User walks the sidebar through every view and back to all
    // THEN: each click activates that view (navigateToView verifies it)
    for (const view of ['favorites', 'videos', 'collages', 'housekeeping', 'all']) {
      await TestHelpers.navigateToView(page, view);
    }

    // AND: the grid still shows photos after the walk, not a stranded empty view
    await TestHelpers.waitForPhotosToLoad(page);
    const photosAfterWalk = await TestHelpers.getPhotoCards(page);
    expect(photosAfterWalk.length).toBeGreaterThan(0);
  });
});
