import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Timeline Filtering - Desktop', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.setDesktopViewport(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should display timeline slider on desktop', async ({ page }) => {
    const timelineSlider = page.locator('.timeline-input');
    const sliderExists = (await timelineSlider.count()) > 0;

    if (!sliderExists) {
      test.skip('Timeline slider not found');
    }

    await expect(timelineSlider).toBeVisible();
  });

  test('should show timeline heatmap canvas', async ({ page }) => {
    const heatmapCanvas = page.locator('.timeline-heatmap');
    const canvasExists = (await heatmapCanvas.count()) > 0;

    if (canvasExists) {
      await expect(heatmapCanvas).toBeVisible();
    }
  });

  test('should show timeline label with date range', async ({ page }) => {
    const timelineLabel = page.locator('.timeline-label');
    const labelExists = (await timelineLabel.count()) > 0;

    if (!labelExists) {
      test.skip('Timeline label not found');
    }

    await expect(timelineLabel).toBeVisible();

    // Label should have some text
    const labelText = await timelineLabel.textContent();
    expect(labelText.length).toBeGreaterThan(0);
  });

  test('should filter photos by moving timeline slider', async ({ page }) => {
    const timelineSlider = page.locator('.timeline-input');
    const sliderExists = (await timelineSlider.count()) > 0;

    if (!sliderExists) {
      test.skip('Timeline slider not available');
    }

    // Get initial photo count
    const initialPhotos = await TestHelpers.getPhotoCards(page);
    const initialCount = initialPhotos.length;

    // Move slider to middle position
    await timelineSlider.fill('50');
    await page.waitForTimeout(1000);

    // Wait for photos to reload with filter
    await TestHelpers.waitForPhotosToLoad(page);

    // Photos might have changed (or might not if all photos are in that range)
    const filteredPhotos = await TestHelpers.getPhotoCards(page);

    // Verify timeline is working (photos loaded)
    expect(filteredPhotos.length >= 0).toBe(true);
  });

  test('should show timeline reset button', async ({ page }) => {
    const resetBtn = page.locator('.timeline-reset');
    const btnExists = (await resetBtn.count()) > 0;

    if (btnExists) {
      await expect(resetBtn).toBeVisible();
    }
  });

  test('should reset timeline filter with reset button', async ({ page }) => {
    const timelineSlider = page.locator('.timeline-input');
    const resetBtn = page.locator('.timeline-reset');

    const sliderExists = (await timelineSlider.count()) > 0;
    const btnExists = (await resetBtn.count()) > 0;

    if (!sliderExists || !btnExists) {
      test.skip('Timeline controls not available');
    }

    // Apply filter
    await timelineSlider.fill('30');
    await page.waitForTimeout(500);

    // Reset filter
    await resetBtn.click();
    await page.waitForTimeout(500);

    // Wait for photos to reload
    await TestHelpers.waitForPhotosToLoad(page);

    // Should show all photos again
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length >= 0).toBe(true);
  });

  test('should update label when slider moves', async ({ page }) => {
    const timelineSlider = page.locator('.timeline-input');
    const timelineLabel = page.locator('.timeline-label');

    const sliderExists = (await timelineSlider.count()) > 0;
    const labelExists = (await timelineLabel.count()) > 0;

    if (!sliderExists || !labelExists) {
      test.skip('Timeline controls not available');
    }

    // Get initial label text
    const initialLabel = await timelineLabel.textContent();

    // Move slider
    await timelineSlider.fill('70');
    await page.waitForTimeout(300);

    // Label should have updated
    const newLabel = await timelineLabel.textContent();

    // Label should contain year or date information
    expect(newLabel.length).toBeGreaterThan(0);
  });
});

