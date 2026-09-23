import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { TestHelpers } from '../setup/test-helpers.js';

/**
 * Fixtures (see test-data/, generated with ffmpeg):
 *   test_video_long.mkv     20 s h264+aac Matroska
 *   test_video_ac3.mp4      20 s h264 + AC-3
 *   test_video_hevc.mp4     2 s hevc
 *   test_video_10bit.mp4    10 s h264 High 10 (yuv420p10le)
 *   test_video_noaudio.mp4  10 s h264, no audio track
 *   test_video_multitrack.mp4 20 s h264 + aac + ac3, seeded progressive
 *   test_video_moov_end.mp4 20 s h264+aac with moov at the end
 *   test_video_legacy.avi   10 s mpeg4 + mp3
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

/**
 * A delivery the browser cannot decode: a real fragmented MP4 whose track does
 * not match the SourceBuffer's declared type (HEVC bytes into the H.264/AAC
 * buffer a Matroska remux is typed with). Chromium reports that on the
 * SourceBuffer and on the element (`MEDIA_ERR_SRC_NOT_SUPPORTED`), which is the
 * DECODE failure the viewer's ladder answers by climbing one rung at once —
 * `msePlayer` reports it untagged for exactly that reason.
 *
 * Deliberately not a body that merely stops early: Chromium silently ignores
 * bytes it cannot parse at all (measured: appending `'not-mp4'` or Matroska
 * bytes to a `video/mp4` buffer fires `updateend` and never `error`), so such a
 * delivery only ever ends as a truncated run. That is a `lostRun` — the viewer
 * replays it on its own rung — and it is a different signal from the one the
 * ladder is for.
 */
const UNDECODABLE_MP4 = readFileSync(
  new URL('../../../test-data/test_video_hevc.mp4', import.meta.url)
);

/**
 * Answer the stream requests for the modes named in `failModes` with a delivery
 * the browser cannot decode (`UNDECODABLE_MP4`) — the server answered, the
 * delivery is unusable, which is exactly what a remux the client cannot
 * actually play looks like on the wire — and record every mode the app asked
 * for, in request order. The modes not named keep hitting the real server, so a
 * successful rung still plays.
 */
async function failStreamModes(page, failModes) {
  const requestedModes = [];
  await page.route('**/video/stream*', async (route) => {
    const mode = new URL(route.request().url()).searchParams.get('mode');
    requestedModes.push(mode);
    if (failModes.includes(mode)) {
      await route.fulfill({
        status: 200,
        headers: { 'content-type': 'video/mp4' },
        body: UNDECODABLE_MP4,
      });
      return;
    }
    await route.continue();
  });
  return requestedModes;
}

/**
 * The byte length of a valid prefix of a fragmented-MP4 delivery: whole
 * top-level boxes only (`ftyp`, `moov`, `moof` and `mdat` are all
 * size-prefixed). A cut inside a box hands Chromium a partial box, which is a
 * parse failure — a DECODE error on the element, the signal the ladder still
 * answers — where the run under test is one that merely stopped delivering.
 *
 * `targetBytes` bounds the prefix; the run's initialization segment
 * (`ftyp` + `moov`) is always kept whole, so a target of 0 is exactly the
 * delivery that was killed before its first fragment: the run attaches and
 * buffers no coded frame at all.
 */
