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

  test('video box stops above the action bar (no native-control collision)', async ({ page }) => {
    const h264 = await findVideoByFilename(page, 'test_video.mp4');

    // Pin the 1920x1080 desktop viewport: the premise below (and the
    // viewport-height guard) only holds at this size, so it must not silently
    // inherit whatever playwright.config defaults to.
    await TestHelpers.setDesktopViewport(page);
    expect(page.viewportSize()).toEqual({ width: 1920, height: 1080 });

    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await TestHelpers.navigateToView(page, 'videos');

    // GIVEN the 1920x1080 fixture is open in the 1920x1080 desktop viewport —
    // height-bound, so its box would reach the viewport bottom
    const card = page.locator(TestHelpers.selectors.photoCard(h264.hash_sha256));
    await card.click();
    await TestHelpers.verifyViewerOpen(page);
    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible({ timeout: 30_000 });

    // WHEN measuring the media box against the action bar
    // (after the video's own dimensions resolve — before metadata arrives the
    // element lays out at Chromium's 300x150 default and the box never reaches
    // the bar, which would make the assertion below pass vacuously)
    await page.waitForFunction(() => document.querySelector('#viewer-video').videoWidth > 0);
    const geometry = await page.evaluate(() => {
      const video = document.querySelector('#viewer-video').getBoundingClientRect();
      return {
        videoBottom: video.bottom,
        videoHeight: video.height,
        controlsTop: document.querySelector('.viewer-controls').getBoundingClientRect().top,
        viewportHeight: window.innerHeight,
      };
    });

    // Premise: the fixture is height-bound, i.e. it fills the height the media
    // box offers instead of sitting small in the middle of it
    expect(geometry.videoHeight).toBeGreaterThan(geometry.viewportHeight * 0.6);

    // THEN the box ends at or above the bar's top edge: Chromium paints the
    // native control strip inside the video's own bottom edge, so any overlap
    // buries the scrubber under the toolbar
    expect(geometry.videoBottom).toBeLessThanOrEqual(geometry.controlsTop);
    expect(geometry.videoBottom).toBeLessThan(geometry.viewportHeight);
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
