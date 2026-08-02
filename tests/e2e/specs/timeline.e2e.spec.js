import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Timeline', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
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
    // GIVEN: Timeline slider is rendered with at least two month buckets
    const slider = page.locator('.timeline-input');
    await expect(slider).toHaveCount(1);

    // Learn the month buckets from the same API the slider renders
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length < 2, 'Timeline needs at least two month buckets to filter');

    await expect(slider).toHaveAttribute('max', String(density.length - 1));

    const initialIds = await page
      .locator('[data-photo-id]')
      .evaluateAll((elements) => elements.map((el) => el.getAttribute('data-photo-id')));
    expect(initialIds.length).toBeGreaterThan(0);

    // WHEN: User drags the slider to the oldest month bucket (index 0)
    const target = density[0];
    const filteredResponse = page.waitForResponse(
      (response) =>
        response.url().includes('/api/photos') &&
        response.url().includes(`year=${target.year}`) &&
        response.url().includes(`month=${target.month}`)
    );
    await slider.evaluate((el) => {
      el.value = '0';
      el.dispatchEvent(new Event('input', { bubbles: true }));
    });

    // THEN: The URL reflects the selection and the grid re-queries with the filter
    await TestHelpers.waitForUrlParam(page, 'year', String(target.year));
    await TestHelpers.waitForUrlParam(page, 'month', String(target.month));
    await filteredResponse;
    await TestHelpers.waitForPhotosToLoad(page);

    // AND: The grid shows only photos from the selected bucket
    const filteredIds = await page
      .locator('[data-photo-id]')
      .evaluateAll((elements) => elements.map((el) => el.getAttribute('data-photo-id')));
    expect(filteredIds.length).toBeGreaterThan(0);
    expect(filteredIds.length).toBeLessThanOrEqual(initialIds.length);
    for (const id of filteredIds) {
      expect(initialIds).toContain(id);
    }
  });
});