function mp4PrefixLength(body, targetBytes) {
  let offset = 0;
  let initSegmentEnd = 0;
  while (offset + 8 <= body.length) {
    const size = body.readUInt32BE(offset);
    if (size < 8 || offset + size > body.length) break;
    const type = body.toString('latin1', offset + 4, offset + 8);
    if (initSegmentEnd > 0 && offset + size > targetBytes) break;
    offset += size;
    if (type === 'moov') initSegmentEnd = offset;
  }
  return offset;
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
    await TestHelpers.clearCachedConversions(page, hevc.hash_sha256);
    await openVideo(page, hevc);

    const video = videoHandle(page);
    await expect(video).toBeVisible();
    // The 5 s bound in the test name is the wait's own timeout: the poll can
    // only resolve inside that window, so a separate elapsed-time check could
    // only misreport a wait that already succeeded.
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 5000 }
    );

    const duration = await video.evaluate((el) => el.duration);
    expect(duration).toBeGreaterThan(1.5); // source duration is 2 s
    expect(duration).toBeLessThan(2.5);

    // The conversion notice must not survive the first frames.
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('Matroska h264 remuxes losslessly and seeks within 3s', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    // A cold cache: the playthrough below fills the `remux/` sidecar, after
    // which the same probe legitimately answers `direct`/`cached` and the
    // viewer plays a file instead of streaming (and a Playwright retry would
    // hit exactly that artifact from the first attempt). Probe *before*
    // playing, so the answer is the first-play one rather than a race against
    // the background fill.
    await TestHelpers.clearCachedConversions(page, mkv.hash_sha256);
    const response = await page.request.get(
      `/api/photos/${mkv.hash_sha256}/video?decision&client=h264-8,aac`
    );
    expect(response.ok()).toBeTruthy();
    const decision = await response.json();
    expect(decision.action).toBe('stream');
    expect(decision.mode).toBe('remux');

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

    // The 3 s bound in the test name is the wait's own timeout below.
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
  });

  test('AC-3 audio converts without re-encoding video', async ({ page }) => {
    const ac3 = await findVideoByFilename(page, 'test_video_ac3.mp4');
    // A cold cache: a playthrough fills the cache (video copied, audio
    // converted), after which the same probe legitimately answers
    // `direct`/`cached`. Probe *before* playing so the answer is the first-play
    // one rather than a race against the background fill.
    await TestHelpers.clearCachedConversions(page, ac3.hash_sha256);
    const response = await page.request.get(
      `/api/photos/${ac3.hash_sha256}/video?decision&client=h264-8,aac`
    );
    const decision = await response.json();
    expect(decision.action).toBe('stream');
    expect(decision.mode).toBe('audio');

    await openVideo(page, ac3);
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
    // A direct source only auto-plays when the viewer's autoPlay setting is on
    // (the MSE path always self-plays), so enable it before opening; without it
    // `currentTime > 0` below would only ever be reached by a user gesture.
    await page.evaluate(() =>
      localStorage.setItem('viewSettings', JSON.stringify({ autoPlay: true }))
    );
    await openVideo(page, h264);
    const video = videoHandle(page);
    await expect(video).toBeVisible();
    const src = await video.getAttribute('src');
    expect(src).toContain(`/api/photos/${h264.hash_sha256}/video`);
    // Containment alone is satisfied by `?transcode=true` as well, so the
    // direct claim needs its own negative: a converted source is a different
    // URL, and the point of this test is that this fixture never takes it.
    expect(src, 'direct-play src must not request a transcode').not.toContain('transcode=true');

    // The src only says which URL was asked for; the delivery itself is proven
    // by decoded frames. A direct request that yields no frame at all (a 404,
    // an undecodable body) must fail here instead of passing on the URL alone.
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('saturated conversions wait visibly and then start', async ({ page }) => {
    test.setTimeout(60_000);
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // Saturation is only observable while the decisions say "stream": a
    // playthrough earlier in the run may have cached the conversion.
    await TestHelpers.clearCachedConversions(page, hevc.hash_sha256);

    // Answer the first two stream requests with 503 + Retry-After, then let the
    // real request through: this is exactly what a full worker pool looks like.
    // Every request is counted and stamped with the moment it arrived, refused
    // or not: the assertions at the end are about the client's pacing, and a
    // counter that only grew while refusing could not tell "two refused attempts
    // and one paced retry" from a client that ignores `Retry-After` and hammers
    // the endpoint — the refusal budget fixes the count at three either way, so
    // only the gap between the second refusal and the request that follows it
    // separates a paced client from an immediate one.
    //
    // The hint is deliberately above the viewer's own floor: PhotoViewer clamps
    // `Retry-After` into [1500 ms, 10 s], so a hint of one second would be
    // raised to 1500 ms and an unpaced client — which waits that same local
    // default — would be indistinguishable in the gap as well. Three seconds
    // survives the clamp, so the wait the client actually performed is the
    // server's number and shows up in the timestamps.
    let streamRequests = 0;
    let refusals = 0;
    const streamRequestTimes = [];
    await page.route('**/video/stream*', async (route) => {
      streamRequests += 1;
      streamRequestTimes.push(Date.now());
      if (refusals < 2) {
        refusals += 1;
        await route.fulfill({
          status: 503,
          headers: { 'retry-after': '3', 'content-type': 'application/json' },
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
    // Exactly two refused attempts plus the single retry that follows them: the
    // refused run waits out its `Retry-After` pacing and re-requests once, so
    // any further stream request fails here.
    expect(streamRequests).toBe(3);
    // The count alone proves nothing about the pacing: the route refuses
    // exactly twice, so an immediate hammering client also lands on three. The
    // gap between the second refusal and the third request is the client's own
    // wait, and `Retry-After: 3` must show up in it — an unpaced retry would
    // leave a gap of milliseconds and fail here. The 500 ms of slack absorbs
    // scheduling and route-interception overhead, and the test's own 60 s
    // timeout keeps the wait bounded.
    const pacedRetryGapMs = streamRequestTimes[2] - streamRequestTimes[1];
    expect(pacedRetryGapMs, 'the retry must wait out Retry-After: 3').toBeGreaterThanOrEqual(2500);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('a permanently disabled conversion pool is named, not waited for', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    // A disabled pool is only observable while the decisions say "stream": a
    // cached artifact is served as a file, with no stream request to refuse.
    await TestHelpers.clearCachedConversions(page, mkv.hash_sha256);

    // The server's own answer for `TURBO_PIX_MAX_TRANSCODES=0`: the same 503 a
    // saturated pool sends, with the refusal body that names the pool as
    // disabled (src/handlers_video.rs, `StreamStartError::Disabled`). No slot
    // will ever free, so waiting for one re-requests forever and leaves the
    // notice naming a pool that does not exist. The refusal is a failure the
    // viewer ends on: the ladder climbs its bounded rungs and the escape hatch
    // is offered.
    const requestedModes = [];
    await page.route('**/video/stream*', async (route) => {
      requestedModes.push(new URL(route.request().url()).searchParams.get('mode'));
      await route.fulfill({
        status: 503,
        headers: { 'retry-after': '5', 'content-type': 'application/json' },
        body: JSON.stringify({ error: 'conversion disabled' }),
      });
    });

    await openVideo(page, mkv);
    await expect(page.locator('.transcode-toast')).toContainText('Video conversion failed', {
      timeout: 30_000,
    });
    await expect(page.locator('.transcode-toast [data-action="play-original"]')).toBeVisible();
    // One request per rung of the ladder, and no more: a client that read this
    // refusal as saturation would keep re-requesting the same mode for as long
    // as the viewer stays open on the photo.
    expect(requestedModes).toEqual(['remux', 'audio', 'transcode']);
  });

  test('a previously converted video starts without a blocking conversion', async ({ page }) => {
    test.setTimeout(120_000);
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // The first open must stream, so nothing may be cached for it yet.
    await TestHelpers.clearCachedConversions(page, hevc.hash_sha256);
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
    // streaming and without a conversion notice. A `direct` source only
    // auto-plays when the viewer's autoPlay setting is on (the MSE path always
    // self-plays), so enable it before measuring.
    await page.evaluate(() =>
      localStorage.setItem('viewSettings', JSON.stringify({ autoPlay: true }))
    );
    const requests = [];
    page.on('request', (request) => requests.push(request.url()));
    await TestHelpers.closeViewer(page);
    await page.locator(TestHelpers.selectors.photoCard(hevc.hash_sha256)).click();
    // The 2 s bound for a cached start is the wait's own timeout below.
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        // Anchor on the cached artifact itself: closing the viewer only pauses
        // and revokes the media, so the previous playthrough's element state is
        // still observable until the reopen swaps the source.
        return (
          el && el.currentSrc.includes('transcode=true') && el.readyState >= 2 && el.currentTime > 0
        );
      },
      null,
      { timeout: 2000 }
    );
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

  test('a failed remux stream escalates one step and recovers', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    // The ladder only runs on a stream delivery: a cached artifact would be
    // played as a file, with no stream request to fail.
    await TestHelpers.clearCachedConversions(page, mkv.hash_sha256);
    const requestedModes = await failStreamModes(page, ['remux']);

    await openVideo(page, mkv);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );

    // Exactly one rung, at once: the undecodable remux becomes audio, and the
    // ladder never falls back to the rung that just failed. This is the DECODE
    // signal's behaviour and only its: a run whose body merely stops early is a
    // LOST run, which the viewer replays on its own rung before the ladder is
    // touched (see the two lost-run specs below), so a sequence that climbs
    // here proves the element's own `error` is still read as a verdict on the
    // mode.
    expect(requestedModes[0]).toBe('remux');
    expect(requestedModes.filter((mode) => mode === 'remux')).toHaveLength(1);
    expect(new Set(requestedModes.slice(1))).toEqual(new Set(['audio']));
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('an exhausted ladder shows the error and keeps the original playable', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    await TestHelpers.clearCachedConversions(page, mkv.hash_sha256);
    const requestedModes = await failStreamModes(page, ['remux', 'audio', 'transcode']);

    await openVideo(page, mkv);
    await expect(page.locator('.transcode-toast')).toContainText('Video conversion failed', {
      timeout: 30_000,
    });

    // One rung per failure, in order, and the ladder stops at its end: three
    // attempts, never a fourth.
    expect(requestedModes).toEqual(['remux', 'audio', 'transcode']);
    // The escape hatch is offered instead of a dead end.
    const playOriginal = page.locator('.transcode-toast [data-action="play-original"]');
    await expect(playOriginal).toBeVisible();

    // AND it actually hands the original over instead of merely rendering: the
    // notice goes away with the ladder, and the element is pointed at the file
    // itself rather than at another conversion. The fixture's container is one
    // Chromium cannot present, so "decoded frames" is not available here — what
    // the hatch promises for this file is the original's URL, which is exactly
    // what a hatch that no-ops (or that re-requests a conversion) would change.
    await playOriginal.click();
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
    const video = videoHandle(page);
    await expect(video).toHaveAttribute('src', new RegExp(`/api/photos/${mkv.hash_sha256}/video`));
    // Containment alone is satisfied by a conversion request as well, so the
    // file claim needs its own negative.
    await expect(video).not.toHaveAttribute('src', /transcode=true/);
  });

  test('a stream that keeps being lost still ends on the ladder', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    await TestHelpers.clearCachedConversions(page, mkv.hash_sha256);

    // EVERY run is served a body that stops after its initialization segment:
    // no coded frame ever reaches the element, so nothing ever plays and the
    // lost-run budget is never refreshed — the case its bound exists for. The
    // ladder must still take over, one rung per spent budget, and end at its
    // own end instead of replaying one mode forever.
    //
    // Each mode's initialization segment is captured from its own real run
    // once and replayed afterwards: it is what types the SourceBuffer the run
    // attaches (`x-turbopix-mime`), so another rung's bytes would be a MIME
    // mismatch — a decode failure, not the lost run this test is about.
    const requestedModes = [];
    const initSegments = new Map();
    await page.route('**/video/stream*', async (route) => {
      const mode = new URL(route.request().url()).searchParams.get('mode');
      requestedModes.push(mode);
      const captured = initSegments.get(mode);
      if (captured) {
        await route.fulfill({
          status: 200,
          headers: {
            'content-type': 'video/mp4',
            'x-turbopix-mime': captured.mime,
          },
          body: captured.body,
        });
        return;
      }
      const response = await route.fetch();
      const body = await response.body();
      const prefix = body.subarray(0, mp4PrefixLength(body, 0));
      initSegments.set(mode, { body: prefix, mime: response.headers()['x-turbopix-mime'] });
      await route.fulfill({ response, body: prefix });
    });

    await openVideo(page, mkv);
    await expect(page.locator('.transcode-toast')).toContainText('Video conversion failed', {
      timeout: 30_000,
    });
    await expect(page.locator('.transcode-toast [data-action="play-original"]')).toBeVisible();

    // The run itself plus the two replays one budget allows, per rung, and then
    // the next rung: a mode that keeps losing is never retried a fourth time,
    // and the ladder's end still terminates.
    expect(requestedModes).toEqual([
      'remux',
      'remux',
      'remux',
      'audio',
      'audio',
      'audio',
      'transcode',
      'transcode',
      'transcode',
    ]);
  });

  test('a seek restart keeps the declared duration', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    await TestHelpers.clearCachedConversions(page, mkv.hash_sha256);

    // A seek only restarts the stream when its target is not buffered yet, and
    // this 20 s remux buffers in a few hundred milliseconds: throttle the
    // delivery so the seek below is a real restart, which is where the inflated
    // duration came from in the first place (a seek run is a stream copy whose
    // last packet lands past the source's own duration).
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.enable');
    await cdp.send('Network.emulateNetworkConditions', {
      offline: false,
      latency: 40,
      downloadThroughput: 150 * 1024,
      uploadThroughput: 150 * 1024,
      connectionType: 'cellular3g',
    });

    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator(TestHelpers.selectors.photoCard(mkv.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);

    const video = videoHandle(page);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );

    // Seek to 15 s the moment playback starts, while the target is still
    // unbuffered: the player answers by restarting the stream there.
    //
    // The restart is what is reported, not the media at the target. The run
    // this test waits on is the one already delivering the 0-20.02 s remux, and
    // its own bytes reach 15 s on their own (the throttle below puts them there
    // in ~9 s), so every condition that only asks about buffered media is
    // satisfied by a regression that dropped the seek restart entirely — no
    // second run, no `start=15` request, no replacement media source. The
    // restart itself is observable: `msePlayer` gives every run its own
    // `MediaSource` behind a fresh blob URL, so `currentSrc` changing is the
    // new run attaching. The media conditions stay: together they say the new
    // run is delivering AT the target, not merely that a source was swapped.
    const restarted = await video.evaluate(
      () =>
        new Promise((resolve) => {
          const el = document.querySelector('#viewer-video');
          let seeked = false;
          let srcBefore = null;
          const timer = setInterval(() => {
            if (!el || el.buffered.length === 0) return;
            const end = el.buffered.end(el.buffered.length - 1);
            if (!seeked) {
              if (end >= 15) return;
              if (el.currentTime <= 0) return;
              seeked = true;
              srcBefore = el.currentSrc;
              el.currentTime = 15;
              return;
            }
            if (el.currentSrc === srcBefore) return;
            if (el.seeking) return;
            if (el.currentTime < 15) return;
            if (end < 15) return;
            clearInterval(timer);
            resolve(true);
          }, 10);
          setTimeout(() => {
            clearInterval(timer);
            resolve(false);
          }, 25_000);
        })
    );
    expect(restarted, 'the seek must restart the stream at an unbuffered target').toBe(true);

    await cdp.send('Network.emulateNetworkConditions', {
      offline: false,
      latency: 0,
      downloadThroughput: -1,
      uploadThroughput: -1,
      connectionType: 'none',
    });
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.currentTime >= 15 && el.readyState >= 2;
      },
      null,
      { timeout: 20_000 }
    );

    // Play the restarted run out to its end before reading the duration. The
    // media source is created AT the declared duration (msePlayer sets
    // `mediaSource.duration` before the first append) and only grows it when an
    // appended fragment ends past it, so a read taken while the run is still
    // arriving observes 20.02 s even when the padded tail is never clamped:
    // `clampDuration` removed would still pass the bounds below. Waiting for
    // the element to finish playback means every fragment of the run has been
    // appended — an unclamped tail lands ~1 s past the source, so playback
    // would run to ~21 s, which is exactly what the upper bound rejects.
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.ended;
      },
      null,
      { timeout: 30_000 }
    );

    // The element reports the source's own duration (20.02 s), not the end of
    // the fragment the restart delivered (21 s) — the seek bar must not grow
    // past the end of the video.
    const duration = await video.evaluate((el) => el.duration);
    expect(duration).toBeGreaterThan(19.5);
    expect(duration).toBeLessThan(20.5);
  });

  test("a streamed run is typed from its own MIME, not the decision's", async ({ page }) => {
    test.setTimeout(60_000);
    const ac3 = await findVideoByFilename(page, 'test_video_ac3.mp4');
    await TestHelpers.clearCachedConversions(page, ac3.hash_sha256);

    // Simulate the mismatch the ladder runs into: the decision advertises a
    // video-only type while the rung the server actually runs emits video +
    // AAC. Chromium rejects an append whose init segment does not match the
    // SourceBuffer's declared type, so a buffer typed from the decision would
    // fail a delivery that is perfectly playable — and the ladder would burn
    // its remaining rungs on it.
    await page.route('**/video?decision*', async (route) => {
      const response = await route.fetch();
      const decision = await response.json();
      await route.fulfill({
        response,
        json: { ...decision, mime: 'video/mp4; codecs="avc1.42E01E"' },
      });
    });

    await openVideo(page, ac3);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  // The capability matrix: each row is a (fixture, client declaration) pair and
  // the decision the server must resolve it to. `mode` is null for actions that
  // carry no conversion mode. Every row clears its own file's cache first: an
  // artifact cached by a playthrough in an earlier test legitimately answers
  // `direct`/`cached`, which would hide the decision under test and make the
  // row order-dependent.
  const matrix = [
    // file, client declaration, expected action/mode
    ['test_video.mp4', 'h264-8,aac', 'direct', null],
    ['test_video_moov_end.mp4', 'h264-8,aac', 'stream', 'remux'],
    ['test_video_long.mkv', 'h264-8,aac', 'stream', 'remux'],
    ['test_video_ac3.mp4', 'h264-8,aac', 'stream', 'audio'],
    ['test_video_10bit.mp4', 'h264-8,aac', 'stream', 'transcode'],
    ['test_video_legacy.avi', 'h264-8,aac', 'stream', 'transcode'],
    ['test_video_hevc.mp4', 'h264-8,hevc,aac', 'direct', null],
    ['test_video_hevc.mp4', 'h264-8,aac', 'stream', 'transcode'],
    ['test_video_noaudio.mp4', 'h264-8,aac', 'direct', null],
  ];

  for (const [filename, client, action, mode] of matrix) {
    test(`decision matrix: ${filename} with [${client}] → ${action}/${mode}`, async ({ page }) => {
      const photo = await findVideoByFilename(page, filename);
      await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
      const response = await page.request.get(
        `/api/photos/${photo.hash_sha256}/video?decision&client=${encodeURIComponent(client)}`
      );
      expect(response.ok()).toBeTruthy();
      const decision = await response.json();
      expect(decision.action).toBe(action);
      expect(decision.mode).toBe(mode);
      if (action === 'stream') {
        expect(decision.mime).toContain('video/mp4');
        expect(decision.duration).toBeGreaterThan(0);
      }
    });
  }

  test('multi-track and silent sources play without audio errors', async ({ page }) => {
    test.setTimeout(120_000);
    for (const filename of ['test_video_multitrack.mp4', 'test_video_noaudio.mp4']) {
      const photo = await findVideoByFilename(page, filename);
      // Both fixtures are h264 the browser decodes, so they play directly —
      // and a direct source only auto-plays when the viewer's autoPlay setting
      // is on (the MSE path always self-plays). Enable it before opening them,
      // or `currentTime > 0` below would only ever be reached by a user
      // gesture.
      await page.evaluate(() =>
        localStorage.setItem('viewSettings', JSON.stringify({ autoPlay: true }))
      );
      await openVideo(page, photo);
      await page.waitForFunction(
        (hash) => {
          const el = document.querySelector('#viewer-video');
          // Anchored on the photo this element holds: closing the viewer only
          // pauses and hides it, so on the second fixture the previous source's
          // stale `currentTime > 0` would satisfy a bare playback predicate
          // before the silent source ever loaded — and an audio error on a
          // track-less file could pass unnoticed.
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
      // The multi-track source carries three streams (h264 + aac + ac3) and the
      // browser plays its first (AAC) track — the second (AC-3) one must not be
      // picked — while the silent source has no track to pick: neither may fail
      // the media element with an audio error.
      const error = await videoHandle(page).evaluate((el) => el.error?.code ?? null);
      expect(error).toBeNull();
      await TestHelpers.closeViewer(page);
    }
  });
});
