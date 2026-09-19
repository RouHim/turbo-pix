import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Timeline', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should render the whole span with density columns and legible labels', async ({ page }) => {
    // GIVEN: a library spanning six decades
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((r) => r.json())
        .then((d) => d.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');

    const lane = page.locator('.timeline-lane');
    await expect(lane).toBeVisible();

    // THEN: the selector needs no horizontal scrolling
    const scroll = await lane.evaluate((el) => ({
      scrollWidth: el.scrollWidth,
      clientWidth: el.clientWidth,
    }));
    expect(scroll.scrollWidth).toBe(scroll.clientWidth);

    // AND: every rendered label sits fully inside the ruler and never overlaps
    const labels = await page
      .locator('.timeline-ruler-label')
      .evaluateAll((els) => els.map((el) => el.getBoundingClientRect().toJSON()));
    const ruler = await page.locator('.timeline-ruler').boundingBox();
    expect(labels.length).toBeGreaterThan(0);
    for (let i = 0; i < labels.length; i += 1) {
      expect(labels[i].left).toBeGreaterThanOrEqual(ruler.x - 1);
      expect(labels[i].right).toBeLessThanOrEqual(ruler.x + ruler.width + 1);
      for (let j = i + 1; j < labels.length; j += 1) {
        const overlaps = labels[i].left < labels[j].right && labels[j].left < labels[i].right;
        expect(overlaps, `labels ${i} and ${j} overlap`).toBe(false);
      }
    }
  });

  test('should zoom with the wheel and the zoom controls, and fit back to the full span', async ({
    page,
  }) => {
    const span = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((r) => r.json())
        .then((d) => (d.density || []).length)
    );
    test.skip(span === 0, 'Timeline needs at least one month bucket');

    // How much of the library is on screen. Column *count* cannot be used
    // here: zooming in also refines the column unit (decade -> year -> month),
    // so the count grows even though the visible span shrinks.
    const visibleSpan = () =>
      page.evaluate(() => {
        const columns = [...document.querySelectorAll('.timeline-column')];
        if (columns.length === 0) return 0;
        const starts = columns.map((column) => Number(column.dataset.periodStart));
        return Math.max(...starts) - Math.min(...starts) + Number(columns[0].dataset.unit);
      });

    const fitAllSpan = await visibleSpan();
    expect(fitAllSpan).toBeGreaterThan(0);

    // WHEN: zooming in with the wheel over a column (not the empty background)
    const column = page.locator('.timeline-column').nth(1);
    const box = await column.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.wheel(0, -400);
    await expect.poll(visibleSpan).toBeLessThan(fitAllSpan);

    // WHEN: zooming back out with the control
    const zoomedInSpan = await visibleSpan();
    await page.click('.timeline-zoom-out');
    await expect.poll(visibleSpan).toBeGreaterThan(zoomedInSpan);
    await expect.poll(visibleSpan).toBeLessThan(fitAllSpan);

    // WHEN: fitting all
    await page.click('.timeline-fit-all');
    await expect.poll(visibleSpan).toBe(fitAllSpan);
  });

  test('should keep the selector off the page when the timeline fails to load', async ({
    page,
  }) => {
    // GIVEN: the timeline endpoint is down
    await page.route('**/api/photos/timeline', (route) =>
      route.fulfill({ status: 500, contentType: 'application/json', body: '{}' })
    );

    // WHEN: the user opens a deep link with an active range
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: no selector renders and the filter is left alone (a load failure
    // must not be mistaken for "the data is gone" and wipe the selection)
    await expect(page.locator('.timeline-selector')).toHaveCount(0);
    const state = TestHelpers.getUrlState(page);
    expect(state.year).toBe(2012);
    expect(state.toMonth).toBe(8);
  });

  test('should clamp a restored selection to the library instead of hiding it', async ({
    page,
  }) => {
    // GIVEN: a range that starts before the library exists
    await page.goto('/?year=1900&month=4&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: the filter narrows to the overlap and the selection is on screen
    const state = TestHelpers.getUrlState(page);
    expect(state.year).toBe(1962);
    const selection = await page.locator('.timeline-selection').boundingBox();
    const lane = await page.locator('.timeline-lane').boundingBox();
    expect(selection.x).toBeGreaterThanOrEqual(lane.x - 1);
    expect(selection.x + selection.width).toBeLessThanOrEqual(lane.x + lane.width + 1);

    // AND: a range with no overlap clears the filter entirely
    await page.goto('/?year=1900&month=4&to_year=1901&to_month=6');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page).not.toHaveURL(/year=/);
  });

  test('should keep the selection visible when the window is resized across the breakpoint', async ({
    page,
  }) => {
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: the viewport narrows below the desktop breakpoint and back
    await TestHelpers.setMobileViewport(page);
    // Exactly one experience is on screen at any width.
    await expect(page.locator('#timeline-year-select')).toHaveCount(1);
    await expect(page.locator('.timeline-selector')).toBeHidden();
    await TestHelpers.setDesktopViewport(page);
    await expect(page.locator('.timeline-selector')).toBeVisible();
    await expect(page.locator('#timeline-year-select')).toBeHidden();

    // THEN: the selector returns with the selection inside its viewport
    await expect(page.locator('.timeline-selection')).toBeVisible();
    const selection = await page.locator('.timeline-selection').boundingBox();
    const lane = await page.locator('.timeline-lane').boundingBox();
    expect(selection.x).toBeGreaterThanOrEqual(lane.x - 1);
    expect(selection.x + selection.width).toBeLessThanOrEqual(lane.x + lane.width + 1);
  });

  test('should keep the visible span inside the data when panning the ruler', async ({ page }) => {
    const span = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((r) => r.json())
        .then((d) => (d.density || []).length)
    );
    test.skip(span < 2, 'Panning needs at least two buckets');

    const firstColumn = page.locator('.timeline-column').first();
    const firstStartBefore = await firstColumn.getAttribute('data-period-start');

    const ruler = await page.locator('.timeline-ruler').boundingBox();
    await page.mouse.move(ruler.x + ruler.width * 0.6, ruler.y + ruler.height / 2);
    await page.mouse.down();
    await page.mouse.move(ruler.x + ruler.width * 0.2, ruler.y + ruler.height / 2, { steps: 10 });
    await page.mouse.up();

    // Panning is clamped to the data span, so the first column can only move
    // forward in time, never before the library start.
    const firstStartAfter = await page
      .locator('.timeline-column')
      .first()
      .getAttribute('data-period-start');
    expect(Number(firstStartAfter)).toBeGreaterThanOrEqual(Number(firstStartBefore));
  });
});
