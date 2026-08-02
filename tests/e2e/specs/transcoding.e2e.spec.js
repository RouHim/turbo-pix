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
    // The server-side transcode + 2s status poll can take a while on slow CI.
    test.setTimeout(120_000);

    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    // Target the HEVC card explicitly: cards sort by taken_at, and the HEVC
    // fixture is pinned to an old date so it never displaces test_video.mp4
    // as the first card (which the h264 test above relies on).
    const photosResponse = await page.request.get('/api/photos?q=type:video&limit=100');
    expect(photosResponse.ok()).toBeTruthy();
    const photosData = await photosResponse.json();
    const hevcPhoto = (photosData.photos || []).find(
      (photo) => photo.filename === 'test_video_hevc.mp4'
    );
    expect(hevcPhoto, 'HEVC test video (test_video_hevc.mp4) must be seeded').toBeTruthy();

    const hevcCard = page.locator(TestHelpers.selectors.photoCard(hevcPhoto.hash_sha256));
    await expect(hevcCard).toBeVisible();

    // WHEN: User opens the HEVC video (Chromium cannot play HEVC natively)
    await hevcCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // THEN: The transcode toast appears while the server converts the video
    await expect(page.locator('.transcode-toast')).toBeVisible();

    // AND: The poll flow completes — the toast spinner is replaced by the
    // transcoded video once polling reports 'Completed'.
    await expect(page.locator(TestHelpers.selectors.viewerVideo)).toBeVisible({
      timeout: 90_000,
    });
  });
});
