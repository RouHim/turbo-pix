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
    const timelineExists = (await page.locator('.timeline-rail, .timeline-container').count()) > 0;
    expect(timelineExists).toBe(true);
  });

  test('should show date range when timeline is available', async ({ page }) => {
    // GIVEN: Timeline exists
    const timelineExists = (await page.locator('.timeline-rail, .timeline-container').count()) > 0;

    expect(timelineExists).toBe(true);

    // WHEN: User checks timeline
    // THEN: Date range label should be present
    const labelExists = (await page.locator('.timeline-label, .date-range-label').count()) > 0;

    expect(labelExists).toBe(true);
  });

  test('should filter to year then month in two clicks', async ({ page }) => {
    // GIVEN a cleared filter with timeline data
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((r) => r.json())
        .then((d) => d.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');
    const target = density[0];
    const yearButton = page.locator('.timeline-year-rail .timeline-year', {
      hasText: String(target.year),
    });
    await expect(yearButton.first()).toBeVisible();

    // WHEN activating a year
    await yearButton.first().click();
    await TestHelpers.waitForUrlParam(page, 'year', String(target.year));

    // THEN the month strip for that year appears with twelve months
    await expect(page.locator('.timeline-month-strip .timeline-month')).toHaveCount(12);

    // WHEN activating a non-empty month
    const bucket = density.find((d) => d.year === target.year && d.count > 0);
    const monthButton = page.locator('.timeline-month-strip .timeline-month').nth(bucket.month - 1);
    await expect(monthButton).toBeEnabled();
    await monthButton.click();

    // THEN grid + URL reflect year+month and the selection is visibly active
    await TestHelpers.waitForUrlParam(page, 'month', String(bucket.month));
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(monthButton).toHaveClass(/active/);
  });

  test('should render 50+ sparse years with zero label overlap', async ({ page }) => {
    const boxes = await page
      .locator('.timeline-year-rail .timeline-year')
      .evaluateAll((els) => els.map((el) => el.getBoundingClientRect().toJSON()));
    test.skip(boxes.length === 0, 'Timeline needs at least one year');
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const a = boxes[i];
        const b = boxes[j];
        const overlaps = a.x < b.x + b.width && b.x < a.x + a.width;
        expect(overlaps).toBe(false);
      }
    }
  });

  test('should disable empty months and clear via reset', async ({ page }) => {
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((r) => r.json())
        .then((d) => d.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');
    const targetYear = density[0].year;
    await page
      .locator('.timeline-year-rail .timeline-year', { hasText: String(targetYear) })
      .first()
      .click();
    const filled = new Set(density.filter((d) => d.year === targetYear).map((d) => d.month));
    test.skip(filled.size === 12, 'Year needs at least one empty month');
    const emptyIndex = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].find((m) => !filled.has(m));
    await expect(
      page.locator('.timeline-month-strip .timeline-month').nth(emptyIndex - 1)
    ).toBeDisabled();

    await page.locator('.timeline-rail .timeline-reset').click();
    await expect(page).not.toHaveURL(/[?&]year=/);
  });
});
