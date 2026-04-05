import { test, expect } from '@playwright/test';
import { mkdirSync } from 'fs';
import { TestHelpers } from '../setup/test-helpers.js';

const EVIDENCE_DIR = '.sisyphus/evidence';

test.describe('Layout Glassmorphism', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('header should have saturate and blur in backdrop-filter', async ({ page }) => {
    // GIVEN: The app shell is loaded

    // WHEN: We check computed styles on .header
    const backdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.header')).backdropFilter
    );

    // THEN: backdrop-filter includes both saturate and blur
    expect(backdropFilter).toContain('saturate');
    expect(backdropFilter).toContain('blur');
  });

  test('sidebar should have saturate and blur in backdrop-filter on desktop', async ({ page }) => {
    // GIVEN: Desktop viewport is active
    await TestHelpers.setDesktopViewport(page);

    // WHEN: We check computed styles on .sidebar
    const backdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.sidebar')).backdropFilter
    );

    // THEN: backdrop-filter includes both saturate and blur
    expect(backdropFilter).toContain('saturate');
    expect(backdropFilter).toContain('blur');
  });

  test('indexing-banner should have saturate and blur in backdrop-filter', async ({ page }) => {
    // GIVEN: The indexing banner is visible
    await page.evaluate(() => {
      document.querySelector('#indexing-banner').style.display = 'block';
    });

    // WHEN: We check computed styles on #indexing-banner
    const backdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('#indexing-banner')).backdropFilter
    );

    // THEN: backdrop-filter includes both saturate and blur
    expect(backdropFilter).toContain('saturate');
    expect(backdropFilter).toContain('blur');
  });

  test('indexing-banner should be positioned at bottom-right', async ({ page }) => {
    // GIVEN: The indexing banner is visible
    await page.evaluate(() => {
      document.querySelector('#indexing-banner').style.display = 'block';
    });

    // WHEN: We check computed position styles on #indexing-banner
    const styles = await page.evaluate(() => {
      const banner = document.querySelector('#indexing-banner');
      const computed = window.getComputedStyle(banner);
      return {
        position: computed.position,
        bottom: computed.bottom,
        right: computed.right,
      };
    });

    // THEN: The banner is fixed positioned at the bottom-right corner
    expect(styles.position).toBe('fixed');
    // bottom and right should be small pixel values (not 'auto')
    expect(parseFloat(styles.bottom)).toBeGreaterThanOrEqual(0);
    expect(parseFloat(styles.right)).toBeGreaterThanOrEqual(0);
    expect(styles.bottom).not.toBe('auto');
    expect(styles.right).not.toBe('auto');
  });

  test('sidebar should NOT be offset when indexing banner is visible', async ({ page }) => {
    // GIVEN: Desktop viewport and indexing banner is visible
    await TestHelpers.setDesktopViewport(page);
    await page.evaluate(() => {
      document.querySelector('#indexing-banner').style.display = 'block';
    });

    // WHEN: We check the sidebar top offset and the header height
    const result = await page.evaluate(() => {
      const sidebarTop = parseFloat(
        window.getComputedStyle(document.querySelector('.sidebar')).top
      );
      const headerHeight = parseFloat(
        window.getComputedStyle(document.querySelector('.header')).height
      );
      return { sidebarTop, headerHeight };
    });

    // THEN: Sidebar top equals exactly the header height (no extra offset from banner)
    expect(result.sidebarTop).toBe(result.headerHeight);
  });

  test('header and sidebar CSS should declare -webkit-backdrop-filter', async ({ page }) => {
    // GIVEN: The app shell is loaded

    // WHEN: We fetch the raw CSS source file
    const cssResponse = await page.request.get('/css/main.css');
    const cssText = await cssResponse.text();

    // THEN: The CSS source includes both selectors and -webkit-backdrop-filter
    expect(cssText).toContain('.header');
    expect(cssText).toContain('.sidebar');
    expect(cssText).toContain('-webkit-backdrop-filter');
  });

  test('indexing-banner CSS should declare -webkit-backdrop-filter', async ({ page }) => {
    // GIVEN: The app shell is loaded

    // WHEN: We fetch the raw CSS source file
    const cssResponse = await page.request.get('/css/components.css');
    const cssText = await cssResponse.text();

    // THEN: The CSS source includes the indexing banner selector and -webkit-backdrop-filter
    expect(cssText).toContain('.indexing-banner');
    expect(cssText).toContain('-webkit-backdrop-filter');
  });

  test('layout should render correctly in light theme', async ({ page }) => {
    // GIVEN: Light theme is active and indexing banner is visible
    await page.evaluate(() => {
      document.documentElement.classList.add('light-theme');
      document.documentElement.classList.remove('dark-theme');
      document.querySelector('#indexing-banner').style.display = 'block';
    });

    // WHEN: Desktop viewport is set
    await TestHelpers.setDesktopViewport(page);

    // THEN: Header, sidebar, and banner all render without errors
    await expect(page.locator('.header')).toBeVisible();
    await expect(page.locator('.sidebar')).toBeVisible();
    await expect(page.locator('#indexing-banner')).toBeVisible();

    mkdirSync(EVIDENCE_DIR, { recursive: true });
    await page.screenshot({ path: `${EVIDENCE_DIR}/task-1-light-theme.png` });
  });

  test('layout should render correctly in dark theme', async ({ page }) => {
    // GIVEN: Dark theme is active and indexing banner is visible
    await page.evaluate(() => {
      document.documentElement.classList.add('dark-theme');
      document.documentElement.classList.remove('light-theme');
      document.querySelector('#indexing-banner').style.display = 'block';
    });

    // WHEN: Desktop viewport is set
    await TestHelpers.setDesktopViewport(page);

    // THEN: Header, sidebar, and banner all render without errors
    await expect(page.locator('.header')).toBeVisible();
    await expect(page.locator('.sidebar')).toBeVisible();
    await expect(page.locator('#indexing-banner')).toBeVisible();

    mkdirSync(EVIDENCE_DIR, { recursive: true });
    await page.screenshot({ path: `${EVIDENCE_DIR}/task-1-dark-theme.png` });
  });
});
