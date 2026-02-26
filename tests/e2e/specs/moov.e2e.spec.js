import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('MOOV Atom Fix — Video Playback', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should load gallery with videos available', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User navigates to videos view
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: Video cards are present
    const videoCards = await TestHelpers.getPhotoCards(page);
    expect(videoCards.length).toBeGreaterThan(0);
  });

  // This test requires a MOOV-at-end test video to be present in the test-data
  // directory and indexed by TurboPix. Since the E2E global-setup uses whatever
  // is in test-data/, and standard test videos already have MOOV at start,
  // this test is skipped until a dedicated MOOV-at-end fixture is added.
  //
  // To enable:
  // 1. Create a MOOV-at-end video:
  //    ffmpeg -y -f lavfi -i "color=c=blue:s=320x240:d=3" -c:v libx265 -crf 28 test-data/moov_at_end.mp4
  // 2. Let TurboPix scan and fix the MOOV atom during indexing
  // 3. Remove the test.skip() call below
  // 4. The test verifies the fixed video plays quickly (< 5s to first frame)
  test.skip('should play MOOV-fixed video without delay', async ({ page }) => {
    // GIVEN: A video that originally had MOOV at end has been scanned and fixed
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    const videoCards = await TestHelpers.getPhotoCards(page);
    expect(videoCards.length).toBeGreaterThan(0);

    // WHEN: User opens the video
    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // THEN: Video element loads and can play within a reasonable time
    const videoElement = page.locator(TestHelpers.selectors.viewerVideo);
    const hasVideo = await TestHelpers.elementExists(page, TestHelpers.selectors.viewerVideo);

    if (hasVideo) {
      await expect(videoElement).toBeVisible();

      // Wait for the video to be ready to play (readyState >= 2 means HAVE_CURRENT_DATA)
      await page.waitForFunction(
        (selector) => {
          const video = document.querySelector(selector);
          return video && video.readyState >= 2;
        },
        TestHelpers.selectors.viewerVideo,
        { timeout: 5000 }
      );
    }
  });
});
