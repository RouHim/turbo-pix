import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

/**
 * Native-first video playback & server-driven playback decisions.
 *
 * The global-setup always seeds `test_video.mp4` (h264, 8-bit, mp4) and
 * `test_video_hevc.mp4` (hevc, 8-bit, mp4), so both fixtures are deterministic
 * here — no test.skip() fallback needed. Hashes are resolved through the API
 * (by filename) rather than relying on card sort order.
 */

async function findVideoByFilename(page, filename) {
  const response = await page.request.get('/api/photos?q=type:video&limit=100');
  expect(response.ok()).toBeTruthy();
  const data = await response.json();
  const photo = (data.photos || []).find((p) => p.filename === filename);
  expect(photo, `${filename} must be seeded and indexed`).toBeTruthy();
  return photo;
}

test.describe('Native-first video playback', () => {
  test('h264 video streams the original without a transcode', async ({ page }) => {
    const h264 = await findVideoByFilename(page, 'test_video.mp4');

    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await TestHelpers.navigateToView(page, 'videos');

    // GIVEN a seeded h264 video card, WHEN opened
    const card = page.locator(TestHelpers.selectors.photoCard(h264.hash_sha256));
    await expect(card).toBeVisible();
    await card.click();
    await TestHelpers.verifyViewerOpen(page);

    // THEN the video element plays natively
    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible({ timeout: 30_000 });

    // AND the src is the original file URL, never a transcode URL
    const src = await video.getAttribute('src');
    expect(src, 'direct-play src must not request a transcode').not.toContain('transcode=true');
    expect(src).toContain(`/api/photos/${h264.hash_sha256}/video`);

    // AND no transcode toast is shown (native-first means no conversion)
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('server-driven decision endpoint picks direct vs transcode', async ({ page }) => {
    const h264 = await findVideoByFilename(page, 'test_video.mp4');
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');

    // GIVEN a client that can only play 8-bit h264, the ?decision probe
    // MUST report direct for h264 and transcode for hevc (the server owns the
    // codec+container decision from the capability record + declared codecs).
    const direct = await page.request.get(
      `/api/photos/${h264.hash_sha256}/video?decision&client=h264-8`
    );
    expect(direct.ok()).toBeTruthy();
    const directJson = await direct.json();
    expect(directJson.action).toBe('direct');
    expect(directJson.url).toBe(`/api/photos/${h264.hash_sha256}/video`);

    const transcode = await page.request.get(
      `/api/photos/${hevc.hash_sha256}/video?decision&client=h264-8`
    );
    expect(transcode.ok()).toBeTruthy();
    const transcodeJson = await transcode.json();
    expect(transcodeJson.action).toBe('transcode');
    expect(transcodeJson.url).toBe(`/api/photos/${hevc.hash_sha256}/video?transcode=true`);
  });
});
