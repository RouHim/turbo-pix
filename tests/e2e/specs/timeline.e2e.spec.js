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

  test('should keep a zoom control press instead of re-framing the selection back', async ({
    page,
  }) => {
    // GIVEN: an active range, which the FR-010 effect re-frames on every view
    // change unless the gesture owns the view change it makes
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toBeVisible();

    // How much of the library is on screen. Column *count* cannot be used here:
    // zooming in also refines the column unit (decade -> year -> month), so the
    // count grows even though the visible span shrinks.
    const visibleSpan = () =>
      page.evaluate(() => {
        const columns = [...document.querySelectorAll('.timeline-column')];
        if (columns.length === 0) return 0;
        const starts = columns.map((column) => Number(column.dataset.periodStart));
        return Math.max(...starts) - Math.min(...starts) + Number(columns[0].dataset.unit);
      });

    // WHEN: zooming in with the control, twice. The first press zoomed the
    // selection in (from 1.2 spans to exactly one span): a press the effect
    // takes back lands on `width / span`, which is a fixed point, so the second
    // press is what proves the view really moved and stayed moved.
    const fitSpan = await visibleSpan();
    await page.click('.timeline-zoom-in');
    const firstZoomSpan = await visibleSpan();
    expect(firstZoomSpan).toBeLessThan(fitSpan);
    await page.click('.timeline-zoom-in');
    await expect.poll(visibleSpan, { timeout: 2000 }).toBeLessThan(firstZoomSpan);
  });

  test('should keep a ruler pan instead of snapping back to the selection', async ({ page }) => {
    // GIVEN: an active range, and therefore a view the FR-010 effect will
    // re-frame as soon as a gesture stops owning it
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toBeVisible();
    const firstColumnStart = () =>
      page.locator('.timeline-column').first().getAttribute('data-period-start').then(Number);

    // WHEN: panning the ruler left, which moves the view later into the library
    // and takes the 2012 selection off screen. The drag guard covers the moves;
    // the release drops it, and that is what re-triggers the effect.
    const before = await firstColumnStart();
    const ruler = await page.locator('.timeline-ruler').boundingBox();
    await page.mouse.move(ruler.x + ruler.width * 0.9, ruler.y + ruler.height / 2);
    await page.mouse.down();
    await page.mouse.move(ruler.x + ruler.width * 0.1, ruler.y + ruler.height / 2, { steps: 10 });
    await page.mouse.up();

    // THEN: the pan stays — the view is ~0.8 viewport widths (≈5.8 months here)
    // later instead of snapping back to the selection's start (one month on)
    await expect.poll(firstColumnStart, { timeout: 2000 }).toBeGreaterThanOrEqual(before + 4);
  });

  test('should zoom with a two-finger pinch on the lane without touching the filter', async ({
    page,
  }) => {
    // Playwright's touch API cannot synthesise a two-point gesture, so the
    // pinch goes through CDP — the same recipe the Task 11 probe used, and the
    // only way FR-003's touch zoom is observable end to end.
    const visibleSpan = () =>
      page.evaluate(() => {
        const columns = [...document.querySelectorAll('.timeline-column')];
        if (columns.length === 0) return 0;
        const starts = columns.map((column) => Number(column.dataset.periodStart));
        return Math.max(...starts) - Math.min(...starts) + Number(columns[0].dataset.unit);
      });
    // The lane paints one render after the timeline fetch lands, so wait for a
    // measurable span before capturing it (a not-yet-painted lane reads 0).
    await expect.poll(visibleSpan).toBeGreaterThan(0);
    const fitAllSpan = await visibleSpan();

    const client = await page.context().newCDPSession(page);
    const touchPoint = (x, y) => ({ x, y, radiusX: 5, radiusY: 5, force: 1 });
    // Two fingers 80px apart, spread symmetrically to 272px (≈3.4×).
    const pinch = async () => {
      const lane = await page.locator('.timeline-lane').boundingBox();
      const cx = lane.x + lane.width / 2;
      const cy = lane.y + lane.height / 2;
      await client.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
      await client.send('Input.dispatchTouchEvent', {
        type: 'touchStart',
        touchPoints: [touchPoint(cx - 40, cy), touchPoint(cx + 40, cy)],
      });
      for (let i = 1; i <= 12; i += 1) {
        await client.send('Input.dispatchTouchEvent', {
          type: 'touchMove',
          touchPoints: [touchPoint(cx - 40 - i * 8, cy), touchPoint(cx + 40 + i * 8, cy)],
        });
        await page.waitForTimeout(25);
      }
      await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
      await client.send('Emulation.setTouchEmulationEnabled', { enabled: false });
    };
    const lane = await page.locator('.timeline-lane').boundingBox();
    const cx = lane.x + lane.width / 2;

    // The pinch's own invariant, and the thing a column activation cannot
    // fake: the period under the pinch midpoint stays under it. A release that
    // drilled into the decade column instead would jump the view to that
    // decade's left edge — 1960s, where the midpoint reads the late 1990s —
    // which is why span/URL/selection alone are not enough here. (`data-unit`
    // cannot serve: a real pinch at fit-all shrinks the span 720 → ~216 months,
    // and `chooseUnit` must refine the columns 120 → 12 for that, exactly as a
    // decade drill-in does.)
    const decadeAtMidpoint = () =>
      page
        .evaluate((midpoint) => {
          const under = [...document.querySelectorAll('.timeline-column')].find((column) => {
            const rect = column.getBoundingClientRect();
            return rect.left <= midpoint && midpoint <= rect.right;
          });
          return under ? Math.floor(Number(under.dataset.periodStart) / 120) : null;
        }, cx)
        .then((decade) => {
          expect(decade).not.toBeNull();
          return decade;
        });
    const decadeBefore = await decadeAtMidpoint();

    // WHEN: two fingers spread symmetrically over the lane
    await pinch();

    // THEN: the view zooms in about the fingers, and the release commits no
    // filter — the pinch is a view gesture, so neither the brush it interrupted
    // nor the lifted finger may reach a column.
    await expect.poll(visibleSpan).toBeLessThan(fitAllSpan);
    await expect(page).not.toHaveURL(/year=/);
    await expect(page.locator('.timeline-selection')).toHaveCount(0);
    expect(await decadeAtMidpoint()).toBe(decadeBefore);

    // AND: with an active range the pinch keeps zooming — the selection is not
    // a cap. A pinch drops the drag that carries the in-progress guard, so with
    // only that guard the FR-010 reframe would pull the view back to
    // `width / span` on every pinch move: here the range is February–September
    // 2012, so the visible span could never pass below its own 8 months.
    await page.goto('/?year=2012&month=2&to_year=2012&to_month=9');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect.poll(visibleSpan).toBeGreaterThan(0);
    await pinch();
    await expect.poll(visibleSpan, { timeout: 2000 }).toBeLessThan(6);
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 2012,
      month: 2,
      toYear: 2012,
      toMonth: 9,
    });
  });

  test('should announce a column at the granularity the ruler shows', async ({ page }) => {
    // The announced name is the only thing a screen reader has: a decade column
    // must not introduce itself by its first month. Asserted at all three units,
    // on the column label and on the status row that repeats it.
    const status = page.locator('.timeline-status');
    const columnAt = (periodStart) =>
      page.locator(`.timeline-column[data-period-start="${periodStart}"]`);

    // GIVEN: the full span, where the columns are decades
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');
    const decade = columnAt(23520); // 1960 * 12
    await expect(decade).toHaveAttribute('aria-label', /^1960s, \d+ photos$/);

    // AND: hovering announces the same name in the status row
    await decade.hover();
    await expect(status).toHaveText(/^1960s, \d+ photos$/);

    // WHEN: drilling into the 1960s — the columns become years
    await decade.click();
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');
    const year = columnAt(23544); // 1962 * 12
    await expect(year).toHaveAttribute('aria-label', /^1962, \d+ photos$/);
    await year.hover();
    await expect(status).toHaveText(/^1962, \d+ photos$/);

    // WHEN: filtering to 1962 — the columns become months
    await year.click();
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');
    const month = columnAt(23546); // 1962 * 12 + 2 === March 1962
    await expect(month).toHaveAttribute('aria-label', /^March 1962, \d+ photos$/);
    await month.hover();
    await expect(status).toHaveText(/^March 1962, \d+ photos$/);
  });

  test('should keep a bound outside the model span inside the announced slider range', async ({
    page,
  }) => {
    const sliderRange = (selector) =>
      page.locator(selector).evaluate((element) => ({
        min: Number(element.getAttribute('aria-valuemin')),
        max: Number(element.getAttribute('aria-valuemax')),
        now: Number(element.getAttribute('aria-valuenow')),
      }));
    const assertInsideRange = async (selector, label) => {
      const { min, max, now } = await sliderRange(selector);
      expect(now, `${label} value vs min`).toBeGreaterThanOrEqual(min);
      expect(now, `${label} value vs max`).toBeLessThanOrEqual(max);
    };

    // GIVEN: a bare-year deep link for the library's first year, which starts in
    // March — the period keeps its own January, so its start bound sits *below*
    // `model.minIndex`
    await page.goto('/?year=1962');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-handle.start')).toBeVisible();
    await assertInsideRange('.timeline-handle.start', 'start handle');
    await assertInsideRange('.timeline-handle.end', 'end handle');

    // AND: the newest year, whose December end sits *above* `model.maxIndex`
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');
    const newest = density[density.length - 1];
    await page.goto(`/?year=${newest.year}`);
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-handle.end')).toBeVisible();
    await assertInsideRange('.timeline-handle.start', 'start handle');
    await assertInsideRange('.timeline-handle.end', 'end handle');
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

    // THEN: one activation applies the single period `?year=1962&month=3` — a
    // month is the smallest single period (FR-007), so no end bound appears and
    // the label names that month instead of a twelve-month range
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 1962,
      month: 3,
      toYear: null,
      toMonth: null,
    });
    await expect(page).toHaveURL(/[?&]year=1962&month=3(&|$)/);
    await expect(page.locator('.timeline-header .timeline-label')).toHaveText('March 1962');

    // THEN: the grid shows exactly the one seeded March 1962 photo
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.photo-card')).toHaveCount(1);

    // AND: the mobile month dropdown writes the same single-period filter for
    // the same choice (year first, then month)
    await TestHelpers.setMobileViewport(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await page.selectOption('#timeline-year-select', '2012');
    await expect(page.locator('#timeline-month-select')).toBeEnabled();
    await page.selectOption('#timeline-month-select', '3');
    await TestHelpers.waitForUrlParam(page, 'month', '3');
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 2012,
      month: 3,
      toYear: null,
      toMonth: null,
    });
    await expect(page).toHaveURL(/[?&]year=2012&month=3(&|$)/);
  });

  test('should commit a year column as the whole year, as the mobile dropdown does', async ({
    page,
  }) => {
    // GIVEN: the decade view, drilling into the library's *first* year — which
    // starts in March, so `buildColumns` clips the 1962 column to March–December
    await page.locator('.timeline-column[data-period-start="23520"]').click();
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');
    await page.locator('.timeline-column[data-period-start="23544"]').click();
    await TestHelpers.waitForUrlParam(page, 'year', '1962');

    // THEN: one activation applied the single period `?year=1962` — the clipped
    // bounds are not a single period (FR-007), and they would also disagree with
    // the same year chosen in the mobile dropdown
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 1962,
      month: null,
      toYear: null,
      toMonth: null,
    });
    await expect(page).toHaveURL(/[?&]year=1962(&|$)/);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');

    // AND: the mobile dropdown writes the very same filter for that year
    await TestHelpers.setMobileViewport(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('#timeline-year-select')).toHaveCount(1);
    await expect(page.locator('#timeline-year-select option[value="1962"]')).toHaveCount(1);
    await page.selectOption('#timeline-year-select', '1962');
    await TestHelpers.waitForUrlParam(page, 'year', '1962');
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 1962,
      month: null,
      toYear: null,
      toMonth: null,
    });
    await expect(page).toHaveURL(/[?&]year=1962(&|$)/);
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
    // 13 months, deliberately never 12: a window that starts in January and ends
    // in December is canonicalised into a bare whole year, which is a period and
    // not a range — the body press would brush instead of translate. The newest
    // bucket is the cluster seed at `now - 7 days`, so a 12-month window reaches
    // that shape every late December.
    const firstIndex = endIndex - 12;
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
    // THEN: the range is clamped where it was, in the canonical form the writer
    // produces — a January start and a December end are written as absent months
    expect(await bounds()).toEqual([
      firstBucket.year,
      firstBucket.month === 1 ? null : firstBucket.month,
      lastBucket.year,
      lastBucket.month === 12 ? null : lastBucket.month,
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

  test('should select a year, a month and a range with the keyboard only', async ({ page }) => {
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');

    // Focus the roving column and read its announcement
    const column = page.locator('.timeline-column[tabindex="0"]');
    await column.focus();
    const name = await column.getAttribute('aria-label');
    expect(name).toMatch(/\d{4}/);
    expect(name).toMatch(/photos|No photos/);

    // Arrow keys move focus to the next period the library has photos in: a
    // zero-photo month announces itself but cannot be activated or selected,
    // so it is stepped over
    await page.keyboard.press('ArrowRight');
    const focused = page.locator('.timeline-column:focus');
    await expect(focused).toHaveCount(1);

    // Enter applies exactly what a pointer click would — the single period, so
    // no end bound is written (a month is not a twelve-month range)
    const start = await focused.getAttribute('data-period-start');
    await page.keyboard.press('Enter');
    await TestHelpers.waitForPhotosToLoad(page);
    const entered = TestHelpers.getUrlState(page);
    expect(entered.month).toBe((Number(start) % 12) + 1);
    expect(entered.toYear).toBeNull();
    expect(entered.toMonth).toBeNull();

    // AND: Shift+Enter on the column the arrow moved to extends the selection
    // into a range — the next populated period, not always its neighbour
    await page.keyboard.press('ArrowRight');
    const extendTo = await page.locator('.timeline-column:focus').getAttribute('data-period-start');
    await page.keyboard.press('Shift+Enter');
    await TestHelpers.waitForPhotosToLoad(page);
    const extended = TestHelpers.getUrlState(page);
    expect(extended.month).toBe((Number(start) % 12) + 1);
    expect(extended.toMonth).toBe((Number(extendTo) % 12) + 1);

    // AND: the commit re-rendered the grid without dropping the roving focus
    // out of the lane (the range reframes the view, which unmounts the column
    // the focus was on)
    await expect(page.locator('.timeline-column:focus')).toHaveCount(1);
    expect(
      await page.evaluate(() => document.activeElement?.closest('.timeline-column') !== null)
    ).toBe(true);
  });

  test('should step a year-granular column to the next populated period, not an off-grid month', async ({
    page,
  }) => {
    // GIVEN: the fixture library's empty 1990s — 1985 is its last populated year
    // before the gap and 2004 the first after it
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => data.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');
    const populated = new Set(density.map((bucket) => bucket.year));
    expect(populated.has(1985), 'legacy_03 seeds 1985').toBe(true);
    expect(populated.has(2004), 'legacy_04 seeds 2004').toBe(true);
    for (let year = 1986; year <= 2003; year += 1) {
      expect(populated.has(year), `${year} must stay empty`).toBe(false);
    }

    // AND: the 1980s decade view, where the columns are years
    await page.locator('.timeline-column[data-period-start="23760"]').click();
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');

    // WHEN: arrowing right from the 1985 column. A scan that advances one month
    // at a time finds the first populated 12-month window starting in November
    // 2003 — a period the year grid does not contain — and `focusColumn` then
    // finds no `data-period-start` to focus, so the arrow silently does nothing.
    const focusedStart = () =>
      page.evaluate(() => document.activeElement?.dataset?.periodStart ?? null);
    await page.locator('.timeline-column[data-period-start="23820"]').focus();
    await expect.poll(focusedStart).toBe('23820');
    await page.keyboard.press('ArrowRight');

    // THEN: the roving focus lands on the next populated *year column*, and the
    // reveal pan brings it into the lane
    await expect.poll(focusedStart, { timeout: 2000 }).toBe('24048');
    const lane = await page.locator('.timeline-lane').boundingBox();
    const column = await page.locator('.timeline-column:focus').boundingBox();
    expect(column.x).toBeGreaterThanOrEqual(lane.x - 1);
    expect(column.x + column.width).toBeLessThanOrEqual(lane.x + lane.width + 1);
  });

  test('should adjust both range bounds by keyboard, one month per activation', async ({
    page,
  }) => {
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=5');
    await TestHelpers.waitForPhotosToLoad(page);

    // The start handle is a keyboard-reachable slider announcing its period
    const startHandle = page.locator('.timeline-handle.start');
    await startHandle.focus();
    expect(await startHandle.getAttribute('aria-label')).toContain('Range start');

    await page.keyboard.press('ArrowLeft');
    await TestHelpers.waitForPhotosToLoad(page);
    expect(TestHelpers.getUrlState(page).month).toBe(2);

    const endHandle = page.locator('.timeline-handle.end');
    await endHandle.focus();
    await page.keyboard.press('ArrowRight');
    await TestHelpers.waitForPhotosToLoad(page);
    expect(TestHelpers.getUrlState(page).toMonth).toBe(6);
  });

  test('should announce zoom, fit-all and clear with a visible focus ring', async ({ page }) => {
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);

    for (const selector of [
      '.timeline-zoom-in',
      '.timeline-zoom-out',
      '.timeline-fit-all',
      '.timeline-reset',
    ]) {
      const control = page.locator(selector).first();
      await expect(control).toHaveAttribute('aria-label', /.+/);
      await control.focus();
      const shadow = await control.evaluate((el) => getComputedStyle(el).boxShadow);
      expect(shadow).not.toBe('none');
    }
  });

  test('should refuse to select a zero-photo single period by keyboard or brush', async ({
    page,
  }) => {
    // GIVEN: the 2012 year view, whose roving column is January 2012 — a month
    // with no photos (April 2012, 24147, is the empty month the click test uses)
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);
    const roving = page.locator('.timeline-column[tabindex="0"]');
    await expect(roving).toHaveAttribute('data-period-start', '24144');

    // WHEN: Shift+Enter extends the whole-year selection onto that same empty
    // month, which is exactly one zero-photo period
    await roving.focus();
    await page.keyboard.press('Shift+Enter');

    // THEN: nothing is committed — the filter still reads as the whole year
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 2012,
      month: null,
      toYear: null,
      toMonth: null,
    });

    // AND: the same holds for a brush confined to one empty month, live scrub
    // included (the pending scrub write would land 100 ms later, so the wait is
    // what makes "nothing happened" an assertion rather than a race)
    const april = await page.locator('.timeline-column[data-period-start="24147"]').boundingBox();
    const midY = april.y + april.height / 2;
    await page.mouse.move(april.x + april.width / 2, midY);
    await page.mouse.down();
    await page.mouse.move(april.x + april.width / 2 + 6, midY, { steps: 3 });
    await page.mouse.up();
    await page.waitForTimeout(200);
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 2012,
      month: null,
      toYear: null,
      toMonth: null,
    });
    await expect(page.locator('.photo-card').first()).toBeAttached();

    // AND: a bound cannot be walked onto an empty month either. March–April
    // 2012 is a range (April is the empty month), so ArrowRight on the start
    // handle would clamp onto the end bound and collapse the range into that
    // one zero-photo month — which must not commit.
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=4');
    await TestHelpers.waitForPhotosToLoad(page);
    const startHandle = page.locator('.timeline-handle.start');
    await startHandle.focus();
    await page.keyboard.press('ArrowRight');
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 2012,
      month: 3,
      toYear: 2012,
      toMonth: 4,
    });
  });
});