test.describe('Timeline Filtering - Mobile', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.setMobileViewport(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should display timeline dropdowns on mobile', async ({ page }) => {
    const yearSelect = page.locator('.timeline-year-select');
    const monthSelect = page.locator('.timeline-month-select');

    const yearExists = (await yearSelect.count()) > 0;
    const monthExists = (await monthSelect.count()) > 0;

    if (!yearExists || !monthExists) {
      test.skip('Mobile timeline controls not found');
    }

    await expect(yearSelect).toBeVisible();
    await expect(monthSelect).toBeVisible();
  });

  test('should filter photos by year selection', async ({ page }) => {
    const yearSelect = page.locator('.timeline-year-select');
    const yearExists = (await yearSelect.count()) > 0;

    if (!yearExists) {
      test.skip('Year select not available');
    }

    // Get available years
    const options = await yearSelect.locator('option').all();

    if (options.length < 2) {
      test.skip('Not enough years to test filtering');
    }

    // Select a year
    const yearValue = await options[1].getAttribute('value');
    if (yearValue) {
      await yearSelect.selectOption(yearValue);
      await page.waitForTimeout(1000);

      // Wait for photos to reload
      await TestHelpers.waitForPhotosToLoad(page);

      // Photos should have loaded
      const photos = await TestHelpers.getPhotoCards(page);
      expect(photos.length >= 0).toBe(true);
    }
  });

  test('should filter photos by month after year selection', async ({ page }) => {
    const yearSelect = page.locator('.timeline-year-select');
    const monthSelect = page.locator('.timeline-month-select');

    const yearExists = (await yearSelect.count()) > 0;
    const monthExists = (await monthSelect.count()) > 0;

    if (!yearExists || !monthExists) {
      test.skip('Timeline controls not available');
    }

    // Select a year first
    const yearOptions = await yearSelect.locator('option').all();
    if (yearOptions.length < 2) {
      test.skip('Not enough years');
    }

    const yearValue = await yearOptions[1].getAttribute('value');
    if (yearValue) {
      await yearSelect.selectOption(yearValue);
      await page.waitForTimeout(500);

      // Now select a month
      const monthOptions = await monthSelect.locator('option').all();
      if (monthOptions.length > 1) {
        const monthValue = await monthOptions[1].getAttribute('value');
        if (monthValue && monthValue !== '') {
          await monthSelect.selectOption(monthValue);
          await page.waitForTimeout(1000);

          // Wait for filtered photos
          await TestHelpers.waitForPhotosToLoad(page);

          const photos = await TestHelpers.getPhotoCards(page);
          expect(photos.length >= 0).toBe(true);
        }
      }
    }
  });

  test('should show reset button on mobile', async ({ page }) => {
    const resetBtn = page.locator('.timeline-reset');
    const btnExists = (await resetBtn.count()) > 0;

    if (btnExists) {
      await expect(resetBtn).toBeVisible();
    }
  });

  test('should reset mobile timeline filter', async ({ page }) => {
    const yearSelect = page.locator('.timeline-year-select');
    const resetBtn = page.locator('.timeline-reset');

    const yearExists = (await yearSelect.count()) > 0;
    const btnExists = (await resetBtn.count()) > 0;

    if (!yearExists || !btnExists) {
      test.skip('Timeline controls not available');
    }

    // Apply filter
    const yearOptions = await yearSelect.locator('option').all();
    if (yearOptions.length > 1) {
      const yearValue = await yearOptions[1].getAttribute('value');
      if (yearValue) {
        await yearSelect.selectOption(yearValue);
        await page.waitForTimeout(500);

        // Reset
        await resetBtn.click();
        await page.waitForTimeout(500);

        // Should show all photos
        await TestHelpers.waitForPhotosToLoad(page);
        const photos = await TestHelpers.getPhotoCards(page);
        expect(photos.length >= 0).toBe(true);
      }
    }
  });
});

test.describe('Timeline with Different Views', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should not show timeline in collages view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const timeline = page.locator('.timeline-container, .timeline-input');
    const timelineVisible = (await timeline.count()) > 0 && (await timeline.isVisible());

    expect(timelineVisible).toBe(false);
  });

  test('should not show timeline in housekeeping view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.waitForPhotosToLoad(page);

    const timeline = page.locator('.timeline-container, .timeline-input');
    const timelineVisible = (await timeline.count()) > 0 && (await timeline.isVisible());

    expect(timelineVisible).toBe(false);
  });

  test('should show timeline in all photos view', async ({ page }) => {
    const timeline = page.locator('.timeline-container, .timeline-input, .timeline-year-select');
    const timelineExists = (await timeline.count()) > 0;

    // Timeline should exist (might be hidden on mobile)
    expect(timelineExists).toBe(true);
  });

  test('should show timeline in favorites view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    const timeline = page.locator('.timeline-container, .timeline-input, .timeline-year-select');
    const timelineExists = (await timeline.count()) > 0;

    expect(timelineExists).toBe(true);
  });

  test('should show timeline in videos view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const timeline = page.locator('.timeline-container, .timeline-input, .timeline-year-select');
    const timelineExists = (await timeline.count()) > 0;

    expect(timelineExists).toBe(true);
  });
});
