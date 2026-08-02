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

    // WHEN: User opens the metadata sidebar (it starts hidden per viewer contract)
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();

    // THEN: The metadata panel is displayed
    await expect(page.locator('.photo-info')).toBeVisible();
  });

  test('should show photo information', async ({ page }) => {
    // GIVEN: Viewer is open
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User opens the metadata sidebar
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();

    // THEN: Photo info rows (date/size/camera/location) are present
    await expect(page.locator('.photo-meta .meta-item')).toHaveCount(4);
  });

  test('should display EXIF data when available', async ({ page }) => {
    // GIVEN: a photo with camera EXIF exists (sample_with_exif.jpg is seeded
    // by global-setup). Its EXIF taken_at is 2024-01-01, so its grid position
    // is not guaranteed — locate it by hash instead of clicking photos[0].
    const photosResponse = await page.request.get('/api/photos?page=1&limit=100');
    expect(photosResponse.ok()).toBeTruthy();
    const photosData = await photosResponse.json();
    const exifPhoto = (photosData.photos || []).find((photo) => {
      const camera = photo.metadata?.camera || {};
      return camera.make || camera.model || camera.lens_make || camera.lens_model;
    });
    test.skip(!exifPhoto, 'No seeded photo carries camera EXIF (sample_with_exif.jpg missing)');

    const exifCard = page.locator(TestHelpers.selectors.photoCard(exifPhoto.hash_sha256));
    await exifCard.click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User opens the metadata sidebar
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();

    // THEN: EXIF-driven sections render
    await expect(page.locator('#camera-section')).toBeVisible();
  });
});
