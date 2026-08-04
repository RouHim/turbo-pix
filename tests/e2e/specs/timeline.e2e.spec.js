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

  test('should announce readable value via aria-valuetext', async ({ page }) => {
    // GIVEN: Timeline slider is rendered
    const slider = page.locator('.timeline-input');

    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');

    await expect(slider).toHaveCount(1);

    // THEN: The initial (rightmost, unfiltered) position announces "All Dates"
    await expect(slider).toHaveAttribute('aria-valuetext', 'All Dates');

    // WHEN: User scrubs to the oldest month bucket
    await slider.evaluate((el) => {
      el.value = '0';
      el.dispatchEvent(new Event('input', { bubbles: true }));
    });

    // THEN: aria-valuetext announces the readable "<Month> <YYYY>" value
    const expected = `${new Date(Date.UTC(density[0].year, density[0].month - 1, 1)).toLocaleString('en-US', { month: 'long' })} ${density[0].year}`;
    await expect(slider).toHaveAttribute('aria-valuetext', expected);
  });

  test('should scrub with keyboard', async ({ page }) => {
    // GIVEN: Timeline slider is rendered with at least two month buckets
    const slider = page.locator('.timeline-input');

    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length < 2, 'Timeline needs at least two month buckets to scrub');

    await expect(slider).toHaveCount(1);

    // WHEN: User focuses the slider and presses Home
    await slider.focus();
    await page.keyboard.press('Home');

    // THEN: The value jumps to the oldest bucket and the URL carries its filter
    await TestHelpers.waitForUrlParam(page, 'year', String(density[0].year));
    expect(await slider.inputValue()).toBe('0');

    // AND: ArrowRight advances one bucket
    await page.keyboard.press('ArrowRight');
    expect(await slider.inputValue()).toBe('1');
    await TestHelpers.waitForUrlParam(page, 'year', String(density[1].year));
    await TestHelpers.waitForUrlParam(page, 'month', String(density[1].month));

    // AND: End clears the filter back to "All Dates"
    await page.keyboard.press('End');
    expect(await slider.inputValue()).toBe(String(density.length - 1));
    await TestHelpers.waitForUrlParam(page, 'year', null);
    await TestHelpers.waitForUrlParam(page, 'month', null);
  });

  test('should show year tick labels', async ({ page }) => {
    // GIVEN: Timeline has data
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');

    // THEN: Year ticks are rendered under the track, oldest year first
    const ticks = page.locator('.timeline-year-tick');
    expect(await ticks.count()).toBeGreaterThanOrEqual(1);
    await expect(ticks.first()).toHaveText(String(density[0].year));
  });

  test('should keep tooltip inside viewport', async ({ page }) => {
    // GIVEN: Timeline has data
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');

    // WHEN: User hovers the first (oldest) bar
    const ribbon = page.locator('.timeline-ribbon');
    await expect(ribbon).toBeVisible();
    const box = await ribbon.boundingBox();
    await page.mouse.move(box.x + box.width / density.length / 2, box.y + box.height / 2);

    // THEN: The tooltip is visible and stays inside the viewport
    await expect(page.locator('.timeline-tooltip')).toBeVisible();
    const tooltip = await page.locator('.timeline-tooltip').boundingBox();
    const viewport = page.viewportSize();
    expect(tooltip.x).toBeGreaterThanOrEqual(0);
    expect(tooltip.y).toBeGreaterThanOrEqual(0);
    expect(tooltip.x + tooltip.width).toBeLessThanOrEqual(viewport.width);
    expect(tooltip.y + tooltip.height).toBeLessThanOrEqual(viewport.height);
  });

  test('should suppress animations with reduced motion', async ({ page }) => {
    // GIVEN: User prefers reduced motion
    await page.emulateMedia({ reducedMotion: 'reduce' });

    // WHEN: The page loads
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);

    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');

    // THEN: Bar growth animation is suppressed
    const name = await page
      .locator('.timeline-bar')
      .first()
      .evaluate((el) => getComputedStyle(el).animationName);
    expect(name).toBe('none');
  });

  test('should animate bars on load with motion allowed', async ({ page }) => {
    // GIVEN: Timeline has data and motion is allowed (default)
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');

    // THEN: Bars use the growth animation (Svelte scopes keyframes with the
    // component hash, so assert the suffix)
    const name = await page
      .locator('.timeline-bar')
      .first()
      .evaluate((el) => getComputedStyle(el).animationName);
    expect(name.endsWith('timeline-bar-grow')).toBe(true);
  });

  test('should fetch timeline data exactly once per mount', async ({ page }) => {
    // GIVEN: A request counter is registered before navigation
    let timelineRequests = 0;
    let timelineDensity = null;
    page.on('request', (request) => {
      if (request.url().includes('/api/photos/timeline')) timelineRequests += 1;
    });
    page.on('response', (response) => {
      if (response.url().includes('/api/photos/timeline') && response.ok()) {
        response
          .json()
          .then((data) => {
            timelineDensity = data?.density || [];
          })
          .catch(() => {});
      }
    });

    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await expect.poll(() => timelineDensity).not.toBeNull();
    const density = timelineDensity;
    test.skip(density.length < 2, 'Timeline needs at least two month buckets to filter');

    // WHEN: User scrubs the slider — a filter change must not remount/refetch
    const slider = page.locator('.timeline-input');
    await expect(slider).toHaveCount(1);
    await slider.evaluate((el) => {
      el.value = '0';
      el.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await TestHelpers.waitForUrlParam(page, 'year', String(density[0].year));
    await page.waitForTimeout(500);

    // THEN: Exactly one timeline fetch happened (the initial mount)
    expect(timelineRequests).toBe(1);
  });
});
