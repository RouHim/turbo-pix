import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

/**
 * Fixtures (see test-data/, generated with ffmpeg):
 *   test_video_long.mp4  20 s h264+aac progressive
 *   test_video_long.mkv  20 s h264+aac Matroska
 *   test_video_ac3.mp4   20 s h264 + AC-3
 *   test_video_hevc.mp4  2 s hevc
 */

async function findVideoByFilename(page, filename) {
  const response = await page.request.get('/api/photos?q=type:video&limit=200');
  expect(response.ok()).toBeTruthy();
  const data = await response.json();
  const photo = (data.photos || []).find((p) => p.filename === filename);
  expect(photo, `${filename} must be seeded and indexed`).toBeTruthy();
  return photo;
}

async function openVideo(page, photo) {
  await TestHelpers.navigateToView(page, 'videos');
  await TestHelpers.waitForPhotosToLoad(page);
  await page.locator(TestHelpers.selectors.photoCard(photo.hash_sha256)).click();
  await TestHelpers.verifyViewerOpen(page);
}

function videoHandle(page) {
  return page.locator(TestHelpers.selectors.viewerVideo);
}

test.describe('On-the-fly streaming playback', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('HEVC plays while converting: first frame within 5s, true duration', async ({ page }) => {
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // This test is about the *streaming* path, so it starts from a cold cache:
    // a playthrough earlier in the run would otherwise make it a direct play.
    await TestHelpers.clearCachedConversions(hevc.hash_sha256);
    await openVideo(page, hevc);

    const video = videoHandle(page);
    await expect(video).toBeVisible();
    const started = Date.now();
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 5000 }
    );
    expect(Date.now() - started).toBeLessThan(5000);

    const duration = await video.evaluate((el) => el.duration);
    expect(duration).toBeGreaterThan(1.5); // source duration is 2 s
    expect(duration).toBeLessThan(2.5);

    // The conversion notice must not survive the first frames.
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('Matroska h264 remuxes losslessly and seeks within 3s', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    await openVideo(page, mkv);

    const video = videoHandle(page);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );

    const seekStart = Date.now();
    await video.evaluate((el) => {
      el.currentTime = 15;
    });
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.currentTime >= 15 && el.readyState >= 2;
      },
      null,
      { timeout: 3000 }
    );
    expect(Date.now() - seekStart).toBeLessThan(3000);
  });

  test('AC-3 audio converts without re-encoding video', async ({ page }) => {
    const ac3 = await findVideoByFilename(page, 'test_video_ac3.mp4');
    // A cold cache: a playthrough fills the whole-file cache, and the second
    // open is then (by design) a direct play instead of an audio conversion.
    await TestHelpers.clearCachedConversions(ac3.hash_sha256);
    await openVideo(page, ac3);

    const response = await page.request.get(
      `/api/photos/${ac3.hash_sha256}/video?decision&client=h264-8,aac`
    );
    const decision = await response.json();
    expect(decision.action).toBe('stream');
    expect(decision.mode).toBe('audio');

    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );
  });

  test('h264 video still plays directly without any conversion notice', async ({ page }) => {
    const h264 = await findVideoByFilename(page, 'test_video.mp4');
    await openVideo(page, h264);
    const video = videoHandle(page);
    await expect(video).toBeVisible();
    const src = await video.getAttribute('src');
    expect(src).toContain(`/api/photos/${h264.hash_sha256}/video`);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('saturated conversions wait visibly and then start', async ({ page }) => {
    test.setTimeout(60_000);
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // Saturation is only observable while the decisions say "stream": a
    // playthrough earlier in the run may have cached the conversion.
    await TestHelpers.clearCachedConversions(hevc.hash_sha256);

    // Answer the first two stream requests with 503 + Retry-After, then let the
    // real request through: this is exactly what a full worker pool looks like.
    let refusals = 0;
    await page.route('**/video/stream*', async (route) => {
      if (refusals < 2) {
        refusals += 1;
        await route.fulfill({
          status: 503,
          headers: { 'retry-after': '1', 'content-type': 'application/json' },
          body: JSON.stringify({ error: 'no conversion slot available' }),
        });
        return;
      }
      await route.continue();
    });

    await openVideo(page, hevc);
    await expect(page.locator('.transcode-toast')).toContainText(
      'Waiting for a free conversion slot',
      {
        timeout: 10_000,
      }
    );
    // Waiting is not a dead end: the escape hatch stays reachable while the
    // pool is full (a permanently disabled pool would otherwise trap the user).
    await expect(page.locator('.transcode-toast [data-action="play-original"]')).toBeVisible();

    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );
    expect(refusals).toBe(2);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('a previously converted video starts without a blocking conversion', async ({ page }) => {
    test.setTimeout(120_000);
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // The first open must stream, so nothing may be cached for it yet.
    await TestHelpers.clearCachedConversions(hevc.hash_sha256);
    await openVideo(page, hevc);

    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );

    // A full playthrough fills the whole-file cache in the background (the
    // fixture is 2 s of video, so the conversion is quick); the same probe then
    // reports the conversion as a cached artifact.
    const decisionUrl = `/api/photos/${hevc.hash_sha256}/video?decision&client=h264-8,aac`;
    await expect
      .poll(
        async () => {
          const response = await page.request.get(decisionUrl);
          return (await response.json()).cached;
        },
        { timeout: 30_000, message: 'the playthrough must fill the conversion cache' }
      )
      .toBe(true);

    const decision = await (await page.request.get(decisionUrl)).json();
    expect(decision.action).toBe('direct');
    expect(decision.cached).toBe(true);

    // Reopen: playback starts natively from the cache, without a byte of
    // streaming and without a conversion notice.
    const requests = [];
    page.on('request', (request) => requests.push(request.url()));
    await TestHelpers.closeViewer(page);
    const started = Date.now();
    await page.locator(TestHelpers.selectors.photoCard(hevc.hash_sha256)).click();
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 2000 }
    );
    expect(Date.now() - started).toBeLessThan(2000);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
    expect(requests.filter((url) => url.includes('/video/stream'))).toEqual([]);
    expect(
      requests.some(
        (url) =>
          url.includes(`/api/photos/${hevc.hash_sha256}/video?`) && url.includes('transcode=true')
      ),
      'the cached conversion must be served as a file'
    ).toBe(true);
  });
});
