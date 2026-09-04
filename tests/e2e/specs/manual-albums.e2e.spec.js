import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

async function deleteAllAlbums(page) {
  const res = await page.request.get('/api/albums');
  const { albums = [] } = await res.json();
  for (const album of albums) {
    await page.request.delete(`/api/albums/${album.id}`);
  }
}

async function hashesByFilenamePrefix(page, prefix) {
  const res = await page.request.get('/api/photos?limit=200');
  const data = await res.json();
  return (data.photos || [])
    .filter((p) => p.filename?.startsWith(prefix))
    .map((p) => p.hash_sha256);
}

async function createAlbumViaApi(page, name, initial_hashes = []) {
  const res = await page.request.post('/api/albums', {
    data: { name, initial_hashes },
  });
  expect(res.status()).toBe(201);
  return res.json();
}

async function albumHashesViaApi(page, id) {
  const res = await page.request.get(`/api/albums/${id}/photos?limit=200`);
  expect(res.status()).toBe(200);
  const data = await res.json();
  return (data.photos || []).map((p) => p.hash_sha256);
}

async function cardHashes(page, count) {
  const cards = await TestHelpers.getPhotoCards(page);
  expect(cards.length).toBeGreaterThanOrEqual(count);
  return Promise.all(cards.slice(0, count).map((c) => c.getAttribute('data-photo-id')));
}

