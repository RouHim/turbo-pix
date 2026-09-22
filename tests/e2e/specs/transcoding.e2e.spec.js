import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

// The encoder hint is fed by the `x-turbopix-encoder` header of the stream run,
// and that header does not exist until the run has produced bytes: the server
// awaits them for up to 10 s (`FIRST_BYTES_TIMEOUT` in `video_stream.rs`) and a
// failed hardware attempt pays that attempt's own time before the software
// respawn. Playwright's default 5 s expect timeout sits below that gate, so a
// slow-but-successful run on a host with a hardware encoder reads as "hint not
// found" — an environment-dependent flake CI cannot surface.
const ENCODER_HINT_TIMEOUT = 20_000;

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

    // The native-first premise, asserted on the element's own source: the
    // viewer hands the file to the media element, so neither a transcode
    // request nor a MediaSource blob may appear. The conversion notice cannot
    // carry this guard — `verifyViewerOpen` resolves on the viewer's `.active`
    // class, which `open()` sets before the `?decision` fetch `displayPhoto`
    // issues has answered, and a notice only renders once `playStream` sets
    // `transcodeMessage` from that response. Playwright passes
    // `not.toBeVisible` immediately when the locator matches no node, so the old
    // assertion held before anything could appear; the MSE/stream path also
    // makes `#viewer-video` visible, so visibility does not distinguish it
    // either. Waiting for the source is waiting for the decision to have been
    // made and applied.
    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible({ timeout: 30_000 });
    await expect.poll(async () => video.getAttribute('src'), { timeout: 30_000 }).toBeTruthy();
    const src = await video.getAttribute('src');
    expect(src, 'a natively playable video must not be converted').not.toContain('transcode=true');
    expect(src, 'a natively playable video must not be streamed through a blob').not.toMatch(
      /^blob:/
    );
    expect(src).toContain(`/api/photos/${h264Photo.hash_sha256}/video`);

    // AND, the decision now made, no conversion notice is up
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('should stream and play hevc video while converting', async ({ page }) => {
    // The server-side conversion can take a while on slow CI.
    test.setTimeout(120_000);

    const hevcPhoto = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // A conversion notice is only shown while the video streams: a playthrough
    // earlier in the run may already have cached the whole file.
    await TestHelpers.clearCachedConversions(page, hevcPhoto.hash_sha256);

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
    // Generous on purpose: the fill wait below is the slow part on a small
    // runner, and it must report its own assertion rather than a test timeout.
    test.setTimeout(180_000);
    const hint = page.locator('[data-testid="viewer-encoder-hint"]');

    // GIVEN a video the client cannot play (the server converts it)
    const hevcPhoto = await findVideoByFilename(page, 'test_video_hevc.mp4');
    await TestHelpers.clearCachedConversions(page, hevcPhoto.hash_sha256);
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
    // encoder that is serving the playback. The timeout outwaits the server's
    // first-byte gate: the header this hint reads does not exist before the run
    // has produced bytes.
    await expect(hint).toHaveCount(1, { timeout: ENCODER_HINT_TIMEOUT });
    await expect(hint).toBeVisible({ timeout: ENCODER_HINT_TIMEOUT });
    await expect(hint).toHaveAttribute(
      'aria-label',
      /\((?:libx264|h264_nvenc|h264_vaapi|h264_qsv|h264_amf|h264_videotoolbox)\)$/,
      { timeout: ENCODER_HINT_TIMEOUT }
    );

    // AND the run is left to FINISH before the viewer moves on: a client that
    // navigates away aborts the stream, and the handler fills the cache only
    // from a run that completed (`outcome.is_ok(...)`) — an aborted run's
    // output is a prefix of the file, so it proves nothing about the rest. The
    // fact waited for here is the one the phase after the switch needs; the
    // 2 s clip ends on its own, so this is bounded by the fill, not playback.
    const decisionUrl = `/api/photos/${hevcPhoto.hash_sha256}/video?decision&client=h264-8%2Caac`;
    const cachedDelivery = async () => (await page.request.get(decisionUrl)).json();
    await expect.poll(cachedDelivery, { timeout: 90_000 }).toMatchObject({
      action: 'direct',
      cached: true,
      encoder: expect.any(String),
    });

    // WHEN the viewer moves to a natively playable video without being closed
    await page.evaluate(() => window.history.back());
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(h264Photo.hash_sha256);
    await expect(page.locator(TestHelpers.selectors.viewer)).toBeVisible();
    await expect(page.locator(TestHelpers.selectors.viewerVideo)).toBeVisible();

    // THEN the hint is gone: nothing is being video-encoded, and the previous
    // playback's hint must not carry over
    await expect(hint).toHaveCount(0);

    // WHEN the viewer returns to the converted video, now served from the
    // cached whole-file artifact instead of a stream.
    //
    // The delivery really is the cached file, and its decision names the
    // encoder — the carrier the viewer has to pass through for this phase to
    // show anything at all. The artifact was waited for above: the run that
    // produced it had to finish before the viewer could leave, so this state
    // is asserted rather than polled.
    expect(await cachedDelivery()).toMatchObject({
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

  test('should report the encoder that produced the conversion', async ({ page }) => {
    test.setTimeout(120_000);

    // GIVEN an AVI/mpeg4 source that always converts (Chromium cannot play it)
    const photo = await findVideoByFilename(page, 'test_video_legacy.avi');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);

    // WHEN the conversion is requested and completes
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    // `?transcode=true` is the whole-file escape hatch: it is the request that
    // claims the conversion slot and spawns the job. A bare byte request for a
    // stream delivery serves the original instead (nothing is claimed), so the
    // status endpoint below would answer 404 forever.
    const trigger = await page.request.get(`/api/photos/${photo.hash_sha256}/video?transcode=true`);
    expect(trigger.status()).toBe(202);
    await expect
      .poll(
        async () => {
          const response = await page.request.get(`/api/photos/${photo.hash_sha256}/video/status`);
          if (!response.ok()) return 'missing-status';
          const status = await response.json();
          return status.state;
        },
        { timeout: 90_000, intervals: [1000] }
      )
      .toBe('Completed');

    // THEN the status names a known encoder and never an alias or a flag blob
    const status = await (
      await page.request.get(`/api/photos/${photo.hash_sha256}/video/status`)
    ).json();
    expect([
      'libx264',
      'h264_nvenc',
      'h264_vaapi',
      'h264_qsv',
      'h264_amf',
      'h264_videotoolbox',
    ]).toContain(status.encoder);

    // AND playback of the produced artifact is unaffected: the artifact is
    // decoded, not merely present. A truncated conversion output — the exact
    // failure mode the whole-file encoder path can produce — is still served
    // and still renders a `<video>`, so it would pass a visibility-only
    // assertion while the element carries an `error` and no frame at all. The
    // cached artifact plays as a file, and the viewer only auto-plays a file
    // when its autoPlay setting is on (the MSE path self-plays), so enable it
    // before opening.
    await page.evaluate(() =>
      localStorage.setItem('viewSettings', JSON.stringify({ autoPlay: true }))
    );
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator(TestHelpers.selectors.photoCard(photo.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);
    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible();
    await page.waitForFunction(
      (hash) => {
        const el = document.querySelector('#viewer-video');
        // Anchored on the photo this element holds, so a stale source of an
        // earlier delivery cannot stand in for the artifact produced above.
        return (
          el &&
          el.dataset.photoHash === hash &&
          el.readyState >= 2 &&
          el.currentTime > 0 &&
          !el.error
        );
      },
      photo.hash_sha256,
      { timeout: 30_000 }
    );
  });
});

// The cold-cache premise every caller above asserts is a property of
// `clearCachedConversions`, so the guard itself is exercised here. A real
// conversion in flight cannot be timed deterministically from a spec (that is
// the whole problem: the fill is still alive when the wipe runs), so the fill
// is simulated: the status reports `InProgress` and the artifact lands after
// the wipe's own delete. No server and no browser are involved — the helper
// only ever reads `/video/status` and the cache directory.
//
// The cache tree the helper lists is resolved against the working directory
// (`test-e2e-data`), so a throwaway data directory is how the fixtures below
// give it a cache of their own: the run's real cache — and the server serving
// from it — is left untouched.
async function withThrowawayDataDir(run) {
  const root = await mkdtemp(path.join(tmpdir(), 'turbo-pix-data-'));
  const cwd = process.cwd();
  try {
    process.chdir(root);
    return await run();
  } finally {
    process.chdir(cwd);
    await rm(root, { recursive: true, force: true });
  }
}

test.describe('clearCachedConversions', () => {
  test('outlasts a conversion that publishes after the wipe', async () => {
    await withThrowawayDataDir(async () => {
      const hash = 'f'.repeat(64);
      const namespace = path.join('test-e2e-data', 'transcode-cache', 'transcoded');
      await mkdir(namespace, { recursive: true });
      const artifact = path.join(namespace, `${hash}_4096_1700000000000.mp4`);

      // The job began before the wipe: its artifact is not on disk yet, so a
      // delete-and-return wipe misses it and leaves a warm cache behind.
      let publishedAt = null;
      const publishing = (async () => {
        await delay(300);
        await writeFile(artifact, 'published by the conversion that was in flight');
        publishedAt = Date.now();
      })();
      let polls = 0;
      const page = {
        request: {
          get: async () => {
            polls += 1;
            return {
              ok: () => true,
              json: async () => ({ state: polls === 1 ? 'InProgress' : 'Completed' }),
            };
          },
        },
      };

      await TestHelpers.clearCachedConversions(page, hash);
      const returnedAt = Date.now();
      await publishing;

      // The premise every caller asserts: no artifact of the conversion the
      // wipe raced may survive the call.
      expect(
        existsSync(artifact),
        'the wipe must not leave the raced conversion artifact behind'
      ).toBe(false);
      // And it got there by outlasting the fill rather than by returning before
      // it: the fill published while the wipe was still running.
      expect(publishedAt, 'the in-flight conversion must have published').not.toBeNull();
      expect(
        publishedAt,
        'the wipe returned before the conversion it raced had published'
      ).toBeLessThanOrEqual(returnedAt);
    });
  });

  // The probe is the wipe's only evidence that no whole-file job claimed the
  // hash, so a probe that does not answer must not read as "nothing is
  // publishing": a caller would then assert `stream`/`remux` on a cache a
  // still-running conversion is about to fill.
  test('throws when the status probe never produces a response', async () => {
    const hash = 'e'.repeat(64);
    const page = {
      request: {
        get: async () => {
          throw new Error('socket hang up');
        },
      },
    };

    await expect(TestHelpers.clearCachedConversions(page, hash)).rejects.toThrow(
      /probe never produced a response/
    );
  });

  test('throws when the status probe cannot be answered from', async () => {
    const hash = 'd'.repeat(64);
    const page = {
      request: {
        get: async () => ({
          ok: () => false,
          status: () => 500,
        }),
      },
    };

    await expect(TestHelpers.clearCachedConversions(page, hash)).rejects.toThrow(
      /probe answered 500/
    );
  });

  // The flip side of the two above: the endpoint's 404 means "this hash has no
  // conversion", the one non-ok answer that IS evidence of an idle cache — it
  // must keep returning quietly rather than throw, and "quietly" spans the
  // settle window, not just the first probe: the first probe cannot already be
  // `CACHE_QUIET_MS` old, so a wipe that returned on it would leave a publish
  // that lands moments later unwaited.
  test('reads the endpoint 404 as no conversion and waits out the settle window', async () => {
    const hash = 'c'.repeat(64);
    await withThrowawayDataDir(async () => {
      // The namespace exists but holds nothing for the hash: the probe's 404
      // and the empty listing agree that the cache is idle.
      await mkdir(path.join('test-e2e-data', 'transcode-cache', 'transcoded'), { recursive: true });
      let probes = 0;
      const page = {
        request: {
          get: async () => {
            probes += 1;
            return { ok: () => false, status: () => 404 };
          },
        },
      };

      await TestHelpers.clearCachedConversions(page, hash);

      expect(
        probes,
        'the wipe must have polled past its first non-busy probe: one probe means it ' +
          'returned without waiting out the quiet window'
      ).toBeGreaterThan(1);
    });
  });

  // The namespace listing is the wipe's other observation, and it is evidence
  // of the same kind: a namespace that could not be listed says nothing about
  // the hash — ENOTDIR when the path is not a directory, EACCES/EIO when the
  // listing fails. Reading that as "nothing was cached" would let an artifact,
  // or an in-flight `{hash}_…tmp`, survive the wipe and warm the cache the
  // caller is about to assert cold.
  test('throws when a cache namespace cannot be listed', async () => {
    const hash = 'b'.repeat(64);
    const page = {
      request: {
        get: async () => ({ ok: () => false, status: () => 404 }),
      },
    };

    await withThrowawayDataDir(async () => {
      const namespace = path.join('test-e2e-data', 'transcode-cache', 'copied');
      await mkdir(path.dirname(namespace), { recursive: true });
      await writeFile(namespace, 'a file where a cache namespace is expected');

      // Premise of the fixture, and of the helper's own path resolution: the
      // namespace path the wipe will list is not a directory.
      await expect(readdir(namespace)).rejects.toMatchObject({ code: 'ENOTDIR' });

      await expect(TestHelpers.clearCachedConversions(page, hash)).rejects.toThrow(
        /copied cache namespace could not be listed \(ENOTDIR\)/
      );
    });
  });

  // The flip side: a namespace that is simply not there (nothing was cached
  // yet) IS an empty cache — the wipe must wait out its window and return, not
  // throw on the ENOENT.
  test('treats a missing namespace as an empty cache', async () => {
    const hash = 'a'.repeat(64);
    let probes = 0;
    const page = {
      request: {
        get: async () => {
          probes += 1;
          return { ok: () => false, status: () => 404 };
        },
      },
    };

    await withThrowawayDataDir(async () => {
      // No data directory at all: every namespace of the cache answers ENOENT.
      expect(
        existsSync(path.join('test-e2e-data', 'transcode-cache')),
        'the throwaway data directory must not hold a cache'
      ).toBe(false);

      await TestHelpers.clearCachedConversions(page, hash);
    });

    expect(
      probes,
      'the wipe must have polled past its first non-busy probe: one probe means it ' +
        'returned without waiting out the quiet window'
    ).toBeGreaterThan(1);
  });
});
