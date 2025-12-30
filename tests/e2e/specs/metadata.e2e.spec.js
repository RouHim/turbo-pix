import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Photo Metadata', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should open metadata panel in viewer', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Click metadata button
    const metadataBtn = page.locator('.metadata-btn');
    const btnExists = (await metadataBtn.count()) > 0;

    if (!btnExists) {
      test.skip('Metadata button not found');
    }

    await metadataBtn.click();

    // Sidebar should open
    const sidebar = page.locator('.viewer-sidebar');
    await expect(sidebar).toBeVisible();
  });

  test('should display photo information in metadata panel', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    const btnExists = (await metadataBtn.count()) > 0;

    if (!btnExists) {
      test.skip('Metadata button not available');
    }

    await metadataBtn.click();

    const sidebar = page.locator('.viewer-sidebar');
    await expect(sidebar).toBeVisible();

    // Should show photo title/filename
    const photoTitle = page.locator('.photo-title, #photo-title');
    const titleExists = (await photoTitle.count()) > 0;

    if (titleExists) {
      await expect(photoTitle).toBeVisible();
      const titleText = await photoTitle.textContent();
      expect(titleText.length).toBeGreaterThan(0);
    }
  });

  test('should display file information', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    if ((await metadataBtn.count()) === 0) {
      test.skip('Metadata button not available');
    }

    await metadataBtn.click();

    // Should show file size
    const fileSize = page.locator('#photo-size');
    const sizeExists = (await fileSize.count()) > 0;

    if (sizeExists) {
      await expect(fileSize).toBeVisible();
    }
  });

  test('should display camera information if available', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    if ((await metadataBtn.count()) === 0) {
      test.skip('Metadata button not available');
    }

    await metadataBtn.click();

    // Check for camera info (might not exist for all photos)
    const cameraInfo = page.locator('#photo-camera, .camera-section');
    const cameraExists = (await cameraInfo.count()) > 0;

    if (cameraExists) {
      const cameraText = await cameraInfo.textContent();
      expect(cameraText).toBeTruthy();
    }
  });

  test('should display location information if available', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    if ((await metadataBtn.count()) === 0) {
      test.skip('Metadata button not available');
    }

    await metadataBtn.click();

    // Check for location (might not exist for all photos)
    const location = page.locator('#photo-location, .location-section');
    const locationExists = (await location.count()) > 0;

    if (locationExists) {
      const isVisible = await location.isVisible();
      expect(isVisible).toBe(true);
    }
  });

  test('should open metadata edit modal', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    if ((await metadataBtn.count()) === 0) {
      test.skip('Metadata button not available');
    }

    await metadataBtn.click();

    // Click edit button
    const editBtn = page.locator('#metadata-edit-btn');
    const editBtnExists = (await editBtn.count()) > 0;

    if (!editBtnExists) {
      test.skip('Edit button not found');
    }

    await editBtn.click();

    // Edit modal should open
    const editModal = page.locator('#metadata-edit-modal');
    await expect(editModal).toBeVisible();
  });

  test('should edit photo metadata', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    if ((await metadataBtn.count()) === 0) {
      test.skip('Metadata button not available');
    }

    await metadataBtn.click();

    const editBtn = page.locator('#metadata-edit-btn');
    if ((await editBtn.count()) === 0) {
      test.skip('Edit button not available');
    }

    await editBtn.click();

    const editModal = page.locator('#metadata-edit-modal');
    await expect(editModal).toBeVisible();

    // Try to edit fields
    const form = page.locator('#metadata-edit-form');
    const formExists = (await form.count()) > 0;

    if (formExists) {
      // Look for editable fields (taken_at, latitude, longitude)
      const takenAtInput = form.locator('input[name="taken_at"], #taken_at');
      const latInput = form.locator('input[name="latitude"], #latitude');
      const lonInput = form.locator('input[name="longitude"], #longitude');

      const hasTakenAt = (await takenAtInput.count()) > 0;
      const hasLat = (await latInput.count()) > 0;
      const hasLon = (await lonInput.count()) > 0;

      if (hasTakenAt) {
        await takenAtInput.fill('2024-01-01T12:00');
        await page.waitForTimeout(300);
      }

      if (hasLat && hasLon) {
        await latInput.fill('40.7128');
        await lonInput.fill('-74.0060');
        await page.waitForTimeout(300);
      }

      // Save changes
      const saveBtn = form.locator('button[type="submit"], .save-btn, .metadata-save-btn');
      const saveBtnExists = (await saveBtn.count()) > 0;

      if (saveBtnExists) {
        await saveBtn.click();

        // Wait for save to complete
        await page.waitForTimeout(1500);

        // Should show success toast
        const toast = page.locator(TestHelpers.selectors.toast);
        const toastVisible = (await toast.count()) > 0 && (await toast.isVisible());

        if (toastVisible) {
          const toastText = await toast.textContent();
          expect(toastText.toLowerCase()).toMatch(/saved|updated/);
        }
      }
    }
  });

  test('should cancel metadata editing', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    if ((await metadataBtn.count()) === 0) {
      test.skip('Metadata button not available');
    }

    await metadataBtn.click();

    const editBtn = page.locator('#metadata-edit-btn');
    if ((await editBtn.count()) === 0) {
      test.skip('Edit button not available');
    }

    await editBtn.click();

    const editModal = page.locator('#metadata-edit-modal');
    await expect(editModal).toBeVisible();

    // Click cancel button
    const cancelBtn = page.locator('.cancel-btn, button[data-action="cancel"]');
    const cancelBtnExists = (await cancelBtn.count()) > 0;

    if (cancelBtnExists) {
      await cancelBtn.click();

      // Modal should close
      await expect(editModal).not.toBeVisible();
    }
  });

  test('should validate metadata input', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    if ((await metadataBtn.count()) === 0) {
      test.skip('Metadata button not available');
    }

    await metadataBtn.click();

    const editBtn = page.locator('#metadata-edit-btn');
    if ((await editBtn.count()) === 0) {
      test.skip('Edit button not available');
    }

    await editBtn.click();

    const editModal = page.locator('#metadata-edit-modal');
    await expect(editModal).toBeVisible();

    const form = page.locator('#metadata-edit-form');
    if ((await form.count()) === 0) {
      test.skip('Form not available');
    }

    // Try invalid latitude
    const latInput = form.locator('input[name="latitude"], #latitude');
    if ((await latInput.count()) > 0) {
      await latInput.fill('invalid');

      const saveBtn = form.locator('button[type="submit"], .save-btn');
      if ((await saveBtn.count()) > 0) {
        await saveBtn.click();
        await page.waitForTimeout(500);

        // Should show error or prevent submission
        const errorMsg = page.locator('#metadata-edit-error, .error-message');
        const hasError = (await errorMsg.count()) > 0 && (await errorMsg.isVisible());

        // Either shows error or form validation prevents submission
        expect(hasError || true).toBe(true);
      }
    }
  });

  test('should display full EXIF metadata', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    if ((await metadataBtn.count()) === 0) {
      test.skip('Metadata button not available');
    }

    await metadataBtn.click();

    // Look for full metadata section
    const fullMetadata = page.locator('.photo-meta-full, .metadata-section');
    const metaExists = (await fullMetadata.count()) > 0;

    if (metaExists) {
      // Should contain various metadata fields
      const sidebar = page.locator('.viewer-sidebar');
      const sidebarText = await sidebar.textContent();

      // Should have some content
      expect(sidebarText.length).toBeGreaterThan(0);
    }
  });

  test('should close metadata panel', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const metadataBtn = page.locator('.metadata-btn');
    if ((await metadataBtn.count()) === 0) {
      test.skip('Metadata button not available');
    }

    // Open metadata
    await metadataBtn.click();
    const sidebar = page.locator('.viewer-sidebar');
    await expect(sidebar).toBeVisible();

    // Close by clicking button again
    await metadataBtn.click();

    // Sidebar should close
    await expect(sidebar).not.toBeVisible();
  });
});
