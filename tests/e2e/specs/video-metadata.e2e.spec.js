import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { TestHelpers } from '../setup/test-helpers.js';

/**
 * Fixtures (see test-data/, all seeded by global-setup):
 *   test_video.mp4                h264 + aac, `mvhd` creation 0, no location carrier
 *   test_video_mov.mov             test_video.mp4 under a `.mov` name (global-setup copies
 *                                 it) — FR-001's MOV acceptance, no new binary needed
 *   test_video_quicktime_keys.mp4 h264 + aac carrying `com.apple.quicktime.creationdate`
 *                                 and `com.apple.quicktime.location.ISO6709`, moov-first
 *   test_video_long.mkv           Matroska — the viewer plays it, the writer cannot rewrite it
 *   test_video_10bit.mp4          h264 High 10 — writable like the others, and the one the
 *                                 decision engine converts for a Chromium client, so the save
 *                                 can be timed against a running conversion
 *
 * The assertions below are the acceptance evidence for an edit at the UI level:
 * one save has to be visible through the FILE (ffprobe), the ROW (the API) and
 * the GRID, and every refusal has to leave the file byte-identical.
 */

async function findVideoByFilename(page, filename) {
  const response = await page.request.get('/api/photos?q=type:video&limit=200');
  expect(response.ok()).toBeTruthy();
  const data = await response.json();
  const photo = (data.photos || []).find((p) => p.filename === filename);
  expect(photo, `${filename} must be seeded and indexed`).toBeTruthy();
  return photo;
}

/** The seeded copy the server edits, relative to the runner's cwd (repo root). */
const fixturePath = (filename) => path.join('test-e2e-data', 'photos', filename);

/**
 * ffprobe's own reading of the file — the reader that owes this app nothing, so
 * a tag it reports is evidence the rewrite landed in the container rather than
 * in this app's bookkeeping.
 */
function probe(file, entries) {
  const output = execFileSync(
    'ffprobe',
    ['-v', 'error', '-show_entries', entries, '-of', 'json', file],
    { encoding: 'utf8' }
  );
  return JSON.parse(output);
}

/**
 * The instant a timestamp text denotes, whatever offset style it is written in
 * (`Z`, `+hhmm`, `+hh:mm`, any fraction width) — QuickTime carries an instant,
 * not a spelling.
 */
function instantOf(text) {
  const match = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?)(Z|[+-]\d{2}:?\d{2})$/.exec(text);
  expect(match, `unparseable timestamp: ${text}`).toBeTruthy();
  const offset = match[2] === 'Z' ? 'Z' : match[2].replace(/^([+-]\d{2})(\d{2})$/, '$1:$2');
  return new Date(`${match[1]}${offset}`).toISOString();
}

/** Everything a refused save must leave alone: the bytes, the size, the mtime. */
function fileIdentity(file) {
  const bytes = readFileSync(file);
  return {
    sha256: createHash('sha256').update(bytes).digest('hex'),
    size: bytes.length,
    mtimeMs: statSync(file).mtimeMs,
  };
}

/**
 * sha256 of every byte outside the top-level `moov` — the media payload a
 * metadata save must not touch (SC-001). The regions are read from the file's
 * own box sizes, the same structure the writer locates its region in.
 */
function payloadDigest(file) {
  const data = readFileSync(file);
  const outside = [];
  let offset = 0;
  while (offset + 8 <= data.length) {
    let size = data.readUInt32BE(offset);
    const kind = data.subarray(offset + 4, offset + 8).toString('latin1');
    if (size === 1) size = Number(data.readBigUInt64BE(offset + 8));
    if (size === 0) size = data.length - offset;
    if (kind !== 'moov') outside.push(data.subarray(offset, offset + size));
    offset += size;
  }
  return createHash('sha256').update(Buffer.concat(outside)).digest('hex');
}

/**
 * Every tag ffprobe reports except the creation timestamps a save exists to
 * move. `payloadDigest` deliberately skips the whole `moov` region, so the
 * file's OTHER carriers — `encoder`, `comment`, the brand entries on
 * test_video.mp4 — are observable nowhere else: a `moov` rewrite that dropped
 * one of them passes every other assertion in this file.
 */
