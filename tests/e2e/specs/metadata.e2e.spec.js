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

  test('should keep long values inside the detail panel', async ({ page }) => {
    // GIVEN: a seeded photo whose path and filename are single unbreakable
    // tokens (no space to wrap at), so a value that is allowed to grow past its
    // column really does reach past the panel's edge
    const photosResponse = await page.request.get('/api/photos?page=1&limit=100');
    expect(photosResponse.ok()).toBeTruthy();
    const photosData = await photosResponse.json();
    const longNamedPhoto = (photosData.photos || []).find(
      (photo) => photo.filename === 'test_video_quicktime_keys.mp4'
    );
    test.skip(!longNamedPhoto, 'test_video_quicktime_keys.mp4 is not seeded');

    await page.locator(TestHelpers.selectors.photoCard(longNamedPhoto.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User opens the metadata sidebar
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();
    // the sidebar's width transition would otherwise be measured mid-animation
    await expect(page.locator('.viewer-sidebar')).toHaveCSS('width', '400px');

    // THEN: the file path and the filename title stay inside the panel instead
    // of growing their box past its right edge
    await expect(page.locator('#meta-filename')).toBeVisible();
    await expect(page.locator('#photo-title')).toBeVisible();

    const panel = await page.locator('.photo-info').boundingBox();
    const filePath = await page.locator('#meta-filename').boundingBox();
    const title = await page.locator('#photo-title').boundingBox();
    const sidewaysScroll = await page
      .locator('.photo-info')
      .evaluate((el) => el.scrollWidth - el.clientWidth);
    const panelRight = panel.x + panel.width;

    expect(sidewaysScroll).toBeLessThanOrEqual(0);
    expect(filePath.x + filePath.width).toBeLessThanOrEqual(panelRight);
    expect(title.x + title.width).toBeLessThanOrEqual(panelRight);
  });
});
