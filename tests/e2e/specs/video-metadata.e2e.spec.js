import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { TestHelpers } from '../setup/test-helpers.js';

/**
 * Fixtures (see test-data/, all seeded by global-setup):
 *   test_video.mp4                h264 + aac, `mvhd` creation 0, no location carrier
 *   test_video_quicktime_keys.mp4 h264 + aac carrying `com.apple.quicktime.creationdate`
 *                                 and `com.apple.quicktime.location.ISO6709`, moov-first
 *   test_video_long.mkv           Matroska — the viewer plays it, the writer cannot rewrite it
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
    const file = fixturePath('test_video.mp4');
    const sizeBefore = statSync(file).size;
    await openMetadataEditor(page, video);

    // WHEN: the date field is set to a past date and saved. 2015 keeps the
    // edited video behind every other seeded video, so no other spec's first
    // video card moves.
    await page.fill('#edit-taken-at', '2015-06-01T12:00');
    // The form submits `new Date(<input value>).toISOString()`, read in the
    // browser's own zone — derive the expectation the same way instead of
    // assuming the runner's zone.
    const expectedTakenAt = await page.evaluate(() => new Date('2015-06-01T12:00').toISOString());
    await page.click('#metadata-edit-save');

    // THEN: the save lands at the UI
    await expectSaved(page);

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

    // AND: the rewrite is in place — the file's length is untouched, which is
    // what lets the scanner see the file as unchanged
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

    // WHEN: the same route the UI uses saves a new capture date mid-playback
    const response = await page.request.patch(`/api/photos/${video.hash_sha256}/metadata`, {
      data: { taken_at: '2017-03-04T10:00:00.000Z' },
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
      '2017-03-04T10:00:00.000Z'
    );
  });

  test('GIVEN a video carrying a QuickTime location WHEN coordinates are saved THEN the carrier is replaced', async ({
    page,
  }) => {
    // GIVEN: test_video_quicktime_keys.mp4 carries a location carrier
    // (`+48.2082+016.3737/`) and a `com.apple.quicktime.creationdate`.
    const video = await findVideoByFilename(page, 'test_video_quicktime_keys.mp4');
    const file = fixturePath('test_video_quicktime_keys.mp4');
    const sizeBefore = statSync(file).size;
    await openMetadataEditor(page, video);

    // WHEN: Berlin is saved
    await page.fill('#edit-latitude', '52.52');
    await page.fill('#edit-longitude', '13.405');
    await page.click('#metadata-edit-save');

    // THEN: the modal closes with the success toast
    await expectSaved(page);

    // AND: the row carries the pair
    const row = await (await page.request.get(`/api/photos/${video.hash_sha256}`)).json();
    expect(row.metadata?.location?.latitude).toBe(52.52);
    expect(row.metadata?.location?.longitude).toBe(13.405);

    // AND: the position was written into the existing carrier, in its own
    // shape — the same mdta key, ffprobe's own reading
    const tags = probe(file, 'format_tags').format?.tags ?? {};
    expect(tags['com.apple.quicktime.location.ISO6709']).toBe('+52.5200+013.4050/');

    // AND: a position-only save left the date carrier alone
    expect(instantOf(tags['com.apple.quicktime.creationdate'])).toBe('2024-05-01T08:00:00.000Z');

    // AND: the rewrite is in place
    expect(statSync(file).size).toBe(sizeBefore);
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

    // WHEN: a coordinate is entered and saved
    await page.fill('#edit-latitude', '52.52');
    await page.fill('#edit-longitude', '13.405');
    await page.click('#metadata-edit-save');

    // THEN: the refusal is the specific one — the missing carrier — not a
    // generic failure, and it is shown inside the still-open modal
    await expect(page.locator('#metadata-edit-error')).toHaveText(
      'This video has no location field that can be updated.'
    );
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
});
