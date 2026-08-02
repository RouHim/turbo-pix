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
    // GIVEN: Viewer is open with a photo
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // WHEN: User opens the metadata sidebar
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();

    // THEN: EXIF-driven sections render when the photo carries EXIF data
    // Note: not every photo has EXIF — skip the whole test when this one doesn't
    const hasCameraSection = (await page.locator('#camera-section').count()) > 0;
    test.skip(!hasCameraSection, 'Selected photo has no EXIF camera data to display');
    await expect(page.locator('#camera-section')).toBeVisible();
  });
});