function tagsExceptCreationTime(report) {
  const format = { ...(report.format?.tags ?? {}) };
  delete format.creation_time;
  return {
    format,
    streams: (report.streams ?? []).map((stream) => {
      const tags = { ...(stream.tags ?? {}) };
      delete tags.creation_time;
      return tags;
    }),
  };
}

/**
 * Records the metadata PATCH the editor sends next, at the wire, together with
 * the row the server answers with — a successful save returns that row
 * already carrying the new values and the patched file's fingerprint, so the
 * response body is part of the contract. Re-issuing the request through
 * `route.fetch()` and fulfilling the route with that very response leaves the
 * page's own request intact.
 *
 * The request contract under test is "only the fields the user touched are
 * sent": the writer rewrites every date carrier whenever it sees `taken_at`, so
 * a save that restates an untouched date would move the file's capture time
 * (and, via the handler's read-back, the row's `taken_at`).
 */
function captureMetadataPatch(page) {
  const captured = { method: null, body: null, status: null, response: null };
  page.route('**/api/photos/*/metadata', async (route) => {
    captured.method = route.request().method();
    captured.body = route.request().postDataJSON();
    const response = await route.fetch();
    captured.status = response.status();
    captured.response = await response.json().catch(() => null);
    await route.fulfill({ response });
  });
  return captured;
}

/** Opens the viewer on `photo` and the metadata editor modal on top of it. */
async function openMetadataEditor(page, photo) {
  await TestHelpers.navigateToView(page, 'videos');
  await TestHelpers.waitForPhotosToLoad(page);
  await page.locator(TestHelpers.selectors.photoCard(photo.hash_sha256)).click();
  await TestHelpers.verifyViewerOpen(page);
  await page.locator('.metadata-btn').click();
  await page.locator('.viewer-sidebar.show').waitFor();
  await page.locator('#metadata-edit-btn').click();
  await expect(page.locator('#metadata-edit-modal')).toBeVisible();
}

/** A save that is expected to land: the success toast, then the closed modal. */
async function expectSaved(page) {
  await expect(page.locator('.toast-success .toast-title')).toHaveText(
    'Metadata updated successfully'
  );
  await expect(page.locator('#metadata-edit-modal')).toBeHidden();
}

