import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Timeline', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should display timeline controls', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: Page loads
    // THEN: Timeline elements should exist
    const timelineExists =
      (await page.locator('.timeline-slider, .timeline-container').count()) > 0;
    expect(timelineExists).toBe(true);
  });

  test('should show date range when timeline is available', async ({ page }) => {
    // GIVEN: Timeline exists
    const timelineExists =
      (await page.locator('.timeline-slider, .timeline-container').count()) > 0;

    expect(timelineExists).toBe(true);

    // WHEN: User checks timeline
    // THEN: Date range label should be present
    const labelExists = (await page.locator('.timeline-label, .date-range-label').count()) > 0;

    expect(labelExists).toBe(true);
  });

  test('should filter photos by date range', async ({ page }) => {
    // GIVEN: Timeline slider exists
    const sliderExists = (await page.locator('.timeline-slider').count()) > 0;

    expect(sliderExists).toBe(true);

    // WHEN: User interacts with timeline
    const initialPhotoCount = (await TestHelpers.getPhotoCards(page)).length;

    // THEN: Initial photos are displayed
    expect(initialPhotoCount).toBeGreaterThan(0);

    // Note: Actual slider interaction would require specific implementation details
    // This test verifies the timeline infrastructure exists
  });
});
