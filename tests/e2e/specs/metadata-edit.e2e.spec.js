import { execFileSync } from 'node:child_process';
import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';
import { fetchAllPhotos } from '../setup/photo-pages.js';

/**
 * The metadata edit flow, end to end: a save through the viewer's modal lands
 * in the PHOTO FILE and never in the database, the app reports the file's own
 * value (DMS precision for coordinates), and a file type the writer refuses
 * says so instead of pretending.
 *
 * The library database the running server serves; global-setup.js points the
 * server at the same file. `sqlite3` is a hard dependency of the harness (it
 * seeds the housekeeping/collage rows the same way).
 */
const DB_PATH = 'test-e2e-data/database/turbo-pix.db';

/**
 * The one camera-EXIF fixture (Canon EOS 1100D, DateTimeOriginal
 * 2024-01-01 12:00:00, no GPS tags). No other spec depends on its date — the
 * camera-EXIF check in metadata.e2e.spec.js locates it by hash — which is what
 * makes it the photo these specs may re-date.
 */
const FIXTURE = 'sample_with_exif.jpg';

/** The date the date half saves, in the form field's local wall-clock format. */
const NEW_LOCAL_DATE = '2024-05-06T14:30';

/**
 * The point the coordinate half saves. Seven decimals on purpose: the writer
 * stores degrees/minutes/seconds with a 1/1000-arcsecond rational, so neither
 * decimal is representable in the file — the API answering the REQUEST back
 * would be visible as an exact match.
 */
const NEW_LATITUDE = -33.8568117;
const NEW_LONGITUDE = 151.2152901;

/** Runs one statement against the library database and returns its stdout. */
function runSql(sql) {
  return execFileSync('sqlite3', [DB_PATH, `PRAGMA busy_timeout=5000; ${sql}`], {
    encoding: 'utf8',
  });
}

/** `PRAGMA table_info(photos)` column names, in table order. */
function photoColumns() {
  return runSql('PRAGMA table_info(photos);')
    .split('\n')
    .filter(Boolean)
    .map((line) => line.split('|')[1]);
}

/** The raw `photos.metadata` JSON as the server stored it. */
function storedMetadata(hashSha256) {
  return runSql(`SELECT metadata FROM photos WHERE hash_sha256='${hashSha256}';`).trim();
}

/** A fixture photo from the listing (the hash these specs address it by). */
async function getPhotoByFilename(page, filename) {
  const photos = await fetchAllPhotos(async (requestPath) => {
    const response = await page.request.get(requestPath);
    expect(response.ok()).toBeTruthy();
    return response.json();
  });
  const photo = photos.find((candidate) => candidate.filename === filename);
  expect(photo, `fixture ${filename} is indexed`).toBeTruthy();
  return photo;
}

/** The server's timeline density as it stands now. */
async function timelineDensity(page) {
  const response = await page.request.get('/api/photos/timeline');
  expect(response.ok()).toBeTruthy();
  const { density = [] } = await response.json();
  return density;
}

/** The count one year/month bucket of that density holds, 0 when absent. */
function monthCount(density, date) {
  const bucket = density.find(
    (candidate) =>
      candidate.year === date.getUTCFullYear() && candidate.month === date.getUTCMonth() + 1
  );
  return bucket?.count ?? 0;
}

/** The photo as the server answers it now — a second, independent read. */
async function getPhotoFromApi(page, hashSha256) {
  const response = await page.request.get(`/api/photos/${hashSha256}`);
  expect(response.ok()).toBeTruthy();
  return await response.json();
}

/** The viewer open on `hash`, with the metadata sidebar that holds the edit button. */
async function openViewerMetadata(page, hashSha256) {
  await TestHelpers.goto(page, `/?photo=${hashSha256}`);
  await TestHelpers.verifyViewerOpen(page);
  await page.locator('.metadata-btn').click();
  await page.locator('.viewer-sidebar.show').waitFor();
}

/**
 * The modal's fields, scoped to the form: the sidebar labels its rows with
 * spans, but scoping keeps the lookup independent of that.
 */
function modalField(page, label) {
  return page.locator('#metadata-edit-form').getByLabel(label);
}

async function openModal(page) {
  await page.locator('#metadata-edit-btn').click();
  await expect(page.locator('#metadata-edit-modal')).toBeVisible();
}

/** Closes the modal without saving (Escape is the modal's own close path). */
async function closeModal(page) {
  await page.keyboard.press('Escape');
  await expect(page.locator('#metadata-edit-modal')).toHaveCount(0);
}