test.describe('Video metadata editing', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
  });

  test('GIVEN an MP4 video WHEN the capture date is saved THEN the file, the row and the grid agree', async ({
    page,
  }) => {
    // GIVEN: test_video.mp4 (h264 + aac, `mvhd` creation 0, no location carrier)
    const video = await findVideoByFilename(page, 'test_video.mp4');
    const rowBefore = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    const file = fixturePath('test_video.mp4');
    const sizeBefore = statSync(file).size;
    const payloadBefore = payloadDigest(file);
    // SC-002 also asks for every entry the file already had to still be there
    // afterwards. `payloadDigest` cannot see it — the region it skips is the
    // one the save rewrites — so the other carriers are read from ffprobe now
    // and compared after the save.
    const tagsBefore = tagsExceptCreationTime(probe(file, 'format_tags:stream_tags'));
    await openMetadataEditor(page, video);

    // WHEN: the date field is set to a past date and saved. The instant is
    // derived from the row as THIS attempt finds it: a Playwright retry runs
    // against the same server, DB and file (global setup does not re-seed), so
    // a literal target would already sit in the row on attempt 2 — the editor's
    // diff would find no change, `handleSubmit` would take its empty-payload
    // early return (no PATCH, no success toast) and `expectSaved` below would
    // wait forever, turning a transient first-attempt flake into a hard,
    // misleading failure.
    //
    // The band keeps the two premises the rest of this case rests on: a year
    // in the 2010s is comfortably inside the writer's window (1990-01-01 to
    // 2040-02-06, `MIN_WRITABLE_UNIX_SECONDS`) and older than every other
    // seeded video, which drops the edited video to last place in the videos
    // view — the grid assertion below depends on that. From here on the first
    // card of that view is the Matroska fixture: fine, because the only specs
    // that open a video card by index assert generically (viewer opens, the
    // video element is visible, the card carries an id), and every other video
    // spec resolves its fixture by filename or hash.
    //
    // Inside the band — which is what a retry reads back, the instant the last
    // attempt wrote — the target steps one whole hour on; outside it, which is
    // the first attempt on the file's own fallback date, it starts at the
    // band's beginning. A step can never reproduce the value it was derived
    // from, and a whole hour is minute-aligned, so the `datetime-local` field
    // spells the instant back exactly (the editor submits it minute-truncated,
    // so a non-aligned target would be rounded under the assertions).
    const target = await page.evaluate((currentTakenAt) => {
      const bandStart = new Date(2015, 0, 1, 0, 0, 0, 0).getTime();
      const bandEnd = new Date(2016, 0, 1, 0, 0, 0, 0).getTime();
      const hourMs = 60 * 60 * 1000;
      const base = Date.parse(currentTakenAt);
      const targetMs =
        Number.isFinite(base) && base >= bandStart && base < bandEnd
          ? Math.floor(base / hourMs) * hourMs + hourMs
          : bandStart;
      const date = new Date(targetMs);
      const pad = (number) => String(number).padStart(2, '0');
      const local = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
      return { local, instant: date.toISOString(), spelled: new Date(local).toISOString() };
    }, rowBefore.taken_at);
    await page.fill('#edit-taken-at', target.local);
    // The form submits `new Date(<input value>).toISOString()`, read in the
    // browser's own zone — derive the expectation the same way instead of
    // assuming the runner's zone, and pin it to the instant the derivation
    // aimed at so a local/UTC spelling slip cannot pass unnoticed.
    const expectedTakenAt = target.spelled;
    expect(expectedTakenAt).toBe(target.instant);

    const patch = captureMetadataPatch(page);
    await page.click('#metadata-edit-save');

    // THEN: the save lands at the UI
    await expectSaved(page);

    // AND: the date the user edited is the date the request carried — an
    // edited field must still be sent
    expect(patch.method).toBe('PATCH');
    expect(patch.body).toHaveProperty('taken_at', expectedTakenAt);

    // AND: the returned row already carries the saved value AND the identity of
    // the file that was patched. The row described that file, so the handler
    // restates its fingerprint: same size, same modification time, which is
    // what keeps the next scan from treating the video as changed (SC-004).
    expect(patch.status).toBe(200);
    expect(new Date(patch.response.taken_at).toISOString()).toBe(expectedTakenAt);
    expect(patch.response.file_size).toBe(statSync(file).size);
    expect(Date.parse(patch.response.file_modified)).toBe(
      Math.floor(statSync(file).mtimeMs / 1000) * 1000
    );

    // AND: the row reports the new instant
    const row = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    expect(new Date(row.taken_at).toISOString()).toBe(expectedTakenAt);

    // AND: the container says the same — `mvhd` (the format tag) and every
    // `tkhd`/`mdhd` (each stream's tag) were rewritten
    const report = probe(file, 'format_tags:stream_tags');
    const formatTime = report.format?.tags?.creation_time;
    expect(formatTime, 'ffprobe must report a format creation_time').toBeTruthy();
    expect(instantOf(formatTime)).toBe(expectedTakenAt);

    const streams = report.streams || [];
    expect(streams.length).toBeGreaterThan(0);
    for (const stream of streams) {
      const streamTime = stream.tags?.creation_time;
      expect(streamTime, `stream ${stream.index} must carry a creation_time`).toBeTruthy();
      expect(instantOf(streamTime)).toBe(expectedTakenAt);
    }

    // AND: nothing else the file carried is gone. The save rewrites the
    // creation timestamps and nothing else, so `encoder`, `comment` and the
    // brand entries still read exactly as they did before it.
    expect(tagsExceptCreationTime(report)).toEqual(tagsBefore);

    // AND: the rewrite is in place — the media payload outside `moov` and the
    // file's length are both untouched, which is what lets the scanner see the
    // file as unchanged
    expect(payloadDigest(file)).toBe(payloadBefore);
    expect(statSync(file).size).toBe(sizeBefore);

    // AND: the grid agrees. Reloaded, the videos view renders the row's own
    // order (taken_at DESC) with the edited video now last.
    await TestHelpers.goto(page, '/videos');
    await TestHelpers.waitForPhotosToLoad(page);
    const listing = await (await page.request.get('/api/photos?q=type:video&limit=200')).json();
    const orderedHashes = listing.photos.map((photo) => photo.hash_sha256);
    await expect
      .poll(() =>
        page
          .locator('#photo-grid [data-photo-id]')
          .evaluateAll((cards) => cards.map((card) => card.getAttribute('data-photo-id')))
      )
      .toEqual(orderedHashes);
    expect(orderedHashes[orderedHashes.length - 1]).toBe(video.hash_sha256);
  });

  test('GIVEN a MOV video WHEN the capture date is saved THEN the container takes the write', async ({
    page,
  }) => {
    // GIVEN: test_video_mov.mov — FR-001 accepts MOV next to MP4 and M4V, and
    // until now only the writer's extension list said so. The fixture is
    // test_video.mp4 under a `.mov` name (no location carrier, no date
    // carrier), so this case is about the gate and the in-place write, not
    // about QuickTime's own box layout — that is covered by the writer's own
    // synthetic `.mov` tests.
    const video = await findVideoByFilename(page, 'test_video_mov.mov');
    const rowBefore = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    const file = fixturePath('test_video_mov.mov');
    const sizeBefore = statSync(file).size;
    const payloadBefore = payloadDigest(file);

    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator(TestHelpers.selectors.photoCard(video.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();
    // THE GATE: a container the writer cannot rewrite leaves the button
    // disabled behind a "not supported" tooltip — MOV must not.
    await expect(page.locator('#metadata-edit-btn')).toBeEnabled();
    await page.locator('#metadata-edit-btn').click();
    await expect(page.locator('#metadata-edit-modal')).toBeVisible();

    // WHEN: an instant derived from the row as THIS attempt finds it is saved.
    // The row starts at a minute-aligned seeded date and every attempt reads
    // back what the last one wrote, so stepping a whole hour can never
    // reproduce the value it was derived from — the alternative is the
    // empty-diff early return of the case above, which never patches.
    const hourMs = 60 * 60 * 1000;
    const targetMs = Math.floor(Date.parse(rowBefore.taken_at) / hourMs) * hourMs + hourMs;
    const target = await page.evaluate((ms) => {
      const date = new Date(ms);
      const pad = (number) => String(number).padStart(2, '0');
      const local = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
      return { local, instant: date.toISOString(), spelled: new Date(local).toISOString() };
    }, targetMs);
    await page.fill('#edit-taken-at', target.local);
    const expectedTakenAt = target.spelled;
    expect(expectedTakenAt).toBe(target.instant);

    const patch = captureMetadataPatch(page);
    await page.click('#metadata-edit-save');

    // THEN: the modal closes with the success toast
    await expectSaved(page);

    // AND: the request carried the instant, the returned row restates it, and
    // the row itself agrees
    expect(patch.method).toBe('PATCH');
    expect(patch.status).toBe(200);
    expect(patch.body).toHaveProperty('taken_at', expectedTakenAt);
    expect(new Date(patch.response.taken_at).toISOString()).toBe(expectedTakenAt);
    const row = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    expect(new Date(row.taken_at).toISOString()).toBe(expectedTakenAt);

    // AND: the container read the write back — `mvhd` and every stream header
    const report = probe(file, 'format_tags:stream_tags');
    const formatTime = report.format?.tags?.creation_time;
    expect(formatTime, 'ffprobe must report a format creation_time').toBeTruthy();
    expect(instantOf(formatTime)).toBe(expectedTakenAt);
    const streams = report.streams || [];
    expect(streams.length).toBeGreaterThan(0);
    for (const stream of streams) {
      const streamTime = stream.tags?.creation_time;
      expect(streamTime, `stream ${stream.index} must carry a creation_time`).toBeTruthy();
      expect(instantOf(streamTime)).toBe(expectedTakenAt);
    }

    // AND: the rewrite stayed in place — the file's length and the media
    // payload outside `moov` are both untouched
    expect(payloadDigest(file)).toBe(payloadBefore);
    expect(statSync(file).size).toBe(sizeBefore);
  });

  test('GIVEN a video playing WHEN its capture date is saved THEN playback continues and the media stream is unchanged', async ({
    page,
  }) => {
    // GIVEN: a 10 s natively playable video open in the viewer and playing.
    // test_video_noaudio.mp4 rather than test_video.mp4 because the latter is
    // 0.3 s long — playback would be over before a save could be observed.
    const video = await findVideoByFilename(page, 'test_video_noaudio.mp4');
    const file = fixturePath('test_video_noaudio.mp4');
    const before = fileIdentity(file);
    const payloadBefore = payloadDigest(file);

    // A directly played source only auto-plays with the viewer's own setting on
    // (the MSE path self-plays); without it the element never leaves 0.
    await page.evaluate(() =>
      localStorage.setItem('viewSettings', JSON.stringify({ autoPlay: true }))
    );
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator(TestHelpers.selectors.photoCard(video.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);

    const handle = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(handle).toBeVisible();
    await page.waitForFunction(
      () => {
        const element = document.querySelector('#viewer-video');
        return element !== null && element.readyState >= 2 && element.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );
    // Restart from the top so the whole 10 s lies ahead of the save, and so the
    // read it starts is in flight across the write — that overlap is the point.
    await handle.evaluate((element) => {
      element.currentTime = 0;
    });
    const currentTimeBefore = await handle.evaluate((element) => element.currentTime);
    const srcBefore = await handle.getAttribute('src');

    // WHEN: the same route the UI uses saves a new capture date mid-playback.
    // The instant is derived from the row as it is *now* — an hour later, whole
    // seconds, because the container keeps the QuickTime epoch value in seconds —
    // so a Playwright retry (same server, same DB, same file; globalSetup does
    // not re-run) writes a fresh value instead of re-saving the one a previous
    // attempt already stored, which would be a no-op these assertions could not
    // observe.
    const rowBefore = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    const baseMs = rowBefore.taken_at
      ? Date.parse(rowBefore.taken_at)
      : Date.parse('2017-03-04T10:00:00.000Z');
    const targetTakenAt = new Date(Math.floor(baseMs / 1000) * 1000 + 60 * 60 * 1000).toISOString();
    const response = await page.request.patch(`/api/photos/${video.hash_sha256}/metadata`, {
      data: { taken_at: targetTakenAt },
    });

    // THEN: the save lands
    expect(response.ok()).toBeTruthy();

    // AND: the element is still playing the file it was handed — the position
    // moves past where it was, on the same non-transcoded source with no error
    await expect
      .poll(() => handle.evaluate((element) => element.currentTime), { timeout: 15_000 })
      .toBeGreaterThan(currentTimeBefore);
    expect(
      await handle.evaluate((element) => (element.error ? element.error.code : null))
    ).toBeNull();
    const srcAfter = await handle.getAttribute('src');
    expect(srcAfter).toBe(srcBefore);
    expect(srcAfter).not.toContain('transcode=true');
    expect(srcAfter).not.toMatch(/^blob:/);

    // AND: the container took the write while it was being read — the bytes
    // outside `moov` are untouched, the length and mtime do not move, and the
    // file as a whole did change (so this is not a no-op save)
    expect(payloadDigest(file)).toBe(payloadBefore);
    expect(statSync(file).size).toBe(before.size);
    expect(statSync(file).mtimeMs).toBe(before.mtimeMs);
    expect(fileIdentity(file).sha256).not.toBe(before.sha256);
    expect(instantOf(probe(file, 'format_tags').format.tags.creation_time)).toBe(
      new Date(targetTakenAt).toISOString()
    );
  });

  test('GIVEN a video carrying a QuickTime location WHEN coordinates are saved THEN the carrier is replaced and the capture date is left alone', async ({
    page,
  }) => {
    // GIVEN: test_video_quicktime_keys.mp4 carries a location carrier
    // (`+48.2082+016.3737/`) and a `com.apple.quicktime.creationdate`.
    const video = await findVideoByFilename(page, 'test_video_quicktime_keys.mp4');
    const file = fixturePath('test_video_quicktime_keys.mp4');
    const sizeBefore = statSync(file).size;
    const payloadBefore = payloadDigest(file);
    const rowBefore = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    await openMetadataEditor(page, video);

    // WHEN: a position one hundredth of a degree north and east of wherever
    // THIS attempt finds the video is saved (the date field is left as the
    // editor opened it). The pair is derived rather than pinned for the same
    // reason as the case above: a retry re-runs against the same server, DB
    // and file, so a literal pair would already sit in the row on attempt 2 —
    // the editor's diff would find no change, `handleSubmit` would take its
    // empty-payload early return (no PATCH, no toast) and `expectSaved` below
    // would wait forever. A step of a fixed amount from the row's own value
    // can never reproduce that value, so every attempt has a real change to
    // send.
    //
    // Four decimals is the container's own precision — the ISO-6709 carrier
    // spells `+DD.DDDD+DDD.DDDD/` — so a rounded step is written back verbatim
    // and the pair the file carries is the pair this test asserts. The seed is
    // the fixture's own carrier value, used only while the row still has none.
    const SEED = { latitude: 48.2082, longitude: 16.3737 };
    const step = (value, limit, seed) => {
      const stepped = Math.round((value + 0.01) * 10_000) / 10_000;
      // Only reachable by a long retry chain walking out of the valid band;
      // the seed is inside it, so the next attempt steps on from there.
      return Math.abs(stepped) <= limit ? stepped : seed;
    };
    const current = rowBefore.metadata?.location ?? {};
    const targetLatitude = step(current.latitude ?? SEED.latitude, 90, SEED.latitude);
    const targetLongitude = step(current.longitude ?? SEED.longitude, 180, SEED.longitude);
    // The carrier's own spelling, read off the pair instead of written out as a
    // literal: a sign, four decimals, and a zero-padded integer part — two
    // digits for latitude, three for longitude (`16.3837` is seven characters,
    // the carrier gives longitude eight), then the terminator.
    const carrierPart = (value, integerDigits) =>
      `${value < 0 ? '-' : '+'}${Math.abs(value).toFixed(4).padStart(integerDigits, '0')}`;
    const targetCarrier = `${carrierPart(targetLatitude, 0)}${carrierPart(targetLongitude, 8)}/`;
    await page.fill('#edit-latitude', String(targetLatitude));
    await page.fill('#edit-longitude', String(targetLongitude));
    const patch = captureMetadataPatch(page);
    await page.click('#metadata-edit-save');

    // THEN: the modal closes with the success toast
    await expectSaved(page);

    // AND: the request carried the position and nothing else — an untouched
    // date is not restated, which is what keeps the writer away from the date
    // carriers
    expect(patch.method).toBe('PATCH');
    expect(patch.body).not.toHaveProperty('taken_at');
    expect(patch.body.latitude).toBe(targetLatitude);
    expect(patch.body.longitude).toBe(targetLongitude);

    // AND: the returned row already carries the pair the user saved — the save
    // hands the caller the state it committed instead of making it re-fetch
    expect(patch.status).toBe(200);
    expect(patch.response.metadata?.location?.latitude).toBe(targetLatitude);
    expect(patch.response.metadata?.location?.longitude).toBe(targetLongitude);

    // AND: the row carries the pair
    const row = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    expect(row.metadata?.location?.latitude).toBe(targetLatitude);
    expect(row.metadata?.location?.longitude).toBe(targetLongitude);

    // AND: the open viewer shows them — the save replaces the photo it is
    // showing, so the sidebar reads the new pair without reopening anything
    const gps = page.locator('#meta-gps');
    await expect(gps).toBeVisible();
    await expect(gps).toHaveText(`${targetLatitude.toFixed(6)}, ${targetLongitude.toFixed(6)}`);

    // AND: the position was written into the existing carrier, in its own
    // shape — the same mdta key, ffprobe's own reading
    const tags = probe(file, 'format_tags').format?.tags ?? {};
    expect(tags['com.apple.quicktime.location.ISO6709']).toBe(targetCarrier);

    // AND: a position-only save left the date carrier alone — no `taken_at` was
    // sent, so it still holds the fixture's own instant
    expect(instantOf(tags['com.apple.quicktime.creationdate'])).toBe('2024-05-01T08:00:00.000Z');

    // AND: so did the row's date, which the handler mirrors from that carrier
    expect(row.taken_at).toBe(rowBefore.taken_at);

    // AND: the rewrite is in place — the media payload outside `moov` and the
    // file's length are both untouched
    expect(payloadDigest(file)).toBe(payloadBefore);
    expect(statSync(file).size).toBe(sizeBefore);

    // AND: the video plots at the new position. The query is scoped to the one
    // file so its marker cannot be folded into the seeded image cluster (a
    // cluster has no per-photo marker), and the tiles are stubbed — the map
    // must prove where the point is, not whether the tile server is reachable.
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map?q=type%3Avideo%20quicktime_keys');
    await expect(
      page.locator(`[data-map-location="${targetLatitude},${targetLongitude}"]`)
    ).toBeVisible({
      timeout: 15_000,
    });
  });

  test('GIVEN a video without a location carrier WHEN coordinates are saved THEN the refusal is specific and nothing changes', async ({
    page,
  }) => {
    // GIVEN: test_video.mp4 has no place to put a coordinate, and a snapshot of
    // the bytes and mtime it must not lose
    const video = await findVideoByFilename(page, 'test_video.mp4');
    const file = fixturePath('test_video.mp4');
    const before = fileIdentity(file);
    const rowBefore = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    await openMetadataEditor(page, video);

    // WHEN: a coordinate is entered and saved (the date the video already has,
    // set by the earlier save, is left as the editor opened it)
    await page.fill('#edit-latitude', '52.52');
    await page.fill('#edit-longitude', '13.405');
    const patch = captureMetadataPatch(page);
    await page.click('#metadata-edit-save');

    // THEN: the refusal is the specific one — the missing carrier — not a
    // generic failure, and it is shown inside the still-open modal
    await expect(page.locator('#metadata-edit-error')).toHaveText(
      'This video has no location field that can be updated.'
    );

    // AND: the refused request was a location-only one — the date the video
    // already had was not restated, so the refusal is about the missing carrier
    // and nothing else. The guard for reading the capture is the
    // `#metadata-edit-error` text above, not a poll on it: the route handler
    // records method and body as the request comes through, then awaits
    // `route.fetch()` before recording the status, and the page cannot render
    // that refusal until `route.fulfill` has handed the response over — so by
    // the time the text matches, all three are recorded, and the plain `expect`
    // assertions below are safe.
    expect(patch.method).toBe('PATCH');
    expect(patch.body).not.toHaveProperty('taken_at');
    expect(patch.status).toBe(422);
    await expect(page.locator('#metadata-edit-modal')).toBeVisible();
    await expect(page.locator('.toast-success')).toHaveCount(0);

    // AND: the entered values are still in the form, so the user can correct
    // them without retyping
    await expect(page.locator('#edit-latitude')).toHaveValue('52.52');
    await expect(page.locator('#edit-longitude')).toHaveValue('13.405');

    // AND: the file is untouched — the refusal was decided before a byte was
    // written, mtime included
    expect(fileIdentity(file)).toEqual(before);

    // AND: so is the row — no coordinate was recorded and the date it already
    // had is still the one it has
    const rowAfter = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    expect(rowAfter.taken_at).toBe(rowBefore.taken_at);
    expect(rowAfter.metadata?.location ?? null).toEqual(rowBefore.metadata?.location ?? null);
  });

  test('GIVEN a Matroska video WHEN the viewer is opened THEN the editor offers no save', async ({
    page,
  }) => {
    // GIVEN: test_video_long.mkv is indexed and opens in the viewer
    const video = await findVideoByFilename(page, 'test_video_long.mkv');
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator(TestHelpers.selectors.photoCard(video.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();

    // THEN: the editor is disabled and its tooltip names the container, so the
    // refusal is about this format rather than a vague "not supported"
    const editBtn = page.locator('#metadata-edit-btn');
    await expect(editBtn).toBeDisabled();
    await expect(editBtn).toHaveAttribute('title', /Matroska/);

    // AND: nothing happens when it is clicked anyway — no modal promises a save
    // this container cannot take
    await editBtn.dispatchEvent('click');
    await expect(page.locator('#metadata-edit-modal')).toHaveCount(0);
  });

  test('GIVEN a video being converted WHEN its capture date is saved THEN the conversion finishes and the media stream is unchanged', async ({
    page,
  }) => {
    test.setTimeout(120_000);

    // GIVEN: test_video_10bit.mp4 is h264 High 10 — the decision engine converts
    // it for a client announcing h264-8 — and its container is writable, so the
    // save below is a real write to a real MP4 while that conversion runs. The
    // conversion is cleared first: a cached artifact would make the "finished"
    // assertions pass without any job ever having been spawned.
    const video = await findVideoByFilename(page, 'test_video_10bit.mp4');
    const file = fixturePath('test_video_10bit.mp4');
    await TestHelpers.clearCachedConversions(page, video.hash_sha256);
    const sizeBefore = statSync(file).size;
    const payloadBefore = payloadDigest(file);
    const stateOf = async () => {
      const response = await page.request.get(`/api/photos/${video.hash_sha256}/video/status`);
      if (!response.ok()) return 'no-status';
      return (await response.json()).state;
    };
    // The instant is derived from the row as this attempt finds it. A Playwright
    // retry runs against the same server, DB and file, so a literal target would
    // be one hour stale on the second run and the save would be a no-op.
    const rowBefore = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    const baseMs = rowBefore.taken_at
      ? Date.parse(rowBefore.taken_at)
      : Date.parse('2017-03-04T10:00:00.000Z');
    const targetTakenAt = new Date(Math.floor(baseMs / 1000) * 1000 + 60 * 60 * 1000).toISOString();

    // WHEN: the whole-file conversion is claimed (202 — the job is spawned
    // before this response is sent) and the date is saved while it runs. The
    // state is read immediately and polled on: a status round trip is three
    // orders of magnitude shorter than the job, so this observes the overlap
    // rather than assuming it.
    const trigger = await page.request.get(`/api/photos/${video.hash_sha256}/video?transcode=true`);
    expect(trigger.status()).toBe(202);
    await expect.poll(stateOf, { timeout: 5_000, intervals: [50] }).toBe('InProgress');

    const saved = await page.request.patch(`/api/photos/${video.hash_sha256}/metadata`, {
      data: { taken_at: targetTakenAt },
    });
    expect(saved.status(), 'the save must succeed while the conversion runs').toBe(200);

    // THEN: the job in flight at the moment of the save is the one that
    // finishes — the save does not cancel, fail or restart it
    await expect.poll(stateOf, { timeout: 90_000, intervals: [1000] }).toBe('Completed');

    // AND: the artifact that finished is the one still being served, so the
    // save did not move the file's identity out from under the running job
    // (the artifact is keyed by exactly the size and mtime the save preserves)
    const decision = await (
      await page.request.get(`/api/photos/${video.hash_sha256}/video?decision&client=h264-8%2Caac`)
    ).json();
    expect(decision).toMatchObject({ action: 'direct', cached: true, encoder: expect.any(String) });

    // AND: what the save wrote is what the file and the row now hold
    expect(instantOf(probe(file, 'format_tags').format.tags.creation_time)).toBe(targetTakenAt);
    const rowAfter = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    expect(new Date(rowAfter.taken_at).toISOString()).toBe(targetTakenAt);

    // AND: the media the conversion read is untouched — the save rewrote only
    // `moov`, and left the length the job was keyed by alone
    expect(payloadDigest(file)).toBe(payloadBefore);
    expect(statSync(file).size).toBe(sizeBefore);

    // AND: the resulting media stream is unchanged, which "the conversion
    // finished" alone does not say. A conversion truncated by the write is
    // still served and still renders a `<video>`, so it would pass a
    // visibility-only assertion while the element carries an `error` and no
    // frame. The viewer only auto-plays a file when autoPlay is on (the MSE
    // path self-plays), so enable it before opening the artifact.
    await page.evaluate(() =>
      localStorage.setItem('viewSettings', JSON.stringify({ autoPlay: true }))
    );
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator(TestHelpers.selectors.photoCard(video.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);
    const handle = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(handle).toBeVisible();
    await page.waitForFunction(
      (hash) => {
        const el = document.querySelector('#viewer-video');
        // Anchored on the photo this element holds, so an earlier delivery
        // cannot stand in for the artifact produced above.
        return (
          el &&
          el.dataset.photoHash === hash &&
          el.readyState >= 2 &&
          el.currentTime > 0 &&
          !el.error
        );
      },
      video.hash_sha256,
      { timeout: 30_000 }
    );
  });
});
