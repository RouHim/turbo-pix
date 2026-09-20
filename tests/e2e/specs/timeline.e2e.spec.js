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

  test('should keep the zoom and the selection when the window is resized across the breakpoint', async ({
    page,
  }) => {
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toBeVisible();

    // GIVEN: a zoomed-in view (fit-all would be decade-granular here)
    await page.locator('.timeline-zoom-in').click();
    const zoomedUnit = await page.locator('.timeline-column').first().getAttribute('data-unit');

    // WHEN: the viewport narrows below the desktop breakpoint and back
    await TestHelpers.setMobileViewport(page);
    // Exactly one experience is on screen at any width.
    await expect(page.locator('#timeline-year-select')).toHaveCount(1);
    await expect(page.locator('.timeline-selector')).toBeHidden();
    await TestHelpers.setDesktopViewport(page);
    await expect(page.locator('.timeline-selector')).toBeVisible();
    await expect(page.locator('#timeline-year-select')).toBeHidden();

    // THEN: the selector returns with the zoom intact (a collapsed view snaps
    // back to fit-all granularity) and the selection inside its viewport
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', zoomedUnit);
    await expect(page.locator('.timeline-selection')).toBeVisible();
    const selection = await page.locator('.timeline-selection').boundingBox();
    const lane = await page.locator('.timeline-lane').boundingBox();
    expect(selection.x).toBeGreaterThanOrEqual(lane.x - 1);
    expect(selection.x + selection.width).toBeLessThanOrEqual(lane.x + lane.width + 1);
  });

  test('should pin panning at the library start and end', async ({ page }) => {
    const timeline = await page.evaluate(() => fetch('/api/photos/timeline').then((r) => r.json()));
    test.skip(!timeline?.min_date || !timeline?.max_date, 'Timeline needs a dated library');

    const monthIndex = (iso) => {
      const date = new Date(iso);
      return date.getUTCFullYear() * 12 + date.getUTCMonth();
    };
    const libraryStart = monthIndex(timeline.min_date);
    const libraryEnd = monthIndex(timeline.max_date);

    // GIVEN: a zoomed-in view, so panning has room to move before it clamps
    await page.locator('.timeline-zoom-in').click();
    const unit = Number(await page.locator('.timeline-column').first().getAttribute('data-unit'));

    const dragRuler = async (fromRatio, toRatio) => {
      const ruler = await page.locator('.timeline-ruler').boundingBox();
      await page.mouse.move(ruler.x + ruler.width * fromRatio, ruler.y + ruler.height / 2);
      await page.mouse.down();
      await page.mouse.move(ruler.x + ruler.width * toRatio, ruler.y + ruler.height / 2, {
        steps: 10,
      });
      await page.mouse.up();
    };

    // WHEN: dragging the ruler past the library start (content moves right),
    // twice — the second drag must be a no-op at the clamp
    await dragRuler(0.2, 0.9);
    await dragRuler(0.2, 0.9);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute(
      'data-period-start',
      String(Math.floor(libraryStart / unit) * unit)
    );

    // AND: past the library end (content moves left)
    await dragRuler(0.9, 0.2);
    await dragRuler(0.9, 0.2);
    await expect(page.locator('.timeline-column').last()).toHaveAttribute(
      'data-period-start',
      String(Math.floor(libraryEnd / unit) * unit)
    );

    // AND: the pan left no gesture flag behind, so the first keyboard
    // activation after a pan is not swallowed (2004 = legacy_04's year, which
    // the panned-to view is showing at year granularity)
    const populated = page.locator('.timeline-column[data-period-start="24048"]');
    await populated.focus();
    await page.keyboard.press('Enter');
    await expect(page).toHaveURL(/year=2004/);
  });

  test('should clear the filter and fit the view from the reset control', async ({ page }) => {
    // GIVEN: a filtered, zoomed selector
    const fitAllUnit = await page.locator('.timeline-column').first().getAttribute('data-unit');
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator('.timeline-zoom-in').click();
    await expect(page.locator('.timeline-column').first()).not.toHaveAttribute(
      'data-unit',
      fitAllUnit
    );
    await expect(page.locator('.timeline-selection')).toBeVisible();

    // WHEN: the user clears the timeline filter
    await page.locator('.timeline-header .timeline-reset').click();

    // THEN: the URL is clean, the selection is gone and the view is fit-all
    await expect(page).not.toHaveURL(/year=/);
    await expect(page.locator('.timeline-selection')).toHaveCount(0);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', fitAllUnit);
  });

  test('should select any month in three interactions from the full span', async ({ page }) => {
    // GIVEN: the decade-spanning fixture, starting from a cleared filter
    await expect(page.locator('.timeline-column').first()).toBeVisible();
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');

    // The calendar mapping is asserted through the labels: the full span shows
    // decade labels, and March 1962 is 1962 * 12 + 2 = 23546.
    await expect(page.locator('.timeline-ruler-label', { hasText: '1960s' })).toHaveCount(1);

    // WHEN: drilling into the 1960s by activating the decade column
    await page.locator('.timeline-column[data-period-start="23520"]').click();

    // THEN: years appear and nothing is filtered yet
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');
    expect(TestHelpers.getUrlState(page).year).toBeNull();

    // WHEN: activating 1962 (23544 === 1962 * 12)
    await page.locator('.timeline-column[data-period-start="23544"]').click();
    await TestHelpers.waitForUrlParam(page, 'year', '1962');

    // THEN: months appear and the year filter is active
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');

    // WHEN: activating March (23546 === 1962 * 12 + 2)
    await page.locator('.timeline-column[data-period-start="23546"]').click();
    await TestHelpers.waitForUrlParam(page, 'month', '3');

    // THEN: the grid shows exactly the one seeded March 1962 photo
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.photo-card')).toHaveCount(1);
  });

  test('should set an inclusive month-granular range with one drag', async ({ page }) => {
    // GIVEN: the 2012 year view (legacy_05 seeded in March 2012)
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');

    // WHEN: dragging from February to March (24145 → 24146; January would be
    // canonicalised into the whole-year form and blur the assertion)
    const lane = await page.locator('.timeline-lane').boundingBox();
    const february = await page
      .locator('.timeline-column[data-period-start="24145"]')
      .boundingBox();
    const march = await page.locator('.timeline-column[data-period-start="24146"]').boundingBox();
    await page.mouse.move(february.x + february.width / 2, lane.y + lane.height / 2);
    await page.mouse.down();
    await page.mouse.move(march.x + march.width / 2, lane.y + lane.height / 2, { steps: 8 });
    await page.mouse.up();

    // THEN: both bounds are written, inclusive, and no drill-in click fired
    const state = TestHelpers.getUrlState(page);
    expect(state.year).toBe(2012);
    expect(state.month).toBe(2);
    expect(state.toYear).toBe(2012);
    expect(state.toMonth).toBe(3);
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.photo-card')).toHaveCount(1);
  });

  test('should adjust a range bound by dragging its handle', async ({ page }) => {
    await page.goto('/?year=2012&month=2&to_year=2012&to_month=3');
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: dragging the start handle one month further right (Feb → Mar would
    // cross the end, so clampBound holds it at March)
    const lane = await page.locator('.timeline-lane').boundingBox();
    const startHandle = await page.locator('.timeline-handle.start').boundingBox();
    const march = await page.locator('.timeline-column[data-period-start="24146"]').boundingBox();
    await page.mouse.move(startHandle.x + startHandle.width / 2, lane.y + lane.height / 2);
    await page.mouse.down();
    await page.mouse.move(march.x + march.width / 2, lane.y + lane.height / 2, { steps: 6 });
    await page.mouse.up();

    // THEN: the start moved and the end stayed, and the range never inverted:
    // the start clamped at the end bound, and March–March is canonically the
    // single period `?year=2012&month=3` (a range of one period *is* that
    // period), so the end appears as the absent `to_year`/`to_month` the
    // canonical form drops — never as a bound before the start.
    const state = TestHelpers.getUrlState(page);
    expect(state.month).toBe(3);
    expect(state.toYear).toBeNull();
    expect(state.toMonth).toBeNull();

    // AND: the same drag on a range whose start is not January leaves the end
    // bound alone. February 2012 – April 2013 is framed wide enough (15 months
    // with the range's 20% padding) to show January 2012 inside the lane.
    await page.goto('/?year=2012&month=2&to_year=2013&to_month=4');
    await TestHelpers.waitForPhotosToLoad(page);
    const widenedHandle = await page.locator('.timeline-handle.start').boundingBox();
    const january = await page.locator('.timeline-column[data-period-start="24144"]').boundingBox();
    await page.mouse.move(widenedHandle.x + widenedHandle.width / 2, lane.y + lane.height / 2);
    await page.mouse.down();
    await page.mouse.move(january.x + january.width / 2, lane.y + lane.height / 2, { steps: 6 });
    await page.mouse.up();

    // THEN: only the start moved, so the range reads January 2012 – April 2013
    // (January is the implicit start of the whole-year form, hence the absent
    // month and the end still carrying its own bound)
    const widened = TestHelpers.getUrlState(page);
    expect(widened.year).toBe(2012);
    expect(widened.month).toBeNull();
    expect(widened.toYear).toBe(2013);
    expect(widened.toMonth).toBe(4);
  });

  test('should translate a range by dragging its body and clamp at the data ends', async ({
    page,
  }) => {
    // GIVEN: February 2012 – May 2014, a range inside the library
    await page.goto('/?year=2012&month=2&to_year=2014&to_month=5');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');

    const lane = await page.locator('.timeline-lane').boundingBox();
    // Rendered month width, averaged over the month columns on screen
    const monthPx = () =>
      page.evaluate(() => {
        const columns = [...document.querySelectorAll('.timeline-column[data-unit="1"]')].map(
          (element) => ({
            start: Number(element.dataset.periodStart),
            x: element.getBoundingClientRect().left,
          })
        );
        columns.sort((a, b) => a.start - b.start);
        const first = columns[0];
        const last = columns[columns.length - 1];
        return (last.x - first.x) / (last.start - first.start);
      });
    const dragBody = async (months) => {
      const y = lane.y + lane.height / 2;
      await page.mouse.move(lane.x + lane.width / 2, y);
      await page.mouse.down();
      await page.mouse.move(lane.x + lane.width / 2 + months * (await monthPx()), y, { steps: 10 });
      await page.mouse.up();
    };
    const bounds = async () => {
      const dragged = TestHelpers.getUrlState(page);
      return [dragged.year, dragged.month, dragged.toYear, dragged.toMonth];
    };

    // WHEN: the body is dragged three months later, then three months back
    await dragBody(3);

    // THEN: the span is preserved and both bounds moved with the pointer
    expect(await bounds()).toEqual([2012, 5, 2014, 8]);
    await dragBody(-3);
    expect(await bounds()).toEqual([2012, 2, 2014, 5]);

    // AND: at the library start the drag clamps instead of shifting the span out
    // of the model (March 1962 is the oldest bucket)
    await page.goto('/?year=1962&month=3&to_year=1963&to_month=2');
    await TestHelpers.waitForPhotosToLoad(page);
    await dragBody(-3);
    expect(await bounds()).toEqual([1962, 3, 1963, 2]);

    // AND: at the library end the same holds, for a range ending on the newest
    // bucket the fixture seeded
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');
    const lastBucket = density[density.length - 1];
    const endIndex = lastBucket.year * 12 + lastBucket.month - 1;
    const firstIndex = endIndex - 11;
    const firstBucket = {
      year: Math.floor(firstIndex / 12),
      month: (firstIndex % 12) + 1,
    };
    await page.goto(
      `/?year=${firstBucket.year}&month=${firstBucket.month}` +
        `&to_year=${lastBucket.year}&to_month=${lastBucket.month}`
    );
    await TestHelpers.waitForPhotosToLoad(page);
    await dragBody(3);
    expect(await bounds()).toEqual([
      firstBucket.year,
      firstBucket.month,
      lastBucket.year,
      lastBucket.month,
    ]);
  });

  test('should cancel a gesture back to the pre-drag selection on Escape', async ({ page }) => {
    // GIVEN: the 2012 year view, where the populated March column (the seeded
    // legacy_05) is the period the pointer ends on
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);

    const lane = await page.locator('.timeline-lane').boundingBox();
    const february = await page
      .locator('.timeline-column[data-period-start="24145"]')
      .boundingBox();
    const march = await page.locator('.timeline-column[data-period-start="24146"]').boundingBox();
    const midY = lane.y + lane.height / 2;
    const bounds = () => {
      const state = TestHelpers.getUrlState(page);
      return [state.year, state.month, state.toYear, state.toMonth];
    };

    // WHEN: a brush is started, Escape is pressed mid-gesture, and only then the
    // pointer is released — over the March column, away from where it started
    await page.mouse.move(february.x + february.width / 2, midY);
    await page.mouse.down();
    await page.mouse.move(march.x + march.width / 2, midY, { steps: 8 });
    await page.keyboard.press('Escape');
    await page.mouse.up();

    // THEN: the release activated nothing — no filter overwrites the selection
    // Escape restored — and the aborted gesture committed no bound
    expect(await bounds()).toEqual([2012, null, null, null]);

    // AND: the same holds when the release lands back on the very column the
    // drag started on, where the release's click targets that column itself
    // (this is the case a released capture cannot protect: without suppressing
    // it, March 2012 gets activated and the URL ends up filtered to it)
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);
    const pressX = march.x + march.width / 2;
    await page.mouse.move(pressX, midY);
    await page.mouse.down();
    await page.mouse.move(pressX + 20, midY, { steps: 4 });
    await page.keyboard.press('Escape');
    await page.mouse.up();

    expect(await bounds()).toEqual([2012, null, null, null]);
  });

  test('should ignore activation of an empty period and clear back to the full span', async ({
    page,
  }) => {
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);

    // April 2012 (24147) has no photos: activating it must not filter. The
    // click is a raw input click, not `locator.click()`: the column carries
    // `aria-disabled="true"` (an empty period announces itself as unavailable
    // but is never given the DOM `disabled` attribute, so it stays focusable
    // and its activation still reaches the handler) and Playwright's
    // actionability gate treats that as not-enabled. A real click also proves
    // the column is reachable: an empty period still renders a pointer target.
    const april = await page.locator('.timeline-column[data-period-start="24147"]').boundingBox();
    await page.mouse.click(april.x + april.width / 2, april.y + april.height / 2);
    const state = TestHelpers.getUrlState(page);
    expect(state.month).toBeNull();
    expect(state.toMonth).toBeNull();

    // WHEN: clearing from a zoomed, filtered state
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=3');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.click('.timeline-reset');

    // THEN: unfiltered and back to the full span in one action
    await expect(page).not.toHaveURL(/year=/);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');
  });
});
