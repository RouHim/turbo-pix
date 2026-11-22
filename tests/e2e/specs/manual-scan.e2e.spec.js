import { test, expect } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { TestHelpers } from '../setup/test-helpers.js';

const PHOTO_ROOT = path.resolve('test-e2e-data', 'photos');

test.describe.configure({ timeout: 420_000 });

test.describe('Manual scan', () => {
  test('indexes a newly added file without a restart and stays idempotent', async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);

    // GIVEN a seeded library and one favorite that must survive the scan
    const seededImage = fs.readdirSync(PHOTO_ROOT).find((name) => /\.(jpe?g|png)$/i.test(name));
    test.skip(!seededImage, 'No seeded image available to copy');

    const targetName = `manual-scan-${Date.now()}${path.extname(seededImage)}`;
    const targetPath = path.join(PHOTO_ROOT, targetName);

    // A retry may inherit a still-running cycle from the previous attempt.
    await expect
      .poll(
        async () => {
          const response = await page.request.get('/api/indexing/status');
          return (await response.json()).is_indexing;
        },
        { timeout: 300_000, intervals: [500, 1000] }
      )
      .toBe(false);

    const firstPhotoResponse = await page.request.get('/api/photos?limit=1');
    expect(firstPhotoResponse.ok()).toBe(true);
    const firstPhoto = (await firstPhotoResponse.json()).photos[0];
    const favoriteHash = firstPhoto.hash_sha256;
    const originalFavorite = firstPhoto.is_favorite === true;
    await page.request.put(`/api/photos/${favoriteHash}/favorite`, {
      data: { is_favorite: true },
    });

    let newHash = null;
    try {
      fs.copyFileSync(path.join(PHOTO_ROOT, seededImage), targetPath);

      // WHEN the user triggers the scan from Housekeeping
      await TestHelpers.goto(page);
      await TestHelpers.waitForPhotosToLoad(page);
      await TestHelpers.navigateToView(page, 'housekeeping');

      const trigger = page.locator(TestHelpers.selectors.action('scan-library'));
      await expect(trigger).toBeEnabled();
      const [scanResponse] = await Promise.all([
        page.waitForResponse(
          (response) =>
            response.url().includes('/api/indexing/scan') && response.request().method() === 'POST'
        ),
        trigger.click(),
      ]);

      // THEN the endpoint accepts and reports the run immediately
      expect(scanResponse.status()).toBe(202);
      const scanBody = await scanResponse.json();
      expect(scanBody.is_indexing).toBe(true);

      // AND a second request is accepted while the run is active (single-flight
      // attach semantics are pinned by the Rust trigger test)
      const attached = await page.request.post('/api/indexing/scan');
      expect(attached.status()).toBe(202);
      const attachedBody = await attached.json();
      expect(attachedBody.is_indexing).toBe(true);

      // AND the new file is indexed under its path-derived hash after phase 1
      await expect
        .poll(
          async () => {
            const response = await page.request.get(
              `/api/photos?q=${encodeURIComponent(targetName)}&limit=10`
            );
            const body = await response.json();
            newHash = body.photos?.[0]?.hash_sha256 ?? null;
            return newHash;
          },
          { timeout: 30_000, intervals: [250, 500, 1000] }
        )
        .not.toBeNull();

      // AND the full cycle finishes on its own (the CLIP phases are slow on
      // CPU, so this is the long tail; leaving it running would leak into
      // later specs)
      await expect
        .poll(
          async () => {
            const response = await page.request.get('/api/indexing/status');
            return (await response.json()).is_indexing;
          },
          { timeout: 300_000, intervals: [500, 1000] }
        )
        .toBe(false);

      // AND the favorite the scan found in the DB is still set
      const favoriteResponse = await page.request.get(`/api/photos/${favoriteHash}`);
      expect((await favoriteResponse.json()).is_favorite).toBe(true);

      // AND the grid shows the new photo without a process restart
      await TestHelpers.navigateToView(page, 'all');
      await TestHelpers.waitForSearchReady(page);
      await page.fill(TestHelpers.selectors.searchInput, targetName);
      await page.keyboard.press('Enter');
      await expect(page.locator(`[data-photo-id="${newHash}"]`)).toBeVisible({
        timeout: 30_000,
      });
    } finally {
      if (newHash) {
        await page.request.delete(`/api/photos/${newHash}`);
      }
      fs.rmSync(targetPath, { force: true });
      await page.request.put(`/api/photos/${favoriteHash}/favorite`, {
        data: { is_favorite: originalFavorite },
      });
    }
  });
});
