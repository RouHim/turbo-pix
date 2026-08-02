import { test, expect } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
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

    // CI retries re-run this test against the SAME server: a completed
    // transcode leaves its cached file behind, and the server then serves it
    // directly (200, no 202/toast) — the retry would fail deterministically
    // at the toast assertion. Clear the per-run cache so every attempt
    // re-exercises the 202 → poll → video flow — but ONLY when no transcode
    // job is in flight (a running job writes its temp file there; removing
    // it would make the job's final rename fail and the retry's poll end in
    // Failed).
    const hevcCardStatus = await page.request
      .get(`/api/photos/${hevcPhoto.hash_sha256}/video/status`)
      .catch(() => null);
    const statusData = hevcCardStatus?.ok() ? await hevcCardStatus.json() : null;
    const inFlight = statusData?.state === 'InProgress';
    if (!inFlight) {
      const transcodeCache = path.join('test-e2e-data', 'transcode-cache');
      await fs.promises.rm(transcodeCache, { recursive: true, force: true });
      await fs.promises.mkdir(transcodeCache, { recursive: true });
    }

    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

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
