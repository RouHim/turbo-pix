import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const scrollbarWidth = (page) =>
  page.evaluate(
    () => getComputedStyle(document.querySelector('[data-testid="main-content"]')).scrollbarWidth
  );

const density = (page) =>
  page.evaluate(() =>
    fetch('/api/photos/timeline')
      .then((response) => response.json())
      .then((payload) => payload.density || [])
  );

/** Is the named card inside the scroll viewport? */
const cardIsVisible = (page, filename) =>
  page.evaluate((name) => {
    const header =
      Number.parseFloat(
        getComputedStyle(document.documentElement).getPropertyValue('--header-height')
      ) || 64;
    const card = [...document.querySelectorAll('#photo-grid .photo-card')].find(
      (element) => element.querySelector('img.photo-card-image')?.alt === name
    );
    if (!card) return false;
    const rect = card.getBoundingClientRect();
    return rect.bottom > header + 1 && rect.top < window.innerHeight;
  }, filename);

test.describe('Date scrubber', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
  });

  const openAllPhotos = async (page) => {
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.getByTestId('date-rail')).toBeVisible();
    // A jump during the initial page load would race the grid's own append.
    await expect(page.locator('#photo-grid')).toHaveAttribute('aria-busy', 'false');
  };

  test('renders over the all view and replaces the native scrollbar', async ({ page }) => {
    await openAllPhotos(page);
    await expect(page.getByTestId('date-track')).toBeVisible();
    await expect(page.getByTestId('date-thumb')).toBeVisible();
    expect(await scrollbarWidth(page)).toBe('none');
  });

  test('keeps the full rail dormant until hover or focus, with a faint density hint', async ({
    page,
  }) => {
    await openAllPhotos(page);
    const rail = page.getByTestId('date-rail');
    const thumb = page.getByTestId('date-thumb');
    const caption = page.getByTestId('date-caption');

    await expect(thumb).toHaveCSS('opacity', '0');
    await expect(caption).toHaveCSS('opacity', '0');
    const hint = Number(
      await page.getByTestId('date-segments').evaluate((el) => getComputedStyle(el).opacity)
    );
    expect(hint).toBeGreaterThan(0.2);
    expect(hint).toBeLessThan(0.5);

    await rail.hover();
    await expect(thumb).toHaveCSS('opacity', '1');
    await expect(caption).toHaveCSS('opacity', '1');

    await page.mouse.move(700, 400);
    await expect(thumb).toHaveCSS('opacity', '0');

    await page.getByTestId('date-track').focus();
    await expect(thumb).toHaveCSS('opacity', '1');
    await expect(thumb).toHaveCSS('box-shadow', /0px 0px 0px 2px/);
  });

  test('floats a glass lane over the grid without reserving a gutter', async ({ page }) => {
    await openAllPhotos(page);
    const rail = page.getByTestId('date-rail');
    const railBox = await rail.boundingBox();
    const gridMetrics = () =>
      page.evaluate(() => {
        const cards = [...document.querySelectorAll('#photo-grid .photo-card')];
        return {
          cardRight: Math.max(...cards.map((card) => card.getBoundingClientRect().right)),
          paddingRight: getComputedStyle(document.querySelector('[data-testid="main-content"]'))
            .paddingRight,
        };
      });
    const before = await gridMetrics();
    expect(Math.round(before.cardRight)).toBeGreaterThan(Math.round(railBox.x));
    expect(Number.parseFloat(before.paddingRight)).toBeLessThan(railBox.width);
    expect(await rail.evaluate((el) => getComputedStyle(el, '::before').opacity)).toBe('0');

    await rail.hover();
    await expect
      .poll(() => rail.evaluate((el) => getComputedStyle(el, '::before').opacity))
      .toBe('1');
    const glass = await rail.evaluate((el) => {
      const style = getComputedStyle(el, '::before');
      return { background: style.backgroundColor, blur: style.backdropFilter };
    });
    expect(glass.background).not.toBe('rgba(0, 0, 0, 0)');
    expect(glass.blur).toContain('blur');
    const after = await gridMetrics();
    expect(Math.round(after.cardRight)).toBe(Math.round(before.cardRight));
    expect(after.paddingRight).toBe(before.paddingRight);
  });

  test.describe('reduced motion', () => {
    test.use({ reducedMotion: 'reduce' });

    test('reveals the rail without transition', async ({ page }) => {
      await openAllPhotos(page);
      const thumb = page.getByTestId('date-thumb');
      const duration = Number.parseFloat(
        await thumb.evaluate((el) => getComputedStyle(el).transitionDuration)
      );
      expect(duration).toBeLessThan(0.05);
      await page.getByTestId('date-rail').hover();
      await expect(thumb).toHaveCSS('opacity', '1');
    });
  });

  test('jumps to the oldest month without writing a date filter', async ({ page }) => {
    await openAllPhotos(page);
    const buckets = await density(page);
    expect(buckets.length).toBeGreaterThan(0);
    const oldest = buckets[0];
    const monthIndex = oldest.year * 12 + oldest.month - 1;
    // The fixture dates are mutable across the suite; derive the month's first
    // photo instead of hard-coding a fixture filename.
    const oldestFilename = await page.evaluate(
      async ({ year, month }) => {
        const response = await fetch(`/api/photos?year=${year}&month=${month}&limit=1`);
        const payload = await response.json();
        return payload.photos?.[0]?.filename ?? '';
      },
      { year: oldest.year, month: oldest.month }
    );
    expect(oldestFilename).not.toBe('');

    await page.getByTestId('date-track').hover();
    await page.locator(`[data-month="${monthIndex}"]`).click({ force: true });
    await expect(page.getByTestId('date-bubble')).toContainText(String(oldest.year));

    // The anchor's proof is the target card itself: it has to be in the
    // viewport. The oldest month is the terminal edge, where the remaining
    // content cannot fill the viewport, so the scroll clamps and the target
    // does not necessarily reach the header row.
    await expect.poll(() => cardIsVisible(page, oldestFilename), { timeout: 10000 }).toBe(true);

    const url = new URL(page.url());
    expect(url.searchParams.get('month')).toBeNull();
    expect(url.searchParams.get('year')).toBeNull();
  });

  test('steps months with the keyboard', async ({ page }) => {
    await openAllPhotos(page);
    const track = page.getByTestId('date-track');
    await track.focus();
    const before = await track.getAttribute('aria-valuetext');
    await track.press('ArrowDown');
    // The key sets the target synchronously; the anchor fetch finishes later.
    await expect.poll(() => track.getAttribute('aria-valuetext')).not.toBe(before);
    await expect(track).toBeFocused();
  });

  test('stays out of small periods, searches and favorites', async ({ page }) => {
    await page.goto('/?year=2019');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.getByTestId('date-rail')).toHaveCount(0);
    expect(await scrollbarWidth(page)).not.toBe('none');

    await page.goto('/?q=legacy');
    await expect(page.locator('main.main-content')).toBeVisible();
    await expect(page.getByTestId('date-rail')).toHaveCount(0);

    await page.goto('/favorites');
    await expect(page.locator('main.main-content')).toBeVisible();
    await expect(page.getByTestId('date-rail')).toHaveCount(0);
  });
});