/**
 * Saves `fill` through the modal and returns the PATCH response's photo.
 * Waits for the modal to close, so the caller reads a settled viewer.
 */
async function saveThroughModal(page, hashSha256, fill) {
  await openModal(page);
  await fill();
  const [response] = await Promise.all([
    page.waitForResponse(
      (candidate) =>
        candidate.request().method() === 'PATCH' &&
        new URL(candidate.url()).pathname === `/api/photos/${hashSha256}/metadata`
    ),
    page.locator('#metadata-edit-save').click(),
  ]);
  expect(response.ok()).toBeTruthy();
  await expect(page.locator('#metadata-edit-modal')).toHaveCount(0);
  return await response.json();
}

test.describe('Metadata edit', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
  });

  test('saving a date lands in the file, never in the database', async ({ page }) => {
    // GIVEN the camera-EXIF fixture, open in its own month — the state the
    // grid is filtered to is what shows whether the client re-reads the file.
    const photo = await getPhotoByFilename(page, FIXTURE);
    const originalTakenAt = photo.taken_at;
    expect(originalTakenAt, `${FIXTURE} carries a file date`).toBeTruthy();
    const original = new Date(originalTakenAt);
    // The save has to MOVE the date, or every assertion below is vacuous. The
    // fixture is restored in the `finally`, so this only fails when an earlier
    // run died between the save and the restore.
    expect(Date.parse(originalTakenAt), `${FIXTURE} is not already re-dated`).not.toBe(
      Date.parse(NEW_LOCAL_DATE)
    );

    let originalLocalDate = null;
    // Set by the `finally`'s restore fallback when even the API restore fails;
    // asserted after the `finally`, never inside it (see there).
    let restoreFallbackFailure = null;
    try {
      await TestHelpers.goto(
        page,
        `/?year=${original.getUTCFullYear()}&month=${original.getUTCMonth() + 1}&photo=${photo.hash_sha256}`
      );
      await TestHelpers.verifyViewerOpen(page);
      const card = page.locator(TestHelpers.selectors.photoCard(photo.hash_sha256));
      await expect(card, `${FIXTURE} is in the month it is filtered to`).toHaveCount(1);

      await page.locator('.metadata-btn').click();
      await page.locator('.viewer-sidebar.show').waitFor();
      const shownBefore = await page.locator('#photo-date').textContent();

      // The app's own rendering of the file's date into the form (local
      // wall-clock, minute granularity); this is what restores the fixture.
      await openModal(page);
      originalLocalDate = await modalField(page, 'Date Taken').inputValue();
      expect(originalLocalDate).toBeTruthy();
      await closeModal(page);

      // GIVEN the timeline the mounted TimelineSlider renders: a server-side
      // aggregate (the file dates), read before the edit. The component only
      // re-reads it when the save dispatches `photosReloadRequested`, so the
      // page's OWN requests for it are tracked below — our `page.request` reads
      // are a separate context and never count — which is what pins the
      // listener: a typo'd event name would leave the graph stale with no
      // request made.
      const densityBefore = await timelineDensity(page);
      const originalDate = new Date(originalTakenAt);
      const originalBucketBefore = monthCount(densityBefore, originalDate);
      const timelineRequests = [];
      const trackTimelineRequest = (request) => {
        if (new URL(request.url()).pathname === '/api/photos/timeline') {
          timelineRequests.push(request.url());
        }
      };
      page.on('request', trackTimelineRequest);
      const timelineReloadsBeforeSave = timelineRequests.length;

      // WHEN the metadata edit modal saves a new date
      const saved = await saveThroughModal(page, photo.hash_sha256, () =>
        modalField(page, 'Date Taken').fill(NEW_LOCAL_DATE)
      );

      // THEN the PATCH answers the instant that was submitted, at the file's
      // second precision — and a second read of the server agrees, so the
      // value came from the file rather than from the request.
      const submitted = Date.parse(NEW_LOCAL_DATE);
      expect(Date.parse(saved.taken_at)).toBe(submitted);
      const reread = await getPhotoFromApi(page, photo.hash_sha256);
      expect(reread.taken_at).toBe(saved.taken_at);

      // AND the viewer shows it: the date row moved off the old value, and the
      // modal renders the file's value back into the form.
      await expect(page.locator('#photo-date')).not.toHaveText(shownBefore);
      const shownAfterSave = await page.locator('#photo-date').textContent();
      await openModal(page);
      await expect(modalField(page, 'Date Taken')).toHaveValue(NEW_LOCAL_DATE);
      await closeModal(page);

      // AND the grid behind the viewer dropped the photo: the date change made
      // the viewer ask for a reload, and the re-read applies the month filter,
      // which the photo no longer matches.
      await expect(card).toHaveCount(0);
      expect(TestHelpers.getUrlState(page)).toMatchObject({
        year: original.getUTCFullYear(),
        month: original.getUTCMonth() + 1,
      });

      // AND the mounted timeline re-read its density, and the photo's month
      // bucket moved with it: the tracked request proves the listener fired, and
      // the aggregate proves the graph now carries the file's new date. Both
      // deltas are relative to the reads above, so whatever month residue other
      // specs left in the shared library cannot make these pass.
      await expect
        .poll(() => timelineRequests.length, {
          message: 'the saved date must make the mounted TimelineSlider re-read its density',
        })
        .toBeGreaterThan(timelineReloadsBeforeSave);
      page.off('request', trackTimelineRequest);
      const savedDate = new Date(saved.taken_at);
      expect(
        `${savedDate.getUTCFullYear()}-${savedDate.getUTCMonth() + 1}`,
        'the save must move the photo to a different month bucket'
      ).not.toBe(`${originalDate.getUTCFullYear()}-${originalDate.getUTCMonth() + 1}`);
      const savedBucketBefore = monthCount(densityBefore, savedDate);
      const densityAfter = await timelineDensity(page);
      expect(monthCount(densityAfter, originalDate)).toBe(originalBucketBefore - 1);
      expect(monthCount(densityAfter, savedDate)).toBe(savedBucketBefore + 1);

      // AND the database stores no part of the date: no column for it, and no
      // key for it in the row's metadata (the schema after the file-only-date
      // migration).
      expect(photoColumns()).not.toContain('taken_at');
      const metadataJson = storedMetadata(photo.hash_sha256);
      expect(metadataJson).not.toMatch(/"taken_at"\s*:/);

      // WHEN the page is reloaded and the server is read again
      await openViewerMetadata(page, photo.hash_sha256);
      const afterReload = await getPhotoFromApi(page, photo.hash_sha256);

      // THEN the date is unchanged and the viewer renders the same value it
      // rendered before the reload: both come from the file.
      expect(afterReload.taken_at).toBe(saved.taken_at);
      expect(await page.locator('#photo-date').textContent()).toBe(shownAfterSave);
    } finally {
      // Restore the fixture's own date through the same modal, so the rest of
      // the run sees the library it was seeded with. A failed restore must not
      // mask the test's own failure: report it, then write the date through the
      // API so the file is left as found either way.
      try {
        if (originalLocalDate === null) throw new Error('the modal never rendered a date value');
        await openViewerMetadata(page, photo.hash_sha256);
        await saveThroughModal(page, photo.hash_sha256, () =>
          modalField(page, 'Date Taken').fill(originalLocalDate)
        );
      } catch (error) {
        console.error(`Restoring ${FIXTURE}'s date through the modal failed: ${error.message}`);
        // Deliberately non-throwing: this runs inside `finally`, and a throw
        // from there REPLACES an exception the test body already raised, hiding
        // the real failure from the report. The fallback's outcome is recorded
        // here and asserted after the `finally` instead.
        try {
          const restored = await page.request.patch(`/api/photos/${photo.hash_sha256}/metadata`, {
            data: { taken_at: new Date(originalTakenAt).toISOString() },
          });
          if (!restored.ok()) {
            restoreFallbackFailure = `the API restore fallback answered ${restored.status()}`;
          }
        } catch (fallbackError) {
          restoreFallbackFailure = `the API restore fallback threw: ${fallbackError.message}`;
        }
      }
    }

    // Asserted after the `finally`, so a failed fallback can never supersede an
    // in-flight body failure (a throw inside `finally` would). When the body
    // did fail, that failure surfaces instead and this line never runs.
    expect(restoreFallbackFailure).toBeNull();

    // The fixture is back where it started, so the rest of the run reads the
    // seeded library. Asserted here, not in the `finally`, so a failed restore
    // cannot replace the error the test body already raised; exact to the
    // second, which is what this fixture's EXIF date and the form's
    // minute-granular field can express.
    const restoredPhoto = await getPhotoFromApi(page, photo.hash_sha256);
    expect(Date.parse(restoredPhoto.taken_at)).toBe(Date.parse(originalTakenAt));
  });

  test('saving coordinates lands in the file at DMS precision and on the map', async ({ page }) => {
    // NOTE: this spec deliberately leaves the fixture's coordinates behind for
    // the rest of the run — clearing a coordinate is unsupported by design, and
    // the map specs derive their geo-located expectations from the API
    // (map.e2e.spec.js reads the expected count and anchors on the densest
    // location), so one more located photo cannot change their outcome.
    const photo = await getPhotoByFilename(page, FIXTURE);
    await openViewerMetadata(page, photo.hash_sha256);

    // WHEN the metadata edit modal saves coordinates
    const saved = await saveThroughModal(page, photo.hash_sha256, async () => {
      await modalField(page, 'Latitude').fill(String(NEW_LATITUDE));
      await modalField(page, 'Longitude').fill(String(NEW_LONGITUDE));
    });

    // THEN the answer is the FILE's round trip, not the requested decimal: the
    // writer stores DMS with 1/1000-arcsecond seconds, which cannot represent
    // either request exactly.
    const savedLatitude = saved.metadata?.location?.latitude;
    const savedLongitude = saved.metadata?.location?.longitude;
    expect(Math.abs(savedLatitude - NEW_LATITUDE)).toBeLessThanOrEqual(1e-4);
    expect(Math.abs(savedLongitude - NEW_LONGITUDE)).toBeLessThanOrEqual(1e-4);
    expect(savedLatitude).not.toBe(NEW_LATITUDE);
    expect(savedLongitude).not.toBe(NEW_LONGITUDE);

    // AND a second read of the server agrees with the PATCH response.
    const reread = await getPhotoFromApi(page, photo.hash_sha256);
    expect(reread.metadata?.location?.latitude).toBe(savedLatitude);
    expect(reread.metadata?.location?.longitude).toBe(savedLongitude);

    // AND the viewer shows the file's values: the panel's six-decimal
    // convention, and the form's own rendering of the file's coordinates.
    await expect(page.locator('#photo-location')).toHaveText(
      `${savedLatitude.toFixed(6)}, ${savedLongitude.toFixed(6)}`
    );
    await openViewerMetadata(page, photo.hash_sha256);
    await openModal(page);
    await expect(modalField(page, 'Latitude')).toHaveValue(String(savedLatitude));
    await expect(modalField(page, 'Longitude')).toHaveValue(String(savedLongitude));
    await closeModal(page);

    // AND the database stores no coordinates, although the file now carries
    // them: the row's metadata keeps no latitude/longitude key.
    const metadataJson = storedMetadata(photo.hash_sha256);
    expect(metadataJson).not.toMatch(/"latitude"\s*:/);
    expect(metadataJson).not.toMatch(/"longitude"\s*:/);

    // AND the map plots the photo at exactly those coordinates: the marker's
    // own key is the pair the API serves (map.js `groupPhotosByLocation`), so
    // finding it proves the plotted point is the file's, not the request's.
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();

    const mapResponse = await page.request.get('/api/photos/map');
    expect(mapResponse.ok()).toBeTruthy();
    const { photos: mapPhotos } = await mapResponse.json();
    const keyOf = (candidate) =>
      `${candidate.metadata?.location?.latitude},${candidate.metadata?.location?.longitude}`;
    const plotted = mapPhotos.find((candidate) => candidate.hash_sha256 === photo.hash_sha256);
    expect(plotted, `${FIXTURE} is in the map's photo set`).toBeTruthy();
    const key = keyOf(plotted);
    expect(key).toBe(`${savedLatitude},${savedLongitude}`);
    const locationCount = mapPhotos.filter((candidate) => keyOf(candidate) === key).length;

    // The fitted view clusters everything, so expand until the marker itself
    // is rendered.
    const marker = page.locator(`[data-map-location="${key}"]`);
    for (let attempt = 0; attempt < 5; attempt += 1) {
      if ((await marker.count()) > 0) break;
      const cluster = page.locator('[data-map-cluster]').first();
      if ((await cluster.count()) === 0) break;
      await cluster.click();
      // The zoom animation pane detaches once the jump completes.
      await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    }
    await expect(marker).toBeVisible();
    await expect(marker).toHaveAttribute('data-map-location-count', String(locationCount));
  });

  test('a video cannot be edited and the interface says so', async ({ page }) => {
    // GIVEN a video fixture open in the viewer
    const video = await getPhotoByFilename(page, 'test_video.mp4');
    await openViewerMetadata(page, video.hash_sha256);

    // THEN the edit control is disabled and its accessible name states the
    // limitation (metadata_writer refuses video/RAW/WebP)
    const editButton = page.locator('#metadata-edit-btn');
    await expect(editButton).toBeDisabled();
    await expect(editButton).toHaveAccessibleName(/not supported/i);
  });
});
