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
});
