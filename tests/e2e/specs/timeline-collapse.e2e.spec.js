import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { TestHelpers } from '../setup/test-helpers.js';
import { TIMELINE_COLLAPSED_STORAGE_KEY } from '../../../frontend/src/lib/timelinePreference.js';

// Same rule set as timeline-a11y.e2e.spec.js: focus appearance is not an axe
// rule in 4.12.x and is covered by the visible ring assertions instead.
const AXE_RULES = [
  'color-contrast',
  'target-size',
  'aria-valid-attr-value',
  'aria-prohibited-attr',
];

const container = (page) => page.locator('.timeline-container');
const details = (page) => page.locator('#timeline-details');
const toggle = (page) => page.getByTestId('timeline-collapse-toggle');
const selector = (page) => page.locator('.timeline-selector');
const reset = (page) => page.locator('.timeline-header .timeline-reset');

const storedCollapsed = (page) =>
  page.evaluate((key) => localStorage.getItem(key), TIMELINE_COLLAPSED_STORAGE_KEY);

const containerHeight = (page) =>
  container(page).evaluate((el) => el.getBoundingClientRect().height);

const gridTop = (page) =>
  page.locator('.photo-grid').evaluate((el) => el.getBoundingClientRect().top);

const columnStarts = (page) =>
  page
    .locator('.timeline-column')
    .evaluateAll((columns) => columns.map((column) => column.dataset.periodStart));

