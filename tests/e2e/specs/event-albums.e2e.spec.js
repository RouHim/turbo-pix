import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

function isoDateDaysFromNow(days) {
  return new Date(Date.now() + days * 24 * 60 * 60 * 1000).toISOString().split('T')[0];
}

async function deleteAllEventAlbums(page) {
  const res = await page.request.get('/api/event-albums');
  const { event_albums = [] } = await res.json();
  for (const album of event_albums) {
    await page.request.delete(`/api/event-albums/${album.id}`);
  }
}

async function hashesByFilenamePrefix(page, prefix) {
  const res = await page.request.get('/api/photos?limit=200');
  const data = await res.json();
  return (data.photos || [])
    .filter((p) => p.filename?.startsWith(prefix))
    .map((p) => p.hash_sha256);
}

async function createAlbumViaApi(page, name, start, end, location = null) {
  const res = await page.request.post('/api/event-albums', {
    data: { name, start_date: start, end_date: end, location },
  });
  expect(res.status()).toBe(201);
  return res.json();
}

test.describe('Event Albums', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await deleteAllEventAlbums(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForSearchReady(page);
  });

  test('creates an album and shows only the matching photos', async ({ page }) => {
    const start = isoDateDaysFromNow(-10);
    const end = isoDateDaysFromNow(1);
    const archiveHashes = await hashesByFilenamePrefix(page, 'archive_');
    expect(archiveHashes.length).toBeGreaterThan(0);

    await page.click('[data-testid="new-album-btn"]');
    await page.fill('[data-testid="album-name-input"]', 'Recent photos');
    await page.fill('[data-testid="album-start-input"]', start);
    await page.fill('[data-testid="album-end-input"]', end);
    await page.click('[data-testid="album-submit"]');

    const rows = page.locator('[data-testid="event-album-row"]');
    await expect(rows).toHaveCount(1);

    await page.click('[data-testid="event-album-open"]');
    await TestHelpers.waitForPhotosToLoad(page);
    const cards = await TestHelpers.getPhotoCards(page);
    expect(cards.length).toBeGreaterThanOrEqual(1);

    // The archive photos (~400 days ago) must NOT appear.
    for (const hash of archiveHashes) {
      await expect(page.locator(`[data-photo-id="${hash}"]`)).toHaveCount(0);
    }
  });

  test('rejects an empty name and an inverted date range client-side', async ({ page }) => {
    await page.click('[data-testid="new-album-btn"]');

    // Empty name
    await page.fill('[data-testid="album-start-input"]', isoDateDaysFromNow(-10));
    await page.fill('[data-testid="album-end-input"]', isoDateDaysFromNow(1));
    await page.click('[data-testid="album-submit"]');
    await expect(page.locator('[data-testid="album-error"]')).toBeVisible();

    // Inverted range
    await page.fill('[data-testid="album-name-input"]', 'Bad range');
    await page.fill('[data-testid="album-start-input"]', isoDateDaysFromNow(1));
    await page.fill('[data-testid="album-end-input"]', isoDateDaysFromNow(-10));
    await page.click('[data-testid="album-submit"]');
    await expect(page.locator('[data-testid="album-error"]')).toBeVisible();

    // Nothing persisted.
    const res = await page.request.get('/api/event-albums');
    const { event_albums = [] } = await res.json();
    expect(event_albums.length).toBe(0);
  });

  test('editing the date range re-computes membership', async ({ page }) => {
    const album = await createAlbumViaApi(
      page,
      'Window',
      isoDateDaysFromNow(-10),
      isoDateDaysFromNow(1)
    );
    const clusterHashes = await hashesByFilenamePrefix(page, 'cluster_');
    const archiveHashes = await hashesByFilenamePrefix(page, 'archive_');
    expect(clusterHashes.length).toBeGreaterThan(0);
    expect(archiveHashes.length).toBeGreaterThan(0);

    await TestHelpers.goto(page);
    await page.click('[data-testid="event-album-open"]');
    await TestHelpers.waitForPhotosToLoad(page);
    for (const hash of clusterHashes) {
      await expect(page.locator(`[data-photo-id="${hash}"]`)).toHaveCount(1);
    }

    // Re-point the album at the archive era (~400 days ago).
    const res = await page.request.patch(`/api/event-albums/${album.id}`, {
      data: {
        name: 'Window',
        start_date: isoDateDaysFromNow(-500),
        end_date: isoDateDaysFromNow(-300),
        location: null,
      },
    });
    expect(res.status()).toBe(200);

    await TestHelpers.goto(page);
    await page.click('[data-testid="event-album-open"]');
    await TestHelpers.waitForPhotosToLoad(page);
    for (const hash of archiveHashes) {
      await expect(page.locator(`[data-photo-id="${hash}"]`)).toHaveCount(1);
    }
    for (const hash of clusterHashes) {
      await expect(page.locator(`[data-photo-id="${hash}"]`)).toHaveCount(0);
    }
  });

  test('shows an empty state for an album with no matches', async ({ page }) => {
    await createAlbumViaApi(page, 'Empty', '3000-01-01', '3000-01-02');
    await TestHelpers.goto(page);
    await page.click('[data-testid="event-album-open"]');
    // Empty state (not the error state): a title renders and no cards or
    // retry button. Locale-independent assertions.
    await expect(page.locator('.error-state-title')).toBeVisible();
    await expect(page.locator('.error-state-button')).toHaveCount(0);
    await expect(page.locator('.photo-card')).toHaveCount(0);
  });

  test('deletes an album without deleting photos', async ({ page }) => {
    await createAlbumViaApi(page, 'Temp', isoDateDaysFromNow(-10), isoDateDaysFromNow(1));

    await TestHelpers.goto(page);
    await expect(page.locator('[data-testid="event-album-row"]')).toHaveCount(1);
    await page.click('[data-testid="event-album-delete"]');
    await expect(page.locator('[data-testid="event-album-row"]')).toHaveCount(0);

    // Photos are intact: cluster photos still listed in All Photos.
    const clusterHashes = await hashesByFilenamePrefix(page, 'cluster_');
    expect(clusterHashes.length).toBeGreaterThan(0);
    await TestHelpers.navigateToView(page, 'all');
    await TestHelpers.waitForPhotosToLoad(page);
    for (const hash of clusterHashes.slice(0, 3)) {
      await expect(page.locator(`[data-photo-id="${hash}"]`)).toHaveCount(1);
    }
  });

  test('leaves the album when navigating to another view', async ({ page }) => {
    await createAlbumViaApi(page, 'Temp', isoDateDaysFromNow(-10), isoDateDaysFromNow(1));
    await TestHelpers.goto(page);
    await page.click('[data-testid="event-album-open"]');
    await expect(page).toHaveURL(/album=/);
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.verifyActiveView(page, 'favorites');
    const url = new URL(page.url());
    expect(url.searchParams.get('album')).toBeNull();
  });
});
