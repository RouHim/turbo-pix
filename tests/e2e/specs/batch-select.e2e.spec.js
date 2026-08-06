import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';
import { TestDataManager } from '../setup/test-data-manager.js';

// NOTE: this spec runs FIRST alphabetically (workers: 1) and MUST restore the
// shared seed state it consumes: favorites toggled back, dates shifted back,
// 2 photos deleted (permanently — nothing asserts a minimum photo count),
// housekeeping candidate reseeded, pending collages reseeded.

const selectionBar = '#selection-bar';

const waitForCollagesOrEmpty = async (page) => {
  await page.waitForFunction(
    () => document.querySelector('.photo-card') || document.querySelector('.empty-state')
  );
};

const scrollMainContentToBottom = async (page) => {
  await page.evaluate(() => {
    const container = document.querySelector('.main-content');
    if (container) container.scrollTop = container.scrollHeight;
  });
};

test.describe('Batch select + actions', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('enter and exit selection mode', async ({ page }) => {
    // GIVEN: the All view with photos loaded
    // WHEN: the user enters selection mode
    await page.click('[data-action="select-mode"]');

    // THEN: the action bar is visible and every action is disabled on an
    // empty selection (SC-010)
    await expect(page.locator(selectionBar)).toBeVisible();
    await expect(page.locator('[data-action="batch-delete"]')).toBeDisabled();

    // WHEN: the user exits selection mode
    await page.click('[data-action="batch-exit"]');

    // THEN: the bar is gone
    await expect(page.locator(selectionBar)).toHaveCount(0);
  });

  test('tap selection, count, and select-all-visible', async ({ page }) => {
    // GIVEN: selection mode is active
    await page.click('[data-action="select-mode"]');
    await expect(page.locator(selectionBar)).toBeVisible();

    // WHEN: three cards are tapped
    const cards = await TestHelpers.getPhotoCards(page);
    expect(cards.length).toBeGreaterThanOrEqual(3);
    await cards[0].click();
    await cards[1].click();
    await cards[2].click();

    // THEN: the count reflects the selection and delete becomes enabled
    await expect(page.locator(selectionBar)).toContainText('3 selected');
    await expect(page.locator('[data-action="batch-delete"]')).toBeEnabled();

    // WHEN: select-all-visible is triggered
    await page.click('[data-action="batch-select-all"]');
    const visibleCount = (await TestHelpers.getPhotoCards(page)).length;

    // THEN: every visible card is selected (FR-014)
    await expect(page.locator(selectionBar)).toContainText(`${visibleCount} selected`);

    // WHEN: select-all-visible is triggered again
    await page.click('[data-action="batch-select-all"]');

    // THEN: the selection is empty again
    await expect(page.locator(selectionBar)).toContainText('0 selected');
    await expect(page.locator('[data-action="batch-delete"]')).toBeDisabled();

    await page.click('[data-action="batch-exit"]');
    await expect(page.locator(selectionBar)).toHaveCount(0);
  });

  test('shift-click range selection and Escape exit', async ({ page }) => {
    // GIVEN: selection mode is active
    await page.click('[data-action="select-mode"]');
    await expect(page.locator(selectionBar)).toBeVisible();

    // WHEN: the user taps card A, then Shift+clicks card C two rows down
    const cards = await TestHelpers.getPhotoCards(page);
    expect(cards.length).toBeGreaterThanOrEqual(3);
    await cards[0].click();
    await page.keyboard.down('Shift');
    await cards[2].click();
    await page.keyboard.up('Shift');

    // THEN: the whole range is selected (FR-003)
    await expect(page.locator(selectionBar)).toContainText('3 selected');

    // WHEN: Escape is pressed
    await page.keyboard.press('Escape');

    // THEN: selection mode exits (FR-003) and re-entering starts empty
    await expect(page.locator(selectionBar)).toHaveCount(0);
    await page.click('[data-action="select-mode"]');
    await expect(page.locator(selectionBar)).toContainText('0 selected');
    await page.click('[data-action="batch-exit"]');
  });

  test('batch favorite and unfavorite', async ({ page }) => {
    // GIVEN: two cluster photos (pinned dates, not favorited by default)
    const dataManager = new TestDataManager();
    const allPhotos = await dataManager.fetchAllPhotos();
    const clusterHashes = allPhotos
      .filter((p) => p.filename?.startsWith('cluster_'))
      .slice(0, 2)
      .map((p) => p.hash_sha256);
    expect(clusterHashes.length).toBe(2);
    const [hashA, hashB] = clusterHashes;

    // WHEN: both are selected and added to favorites
    await page.click('[data-action="select-mode"]');
    await page.locator(TestHelpers.selectors.photoCard(hashA)).first().click();
    await page.locator(TestHelpers.selectors.photoCard(hashB)).first().click();
    await expect(page.locator(selectionBar)).toContainText('2 selected');
    const addResponse = TestHelpers.waitForApiCall(page, '/api/photos/batch/favorite');
    await page.click('[data-action="batch-add-favorite"]');
    await addResponse;

    // THEN: both appear in the Favorites view (SC-005)
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator(TestHelpers.selectors.photoCard(hashA))).toHaveCount(1);
    await expect(page.locator(TestHelpers.selectors.photoCard(hashB))).toHaveCount(1);

    // WHEN: both are selected again and removed from favorites
    await page.click('[data-action="select-mode"]');
    await page.click('[data-action="batch-select-all"]');
    const removeResponse = TestHelpers.waitForApiCall(page, '/api/photos/batch/favorite');
    await page.click('[data-action="batch-remove-favorite"]');
    await removeResponse;

    // THEN: the favorites grid empties (cards spliced + selection auto-exits)
    await expect(page.locator(TestHelpers.selectors.photoCard(hashA))).toHaveCount(0);
    await expect(page.locator(selectionBar)).toHaveCount(0);

    // AND: both photos are back to non-favorite in the library (restores the
    // shared seed state)
    const afterFavorite = await dataManager.fetchAllPhotos();
    for (const hash of [hashA, hashB]) {
      const photo = afterFavorite.find((p) => p.hash_sha256 === hash);
      expect(photo, `photo ${hash} should exist`).toBeTruthy();
      expect(photo.is_favorite, `photo ${hash} should not be favorite`).toBeFalsy();
    }
  });

  test('batch date-shift applies and reports', async ({ page }) => {
    // GIVEN: two cluster photos with pinned taken_at dates
    const dataManager = new TestDataManager();
    const hashes = await dataManager.fetchTestPhotoHashes();
    const allPhotos = await dataManager.fetchAllPhotos();
    const clusterHashes = allPhotos
      .filter((p) => p.filename?.startsWith('cluster_'))
      .slice(0, 2)
      .map((p) => p.hash_sha256);
    expect(clusterHashes.length).toBe(2);
    const before = allPhotos.filter((p) => clusterHashes.includes(p.hash_sha256));
    for (const photo of before) {
      expect(photo.taken_at, `${photo.filename} must have a taken date`).toBeTruthy();
    }

    // WHEN: both are selected and shifted by -1 day
    await page.click('[data-action="select-mode"]');
    await page.locator(TestHelpers.selectors.photoCard(clusterHashes[0])).first().click();
    await page.locator(TestHelpers.selectors.photoCard(clusterHashes[1])).first().click();
    await expect(page.locator(selectionBar)).toContainText('2 selected');
    await page.click('[data-action="batch-date-shift"]');
    await page.fill('#batch-days-input', '-1');
    const shiftResponse = TestHelpers.waitForApiCall(page, '/api/photos/batch/date-shift');
    await page.click('[data-action="batch-date-shift-apply"]');
    await shiftResponse;

    // THEN: each taken_at moved back exactly one day (same time of day)
    const afterShift = await dataManager.fetchAllPhotos();
    for (const photo of before) {
      const shifted = afterShift.find((p) => p.hash_sha256 === photo.hash_sha256);
      expect(shifted).toBeTruthy();
      const beforeMs = Date.parse(photo.taken_at);
      const shiftedMs = Date.parse(shifted.taken_at);
      expect(shiftedMs).toBe(beforeMs - 24 * 60 * 60 * 1000);
    }

    // AND: shifting back +1 day restores the pinned dates (seed cleanup).
    // The selection survives the reload (FR-002) — the same two hashes stay
    // selected, so no select-all needed (that would shift every visible card).
    await page.click('[data-action="batch-date-shift"]');
    await page.fill('#batch-days-input', '1');
    const restoreResponse = TestHelpers.waitForApiCall(page, '/api/photos/batch/date-shift');
    await page.click('[data-action="batch-date-shift-apply"]');
    await restoreResponse;

    const restored = await dataManager.fetchAllPhotos();
    for (const photo of before) {
      const back = restored.find((p) => p.hash_sha256 === photo.hash_sha256);
      expect(Date.parse(back.taken_at)).toBe(Date.parse(photo.taken_at));
    }
    await page.click('[data-action="batch-exit"]');
  });

  test('batch delete with confirmation', async ({ page }) => {
    // GIVEN: two cluster photos
    const dataManager = new TestDataManager();
    const allPhotos = await dataManager.fetchAllPhotos();
    const clusterHashes = allPhotos
      .filter((p) => p.filename?.startsWith('cluster_'))
      .slice(0, 2)
      .map((p) => p.hash_sha256);
    expect(clusterHashes.length).toBe(2);

    // WHEN: the user confirms the count-stating dialog and deletes
    await page.click('[data-action="select-mode"]');
    await page.locator(TestHelpers.selectors.photoCard(clusterHashes[0])).first().click();
    await page.locator(TestHelpers.selectors.photoCard(clusterHashes[1])).first().click();
    await expect(page.locator(selectionBar)).toContainText('2 selected');
    page.once('dialog', (dialog) => dialog.accept());
    const deleteResponse = TestHelpers.waitForApiCall(page, '/api/photos/batch/delete');
    await page.click('[data-action="batch-delete"]');
    await deleteResponse;

    // THEN: the cards disappear from the grid and the photos are permanently
    // gone from the library (SC-002)
    await expect(page.locator(TestHelpers.selectors.photoCard(clusterHashes[0]))).toHaveCount(0);
    await expect(page.locator(TestHelpers.selectors.photoCard(clusterHashes[1]))).toHaveCount(0);
    await expect(page.locator(selectionBar)).toHaveCount(0); // selection auto-exited
    const afterDelete = await dataManager.fetchAllPhotos();
    for (const hash of clusterHashes) {
      expect(afterDelete.find((p) => p.hash_sha256 === hash)).toBeUndefined();
    }
  });

  test('housekeeping batch keep', async ({ page }) => {
    // GIVEN: housekeeping candidates exist (seeded by global-setup)
    await TestHelpers.navigateToView(page, 'housekeeping');
    // Wait for the candidates grid OR the empty state (the view loads async;
    // reading cards immediately races the fetch and sees a skeleton).
    await page.waitForFunction(
      () => document.querySelector('.photo-card') || document.querySelector('.no-photos')
    );
    let cards = await TestHelpers.getPhotoCards(page);
    test.skip(cards.length === 0, 'No housekeeping candidates in dataset');
    expect(cards.length).toBeGreaterThan(0);
    const keptHash = await cards[0].getAttribute('data-photo-id');
    expect(keptHash).toBeTruthy();

    // WHEN: the user selects the first candidate and triggers batch keep
    await page.click('[data-action="select-mode"]');
    await page.locator(TestHelpers.selectors.photoCard(keptHash)).first().click();
    await expect(page.locator(selectionBar)).toContainText('1 selected');
    const keepResponse = TestHelpers.waitForApiCall(
      page,
      '/api/housekeeping/candidates/batch-remove'
    );
    await page.click('[data-action="batch-keep"]');
    await keepResponse;

    // THEN: the candidate card is dismissed without deleting the photo (SC-006)
    await expect(page.locator(TestHelpers.selectors.photoCard(keptHash))).toHaveCount(0);
    const dataManager = new TestDataManager();
    const allPhotos = await dataManager.fetchAllPhotos();
    expect(allPhotos.find((p) => p.hash_sha256 === keptHash)).toBeTruthy();

    // AND: the seed is restored for housekeeping.e2e.spec.js
    await TestDataManager.reseedHousekeepingCandidate();
  });

  test('collages batch accept and reject', async ({ page }) => {
    // GIVEN: at least two pending collages (seeded by global-setup)
    await TestHelpers.navigateToView(page, 'collages');
    await waitForCollagesOrEmpty(page);
    let cards = await TestHelpers.getPhotoCards(page);
    test.skip(cards.length < 2, 'Fewer than 2 pending collages in dataset');
    expect(cards.length).toBeGreaterThanOrEqual(2);

    // WHEN: all visible cards are selected and batch-accepted (no confirm)
    await page.click('[data-action="select-mode"]');
    await page.click('[data-action="batch-select-all"]');
    const acceptResponse = TestHelpers.waitForApiCall(page, '/api/collages/batch-accept');
    await page.click('[data-action="batch-accept"]');
    await acceptResponse;

    // THEN: every card is resolved and disappears from the pending list
    await expect(page.locator('.photo-card')).toHaveCount(0);
    await expect(page.locator(selectionBar)).toHaveCount(0); // selection auto-exited

    // AND: the seed is restored for collages.e2e.spec.js
    TestDataManager.reseedPendingCollages(2);
    await TestHelpers.goto(page, '/collages');
    await waitForCollagesOrEmpty(page);
    cards = await TestHelpers.getPhotoCards(page);
    expect(cards.length).toBeGreaterThanOrEqual(2);

    // WHEN: one card is selected and batch-rejected with confirmation
    const rejectHash = await cards[0].getAttribute('data-photo-id');
    await page.click('[data-action="select-mode"]');
    await page.locator(TestHelpers.selectors.photoCard(rejectHash)).first().click();
    await expect(page.locator(selectionBar)).toContainText('1 selected');
    page.once('dialog', (dialog) => dialog.accept());
    const rejectResponse = TestHelpers.waitForApiCall(page, '/api/collages/batch-reject');
    await page.click('[data-action="batch-reject"]');
    await rejectResponse;

    // THEN: the rejected card is gone (FR-010 confirmation honored)
    await expect(page.locator(TestHelpers.selectors.photoCard(rejectHash))).toHaveCount(0);

    // AND: the seed is topped up again (one card remains from the last reseed)
    TestDataManager.reseedPendingCollages(2);
  });

  test('batch export downloads an archive', async ({ page }) => {
    // GIVEN: one JPEG card and the seeded video card (SC-004 mixed formats)
    const dataManager = new TestDataManager();
    const allPhotos = await dataManager.fetchAllPhotos();
    const videoHash = allPhotos.find((p) => p.filename === 'test_video.mp4')?.hash_sha256;
    const jpgHash = allPhotos
      .filter((p) => p.filename?.startsWith('cluster_'))
      .map((p) => p.hash_sha256)[0];
    expect(videoHash).toBeTruthy();
    expect(jpgHash).toBeTruthy();

    await page.click('[data-action="select-mode"]');
    await page.locator(TestHelpers.selectors.photoCard(jpgHash)).first().click();
    await expect(page.locator(selectionBar)).toContainText('1 selected');

    // The video sorts first (fresh creation time): scroll the grid container
    // until its card is loaded, then select it too.
    await page.waitForFunction(
      (hash) => {
        const container = document.querySelector('.main-content');
        if (container) container.scrollTop = container.scrollHeight;
        return !!document.querySelector(`[data-photo-id="${hash}"]`);
      },
      videoHash,
      { timeout: 30000 }
    );
    await page.locator(TestHelpers.selectors.photoCard(videoHash)).scrollIntoViewIfNeeded();
    await page.locator(TestHelpers.selectors.photoCard(videoHash)).click();
    await expect(page.locator(selectionBar)).toContainText('2 selected');

    // WHEN: the user exports the selection
    const downloadPromise = page.waitForEvent('download');
    await page.click('[data-action="batch-export"]');
    const download = await downloadPromise;

    // THEN: a ZIP archive is downloaded (SC-004)
    expect(download.suggestedFilename()).toMatch(/\.zip$/);
    const stream = await download.createReadStream();
    const chunks = [];
    for await (const chunk of stream) {
      chunks.push(chunk);
      if (chunks.length > 4) break;
    }
    const head = Buffer.concat(chunks);
    expect(head.length).toBeGreaterThan(0);
    expect(head.subarray(0, 4).toString('latin1')).toBe('PK\x03\x04');

    // Clean up: exit selection mode
    await page.keyboard.press('Escape');
    await expect(page.locator(selectionBar)).toHaveCount(0);
  });
});