test.describe('Manual Albums', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await deleteAllAlbums(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForSearchReady(page);
  });

  test('creates an empty album and shows the empty state', async ({ page }) => {
    await page.click('[data-testid="new-album-btn"]');
    await page.fill('[data-testid="album-name-input"]', 'Empty album');
    await page.click('[data-testid="album-submit"]');

    await expect(page.locator('[data-testid="album-row"]')).toHaveCount(1);
    await expect(page.locator('[data-testid="album-row"]')).toContainText('Empty album');

    await page.click('[data-testid="album-open"]');
    await expect(page.locator('.error-state-title')).toBeVisible();
    await expect(page.locator('.photo-card')).toHaveCount(0);
  });

  test('creates an album from the active selection with exactly those photos', async ({ page }) => {
    await TestHelpers.waitForPhotosToLoad(page);
    const [first, second] = await cardHashes(page, 2);

    await page.click('[data-action="select-mode"]');
    await page.locator(`[data-photo-id="${first}"]`).click();
    await page.locator(`[data-photo-id="${second}"]`).click();

    await page.click('[data-testid="new-album-btn"]');
    // FR-002: the creation flow offers to include the selection.
    await expect(page.locator('[data-testid="album-include-selection"]')).toBeChecked();
    await page.fill('[data-testid="album-name-input"]', 'From selection');
    await page.click('[data-testid="album-submit"]');

    await expect(page.locator('[data-testid="album-row"]')).toHaveCount(1);
    await page.click('[data-testid="album-open"]');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.photo-card')).toHaveCount(2);
    await expect(page.locator(`[data-photo-id="${first}"]`)).toHaveCount(1);
    await expect(page.locator(`[data-photo-id="${second}"]`)).toHaveCount(1);
  });

  test('adds via the toolbar picker and the viewer without duplicates', async ({ page }) => {
    const album = await createAlbumViaApi(page, 'Curated');
    await TestHelpers.waitForPhotosToLoad(page);
    const [first, second] = await cardHashes(page, 2);

    // Add the first photo through the toolbar picker.
    await page.click('[data-action="select-mode"]');
    await page.locator(`[data-photo-id="${first}"]`).click();
    await page.click('[data-action="batch-add-to-album"]');
    await page.locator(`[data-testid="album-pick-row"][data-album-id="${album.id}"]`).click();
    // Selection is kept after an add (membership is not a library change).
    await expect(page.locator('#selection-bar')).toContainText('1 selected');
    await page.click('[data-action="batch-exit"]');

    // Add the second photo from the single-photo viewer.
    await TestHelpers.openViewer(page, second);
    await TestHelpers.verifyViewerOpen(page);
    await page.click('[data-action="add-to-album"]');
    await page.locator(`[data-testid="album-pick-row"][data-album-id="${album.id}"]`).click();
    await TestHelpers.closeViewer(page);

    // Re-adding an existing member is a no-op success (SC-003).
    const reAdd = await page.request.post(`/api/albums/${album.id}/members`, {
      data: { hashes: [first] },
    });
    expect(reAdd.status()).toBe(200);
    expect((await reAdd.json()).added).toBe(0);

    const members = await albumHashesViaApi(page, album.id);
    expect(members).toHaveLength(2);
    expect(members).toEqual(expect.arrayContaining([first, second]));
  });

  test('removes from the album but keeps the library photo intact', async ({ page }) => {
    await TestHelpers.waitForPhotosToLoad(page);
    const [first] = await cardHashes(page, 1);
    const album = await createAlbumViaApi(page, 'Prune me', [first]);

    await TestHelpers.goto(page);
    await page.click('[data-testid="album-open"]');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator(`[data-photo-id="${first}"]`)).toHaveCount(1);

    await page.click('[data-action="select-mode"]');
    await page.locator(`[data-photo-id="${first}"]`).click();
    await page.click('[data-action="batch-remove-from-album"]');
    await expect(page.locator(`[data-photo-id="${first}"]`)).toHaveCount(0);
    expect(await albumHashesViaApi(page, album.id)).toHaveLength(0);

    // The photo survives in the library with its metadata intact.
    await TestHelpers.navigateToView(page, 'all');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator(`[data-photo-id="${first}"]`)).toHaveCount(1);
  });

  test('renames keeping membership and deletes keeping photos', async ({ page }) => {
    await TestHelpers.waitForPhotosToLoad(page);
    const [first] = await cardHashes(page, 1);
    const album = await createAlbumViaApi(page, 'Old name', [first]);

    await TestHelpers.goto(page);
    await expect(page.locator('[data-testid="album-row"]')).toHaveCount(1);

    // Rename through the sidebar dialog.
    await page.click('[data-testid="album-rename"]');
    await page.fill('[data-testid="album-name-input"]', 'New name');
    await page.click('[data-testid="album-submit"]');
    await expect(page.locator('[data-testid="album-row"]')).toContainText('New name');
    expect(await albumHashesViaApi(page, album.id)).toEqual([first]);

    // Open the album, then delete it: navigation lands on a safe view and
    // every photo stays in the library (SC-005).
    await page.click('[data-testid="album-open"]');
    await expect(page).toHaveURL(/album=/);
    await page.click('[data-testid="album-delete"]');
    await expect(page.locator('[data-testid="album-row"]')).toHaveCount(0);
    const url = new URL(page.url());
    expect(url.searchParams.get('album')).toBeNull();

    const clusterHashes = await hashesByFilenamePrefix(page, 'cluster_');
    expect(clusterHashes.length).toBeGreaterThan(0);
    await TestHelpers.navigateToView(page, 'all');
    await TestHelpers.waitForPhotosToLoad(page);
    for (const hash of clusterHashes.slice(0, 3)) {
      await expect(page.locator(`[data-photo-id="${hash}"]`)).toHaveCount(1);
    }
  });

  test('rejects an empty album name and saves nothing', async ({ page }) => {
    await page.click('[data-testid="new-album-btn"]');
    await page.fill('[data-testid="album-name-input"]', '   ');
    await page.click('[data-testid="album-submit"]');
    await expect(page.locator('[data-testid="album-error"]')).toBeVisible();

    const res = await page.request.get('/api/albums');
    const { albums = [] } = await res.json();
    expect(albums.length).toBe(0);
  });

  test('albums and membership survive a page reload', async ({ page }) => {
    await TestHelpers.waitForPhotosToLoad(page);
    const [first] = await cardHashes(page, 1);
    await createAlbumViaApi(page, 'Persistent', [first]);

    await page.reload();
    await TestHelpers.waitForSearchReady(page);
    await expect(page.locator('[data-testid="album-row"]')).toHaveCount(1);

    const res = await page.request.get('/api/albums');
    const { albums = [] } = await res.json();
    expect(albums).toHaveLength(1);
    expect(await albumHashesViaApi(page, albums[0].id)).toEqual([first]);
  });
});
