import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

async function findVideoByFilename(page, filename) {
  const response = await page.request.get('/api/photos?q=type:video&limit=200');
  expect(response.ok()).toBeTruthy();
  const data = await response.json();
  const photo = (data.photos || []).find((p) => p.filename === filename);
  expect(photo, `${filename} must be seeded and indexed`).toBeTruthy();
  return photo;
}

test.describe('Transcoding', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should not show transcode toast for h264 video', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    // Target the fixture by name: the videos view orders by date (then hash),
    // so "the first card" is not this file once more video fixtures are seeded.
    const h264Photo = await findVideoByFilename(page, 'test_video.mp4');
    await page.locator(TestHelpers.selectors.photoCard(h264Photo.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);

    await expect(page.locator('.transcode-toast')).not.toBeVisible({ timeout: 3000 });
    await expect(page.locator(TestHelpers.selectors.viewerVideo)).toBeVisible();
  });

  test('should stream and play hevc video while converting', async ({ page }) => {
    // The server-side conversion can take a while on slow CI.
    test.setTimeout(120_000);

    const hevcPhoto = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // A conversion notice is only shown while the video streams: a playthrough
    // earlier in the run may already have cached the whole file.
    await TestHelpers.clearCachedConversions(hevcPhoto.hash_sha256);

    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const hevcCard = page.locator(TestHelpers.selectors.photoCard(hevcPhoto.hash_sha256));
    await expect(hevcCard).toBeVisible();

    // WHEN: User opens the HEVC video (Chromium cannot play HEVC natively, so
    // the decision selects a streamed conversion)
    await hevcCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // THEN: the conversion notice appears while the stream starts
    await expect(page.locator('.transcode-toast')).toBeVisible();

    // AND: playback starts before the conversion finishes — the element plays
    // the streamed chunks (a MediaSource blob), not a complete file
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.src.startsWith('blob:') && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 90_000 }
    );

    // AND: the notice clears once frames are playing
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });
});
