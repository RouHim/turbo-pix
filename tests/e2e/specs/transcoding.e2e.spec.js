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

  test('should hint which encoder serves a converting video, and none for direct play', async ({
    page,
  }) => {
    test.setTimeout(120_000);
    const hint = page.locator('[data-testid="viewer-encoder-hint"]');

    // GIVEN a video the client cannot play (the server converts it)
    const hevcPhoto = await findVideoByFilename(page, 'test_video_hevc.mp4');
    await TestHelpers.clearCachedConversions(hevcPhoto.hash_sha256);
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    // The switch below happens INSIDE the open viewer. The viewer is a modal
    // dialog, so a grid card is unreachable behind it — the app's own way of
    // moving between photos with the viewer open is its history navigation
    // (arrow keys / Back), which needs an entry to land on. Seed that entry
    // with the natively playable fixture, then open the HEVC video on top of
    // it: one Back press moves the viewer onto the h264 video.
    const h264Photo = await findVideoByFilename(page, 'test_video.mp4');
    await page.evaluate((hash) => {
      const url = new URL(window.location.href);
      url.searchParams.set('photo', hash);
      window.history.pushState(null, '', url);
    }, h264Photo.hash_sha256);

    await page.locator(TestHelpers.selectors.photoCard(hevcPhoto.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);

    // THEN exactly one hint is visible, and its accessible label names the
    // encoder that is serving the playback
    await expect(hint).toHaveCount(1);
    await expect(hint).toBeVisible();
    await expect(hint).toHaveAttribute(
      'aria-label',
      /\((?:libx264|h264_nvenc|h264_vaapi|h264_qsv|h264_amf|h264_videotoolbox)\)$/
    );

    // WHEN the viewer moves to a natively playable video without being closed
    await page.evaluate(() => window.history.back());
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(h264Photo.hash_sha256);
    await expect(page.locator(TestHelpers.selectors.viewer)).toBeVisible();
    await expect(page.locator(TestHelpers.selectors.viewerVideo)).toBeVisible();

    // THEN the hint is gone: nothing is being video-encoded, and the previous
    // playback's hint must not carry over
    await expect(hint).toHaveCount(0);

    // WHEN the viewer returns to the converted video, now served from the
    // cached whole-file artifact instead of a stream
    await expect
      .poll(
        async () =>
          (await page.request.get(`/api/photos/${hevcPhoto.hash_sha256}/video/status`)).json(),
        { timeout: 30_000 }
      )
      .toMatchObject({ state: 'Completed' });
    // The delivery really is the cached file, and its decision names the
    // encoder — the carrier the viewer has to pass through for this phase to
    // show anything at all.
    const cachedDecision = await (
      await page.request.get(
        `/api/photos/${hevcPhoto.hash_sha256}/video?decision&client=h264-8%2Caac`
      )
    ).json();
    expect(cachedDecision).toMatchObject({
      action: 'direct',
      cached: true,
      encoder: expect.any(String),
    });

    await page.evaluate(() => window.history.forward());
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(hevcPhoto.hash_sha256);

    // THEN the hint names the encoder again. A file delivery carries no
    // response header the client can read, so this value can only come from
    // the decision payload — the carrier the viewer has to pass through.
    await expect(hint).toHaveCount(1);
    await expect(hint).toHaveAttribute(
      'aria-label',
      /\((?:libx264|h264_nvenc|h264_vaapi|h264_qsv|h264_amf|h264_videotoolbox)\)$/
    );
  });
});
