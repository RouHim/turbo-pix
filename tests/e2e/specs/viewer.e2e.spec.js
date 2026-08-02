import { test, expect } from '@playwright/test';
import { readFile, stat, utimes, writeFile } from 'fs/promises';
import { DatabaseSync } from 'node:sqlite';
import { TestHelpers } from '../setup/test-helpers.js';
import { TestDataManager } from '../setup/test-data-manager.js';

test.describe('Photo Viewer', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should open viewer when photo is clicked', async ({ page }) => {
    // GIVEN: User has photos loaded
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    // WHEN: User clicks on a photo
    await photos[0].click();

    // THEN: Viewer opens
    await TestHelpers.verifyViewerOpen(page);
    await expect(page.locator(TestHelpers.selectors.viewer)).toBeVisible();
  });

  test('should close viewer with Escape key', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User presses Escape
    await TestHelpers.closeViewer(page);

    // THEN: Viewer is closed
    await expect(page.locator(TestHelpers.selectors.viewer)).not.toBeVisible();
  });

  test('should close viewer with close button', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User clicks close button
    const closeBtn = page.locator(TestHelpers.selectors.closeViewerBtn);
    await expect(closeBtn).toBeVisible();
    await closeBtn.click();

    // THEN: Viewer is closed
    await expect(page.locator(TestHelpers.selectors.viewer)).not.toBeVisible();
  });

  test('should navigate to next photo with arrow key', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);

    // The seeded dataset always contains more than one photo
    expect(photos.length).toBeGreaterThan(1);

    // WHEN: User presses right arrow
    await page.keyboard.press('ArrowRight');

    // THEN: Next photo is displayed (URL is updated synchronously via replaceState)
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(firstHash);
    const secondHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(secondHash).not.toBe(firstHash);
  });

  test('should navigate to previous photo with arrow key', async ({ page }) => {
    // GIVEN: Viewer is open on second photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(1);

    await photos[1].click();
    await TestHelpers.verifyViewerOpen(page);

    const secondHash = await TestHelpers.getCurrentPhotoHash(page);

    // WHEN: User presses left arrow
    await page.keyboard.press('ArrowLeft');

    // THEN: Previous photo is displayed (URL is updated synchronously via replaceState)
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(secondHash);
    const firstHash = await TestHelpers.getCurrentPhotoHash(page);
    expect(firstHash).not.toBe(secondHash);
  });

  test('should display viewer image', async ({ page }) => {
    // GIVEN: Try to find and load a valid displayable photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    // Try each photo until we find one whose media loads (auto-wait on the
    // media element's visibility instead of guessing a fixed load duration)
    let mediaLoaded = false;
    for (let i = 0; i < Math.min(photos.length, 5) && !mediaLoaded; i++) {
      await photos[i].click();
      await TestHelpers.verifyViewerOpen(page);

      // Exactly one of the two media elements is ever shown (the other is
      // display:none), so poll either-visible — a union CSS selector would
      // resolve to BOTH elements and trip strict mode.
      try {
        await expect
          .poll(
            async () => {
              const imageVisible = await page
                .locator(TestHelpers.selectors.viewerImage)
                .isVisible();
              const videoVisible = await page
                .locator(TestHelpers.selectors.viewerVideo)
                .isVisible();
              return imageVisible || videoVisible;
            },
            { timeout: 10000 }
          )
          .toBe(true);
        mediaLoaded = true;
      } catch {
        // This photo's media did not load — close the viewer and try the next
        await TestHelpers.closeViewer(page);
      }
    }

    // THEN: At least one photo should have loaded successfully
    expect(mediaLoaded).toBe(true);
  });

  test('should show formatted metadata values in the sidebar', async ({ page }) => {
    // GIVEN: Viewer is open on a photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User opens the metadata sidebar (hidden by default)
    await page.locator('.metadata-btn').click();
    await expect(page.locator('.viewer-sidebar.show')).toBeVisible();

    // THEN: Field values render the formatted data, not the element id strings
    // (regression: setField was called with a leftover element-id first arg,
    // rendering literal "meta-filesize" etc. for every photo)
    const fileSize = await page.locator('#meta-filesize').textContent();
    expect(fileSize).not.toBeNull();
    expect(fileSize).not.toContain('meta-filesize');
    expect(fileSize).not.toBe('-');
  });

  test('should remove deleted photo from stream without manual reload', async ({ page }) => {
    // GIVEN: At least one photo card is visible in stream
    const cardsBefore = await TestHelpers.getPhotoCards(page);
    expect(cardsBefore.length).toBeGreaterThan(0);

    const firstCard = cardsBefore[0];
    const photoHash = await firstCard.getAttribute('data-photo-id');
    expect(photoHash).toBeTruthy();

    // Capture everything needed to restore the photo afterwards: the full API
    // record (to rebuild the DB row) plus the file bytes and mtime (to restore
    // the file on disk). Deletion permanently removes both, so the test would
    // otherwise mutate shared server state for every subsequent test.
    const dataManager = new TestDataManager();
    const photos = await dataManager.fetchAllPhotos();
    const deletedPhoto = photos.find((photo) => photo.hash_sha256 === photoHash);
    expect(deletedPhoto, `photo ${photoHash} should be listed by the photos API`).toBeTruthy();

    const fileBytes = await readFile(deletedPhoto.file_path);
    const fileStats = await stat(deletedPhoto.file_path);

    // WHEN: Open viewer and delete photo
    await firstCard.click();
    await TestHelpers.verifyViewerOpen(page);

    page.once('dialog', (dialog) => dialog.accept());
    await page.locator('.delete-photo-btn').click();

    // THEN: Photo card is removed from stream immediately (no reload)
    await TestHelpers.verifyViewerOpen(page);
    await expect(page.locator(TestHelpers.selectors.photoCard(photoHash))).toHaveCount(0);

    // CLEANUP: restore the deleted photo so shared server state is unchanged.
    // Deleting removes the file and the DB row, and there is no rescan API to
    // rebuild the row, so write the file back and re-insert the row directly.
    await writeFile(deletedPhoto.file_path, fileBytes);
    await utimes(deletedPhoto.file_path, fileStats.atime, fileStats.mtime);

    const db = new DatabaseSync('test-e2e-data/database/turbo-pix.db', { timeout: 5000 });
    try {
      db.prepare(
        `INSERT OR REPLACE INTO photos (
          hash_sha256, file_path, filename, file_size, mime_type,
          taken_at, width, height, orientation, duration,
          thumbnail_path, has_thumbnail, blurhash, is_favorite, semantic_vector_indexed,
          metadata, file_modified, date_indexed
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      ).run(
        deletedPhoto.hash_sha256,
        deletedPhoto.file_path,
        deletedPhoto.filename,
        deletedPhoto.file_size,
        deletedPhoto.mime_type ?? null,
        deletedPhoto.taken_at ?? null,
        deletedPhoto.width ?? null,
        deletedPhoto.height ?? null,
        deletedPhoto.orientation ?? null,
        deletedPhoto.duration ?? null,
        deletedPhoto.thumbnail_path ?? null,
        deletedPhoto.has_thumbnail ? 1 : 0,
        deletedPhoto.blurhash ?? null,
        deletedPhoto.is_favorite ? 1 : 0,
        deletedPhoto.semantic_vector_indexed ? 1 : 0,
        JSON.stringify(deletedPhoto.metadata ?? {}),
        deletedPhoto.file_modified ?? new Date().toISOString(),
        deletedPhoto.date_indexed ?? new Date().toISOString()
      );
    } finally {
      db.close();
    }

    // THEN: the photo reappears in the stream after a reload
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator(TestHelpers.selectors.photoCard(photoHash))).toHaveCount(1);
  });
});