test.describe('Timeline collapse', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.setDesktopViewport(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    // The selector is rendered only when the density has more than two
    // populated years, and that fetch resolves independently of the grid, so a
    // plain locator count right after the grid load can race it into a silent
    // skip. Decide the skip from the same data the component reads.
    const populatedYears = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) => {
          const rows = (data.density || []).filter((row) => row.count > 0);
          return new Set(rows.map((row) => row.year)).size;
        })
    );
    test.skip(populatedYears <= 2, 'Timeline needs at least three populated years');
    await expect(selector(page)).toBeVisible();
  });

  test('starts open on a fresh session with the header controls', async ({ page }) => {
    // GIVEN: A fresh desktop session with no stored timeline preference
    // THEN: The selector is open, the toggle announces the collapse action
    await expect(details(page)).toBeVisible();
    await expect(toggle(page)).toBeVisible();
    await expect(toggle(page)).toHaveAttribute('aria-expanded', 'true');
    await expect(toggle(page)).toHaveAttribute('aria-controls', 'timeline-details');
    await expect(toggle(page)).toHaveAttribute('aria-label', 'Collapse timeline');
    await expect(reset(page)).toBeVisible();
    expect(await storedCollapsed(page)).toBeNull();

    // AND: The full 210px card (header 32 + gap 12 + selector 124 + padding/border)
    expect(await containerHeight(page)).toBeCloseTo(210, 0);
  });

  test('collapses to the header row and gives the space to the grid', async ({ page }) => {
    // GIVEN: The open card above the grid
    const gridBefore = await gridTop(page);

    // WHEN: The toggle is activated
    await toggle(page).click();

    // THEN: Only the selector is hidden; label and reset stay reachable
    await expect(details(page)).toBeHidden();
    await expect(selector(page)).toBeHidden();
    await expect(toggle(page)).toHaveAttribute('aria-expanded', 'false');
    await expect(toggle(page)).toHaveAttribute('aria-label', 'Expand timeline');
    await expect(page.locator('.timeline-label')).toBeVisible();
    await expect(reset(page)).toBeVisible();
    expect(await storedCollapsed(page)).toBe('true');

    // AND: The card is header 32 + padding 40 + border 2, and the grid moved up
    expect(await containerHeight(page)).toBeCloseTo(74, 0);
    expect(gridBefore - (await gridTop(page))).toBeCloseTo(136, 0);

    // WHEN: The toggle is activated again
    await toggle(page).click();

    // THEN: The selector returns and the preference records the open state
    await expect(selector(page)).toBeVisible();
    await expect(toggle(page)).toHaveAttribute('aria-expanded', 'true');
    expect(await storedCollapsed(page)).toBe('false');
  });

  test('keeps the collapsed choice across a reload', async ({ page }) => {
    // GIVEN: A collapsed card
    await toggle(page).click();
    await expect(selector(page)).toBeHidden();

    // WHEN: The page is reloaded
    await page.reload({ waitUntil: 'domcontentloaded' });
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: It comes back collapsed, with the toggle offering the way out
    await expect(selector(page)).toBeHidden();
    await expect(toggle(page)).toHaveAttribute('aria-expanded', 'false');
    expect(await storedCollapsed(page)).toBe('true');

    // WHEN: The user expands it
    await toggle(page).click();

    // THEN: The open state is what the next load will see
    await expect(selector(page)).toBeVisible();
    expect(await storedCollapsed(page)).toBe('false');
  });

  test('moves focus to the toggle when the selector is folded away', async ({ page }) => {
    // GIVEN: Keyboard focus on a timeline column
    const column = page.locator('.timeline-column').first();
    await column.focus();
    await expect(column).toBeFocused();

    // WHEN: The fold is triggered without a pointer focus move
    await page.evaluate(() => document.querySelector('.timeline-collapse').click());

    // THEN: Focus is not stranded on the now-hidden selector
    await expect(toggle(page)).toBeFocused();
    await expect(selector(page)).toBeHidden();
  });

  test('keeps the selector zoom across a fold', async ({ page }) => {
    // GIVEN: A fit-all selector
    const fitAll = await columnStarts(page);

    // WHEN: The selector is zoomed in twice
    await page.locator('.timeline-zoom-in').click();
    await page.locator('.timeline-zoom-in').click();
    const zoomed = await columnStarts(page);

    // THEN: The zoom changed the rendered periods, so a fold is measurable
    expect(zoomed).not.toEqual(fitAll);

    // WHEN: The card is folded and reopened
    await toggle(page).click();
    await expect(details(page)).toBeHidden();
    await toggle(page).click();

    // THEN: The selector was not remounted into its fit-all state
    await expect(selector(page)).toBeVisible();
    expect(await columnStarts(page)).toEqual(zoomed);
  });

  test('folds and unfolds with Enter and Space from the keyboard', async ({ page }) => {
    // GIVEN: Focus on the toggle
    await toggle(page).focus();
    await expect(toggle(page)).toBeFocused();

    // WHEN: Enter activates the collapse
    await page.keyboard.press('Enter');

    // THEN: The selector folds and focus stays on the toggle
    await expect(details(page)).toBeHidden();
    await expect(toggle(page)).toHaveAttribute('aria-expanded', 'false');
    await expect(toggle(page)).toBeFocused();

    // WHEN: Space activates the expand
    await page.keyboard.press('Space');

    // THEN: The selector returns and focus stays on the toggle
    await expect(selector(page)).toBeVisible();
    await expect(toggle(page)).toHaveAttribute('aria-expanded', 'true');
    await expect(toggle(page)).toBeFocused();
  });

  test('clears the filter and resets the view while folded', async ({ page }) => {
    // GIVEN: A narrow desktop where a stale month-level view cannot clamp to fit-all
    await page.setViewportSize({ width: 800, height: 800 });
    await page.reload({ waitUntil: 'domcontentloaded' });
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(selector(page)).toBeVisible();
    await page.locator('.timeline-fit-all').click();
    const fitAll = await columnStarts(page);

    // AND: The next mount starts folded, so the lane is never measured
    await page.evaluate((key) => localStorage.setItem(key, 'true'), TIMELINE_COLLAPSED_STORAGE_KEY);
    await page.reload({ waitUntil: 'domcontentloaded' });
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(details(page)).toBeHidden();
    await expect(selector(page)).toBeHidden();

    // WHEN: The folded header's reset is used, then the card is expanded
    await reset(page).click();
    await toggle(page).click();
    await expect(selector(page)).toBeVisible();

    // THEN: The deferred reset lands on fit-all once the lane is measured
    await expect.poll(() => columnStarts(page)).toEqual(fitAll);
  });

  test('resets to fit-all when the fold outlives a resize', async ({ page }) => {
    // GIVEN: A measured, zoomed lane on a wide desktop
    await page.locator('.timeline-zoom-in').click();
    await page.locator('.timeline-zoom-in').click();

    // WHEN: The card is folded, the window narrows, and reset is used while folded
    await toggle(page).click();
    await expect(details(page)).toBeHidden();
    await page.setViewportSize({ width: 1100, height: 900 });
    await reset(page).click();

    // AND: The card expands
    await toggle(page).click();
    await expect(selector(page)).toBeVisible();

    // THEN: The view is fit-all for the NEW width, not the pre-resize one
    const newestYear = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((response) => response.json())
        .then((data) =>
          Math.max(...(data.density || []).filter((row) => row.count > 0).map((row) => row.year))
        )
    );
    const newestDecade = `${Math.floor(newestYear / 10) * 10}s`;
    await expect(page.locator('.timeline-ruler')).toContainText(newestDecade);
  });

  test('keeps the desktop preference out of the mobile dropdowns', async ({ page }) => {
    // GIVEN: A collapsed desktop card
    await toggle(page).click();
    await expect(selector(page)).toBeHidden();

    // WHEN: The viewport crosses into the mobile breakpoint
    await TestHelpers.setMobileViewport(page);

    // THEN: The fold belongs to the desktop rail; the dropdowns stay usable
    await expect(toggle(page)).toBeHidden();
    await expect(page.locator('#timeline-year-select')).toBeVisible();
    await expect(selector(page)).toBeHidden();
    expect(await storedCollapsed(page)).toBe('true');

    // WHEN: It returns to desktop
    await TestHelpers.setDesktopViewport(page);

    // THEN: The desktop preference still wins
    await expect(toggle(page)).toBeVisible();
    await expect(toggle(page)).toHaveAttribute('aria-expanded', 'false');
    await expect(selector(page)).toBeHidden();
  });

  test('has no axe violations while collapsed', async ({ page }) => {
    // GIVEN: The collapsed card
    await toggle(page).click();
    await expect(selector(page)).toBeHidden();

    // THEN: The header row (label, reset, toggle) scans clean
    const results = await new AxeBuilder({ page })
      .include('.timeline-container')
      .withRules(AXE_RULES)
      .analyze();
    expect(results.violations).toEqual([]);
  });
});
