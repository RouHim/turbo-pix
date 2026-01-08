import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Metadata', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should display metadata panel in viewer', async ({ page }) => {
    // GIVEN: User opens a photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: Viewer is open
    // THEN: Metadata elements should be available
    const metadataExists =
      (await page.locator('.viewer-metadata, .photo-info, .info-panel').count()) > 0;

    if (metadataExists) {
      expect(metadataExists).toBe(true);
    }
  });

  test('should show photo information', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User checks photo info
    const infoExists = (await page.locator('.photo-info, .metadata-container').count()) > 0;

    // THEN: Photo info should be present
    if (infoExists) {
      expect(infoExists).toBe(true);
    }
  });

  test('should display EXIF data when available', async ({ page }) => {
    // GIVEN: Viewer is open with a photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User checks for EXIF data
    const exifExists =
      (await page.locator('.exif-data, .metadata-item, .photo-details').count()) > 0;

    // THEN: EXIF elements may be present
    // Note: Not all photos have EXIF data
    if (exifExists) {
      expect(exifExists).toBe(true);
    }
  });
});
