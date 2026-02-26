import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Transcoding', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should not show transcode toast for h264 video', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);
    expect(videoCards.length).toBeGreaterThan(0);

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    await expect(page.locator('.transcode-toast')).not.toBeVisible({ timeout: 3000 });
    await expect(page.locator(TestHelpers.selectors.viewerVideo)).toBeVisible();
  });

  test('should show toast and poll for hevc video', async ({ page }) => {
    test.skip(
      true,
      'Requires HEVC test video (test_hevc.mp4) in test-e2e-data/photos/. Create with: ffmpeg -y -f lavfi -i color=c=blue:s=320x240:d=3 -vcodec libx265 test_hevc.mp4'
    );

    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);
    expect(videoCards.length).toBeGreaterThan(0);

    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    await expect(page.locator('.transcode-toast')).toBeVisible();
  });
});
