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

  test('indexing orbit should have saturate and blur in backdrop-filter', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('[data-phase-ring]').setAttribute('data-ring-mode', 'large');
    });

    const backdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('[data-phase-ring]')).backdropFilter
    );

    expect(backdropFilter).toContain('saturate');
    expect(backdropFilter).toContain('blur');
  });

  test('indexing orbit compact mode should be positioned at bottom-right', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('[data-phase-ring]').setAttribute('data-ring-mode', 'compact');
    });

    const styles = await page.evaluate(() => {
      const ring = document.querySelector('[data-phase-ring]');
      const computed = window.getComputedStyle(ring);
      return {
        position: computed.position,
        bottom: computed.bottom,
        right: computed.right,
      };
    });

    expect(styles.position).toBe('fixed');
    expect(parseFloat(styles.bottom)).toBeGreaterThanOrEqual(0);
    expect(parseFloat(styles.right)).toBeGreaterThanOrEqual(0);
    expect(styles.bottom).not.toBe('auto');
    expect(styles.right).not.toBe('auto');
  });

  test('sidebar should NOT be offset when indexing orbit is visible', async ({ page }) => {
    await TestHelpers.setDesktopViewport(page);
    await page.evaluate(() => {
      document.querySelector('[data-phase-ring]').setAttribute('data-ring-mode', 'compact');
    });

    const result = await page.evaluate(() => {
      const sidebarTop = parseFloat(
        window.getComputedStyle(document.querySelector('.sidebar')).top
      );
      const headerHeight = parseFloat(
        window.getComputedStyle(document.querySelector('.header')).height
      );
      return { sidebarTop, headerHeight };
    });

    expect(result.sidebarTop).toBe(result.headerHeight);
  });

  test('header and sidebar CSS should declare -webkit-backdrop-filter', async ({ page }) => {
    const cssResponse = await page.request.get('/css/main.css');
    const cssText = await cssResponse.text();

    expect(cssText).toContain('.header');
    expect(cssText).toContain('.sidebar');
    expect(cssText).toContain('-webkit-backdrop-filter');
  });

  test('indexing orbit CSS should declare -webkit-backdrop-filter', async ({ page }) => {
    const cssResponse = await page.request.get('/css/components.css');
    const cssText = await cssResponse.text();

    expect(cssText).toContain('[data-phase-ring]');
    expect(cssText).toContain('-webkit-backdrop-filter');
  });

  test('layout should render correctly in light theme', async ({ page }) => {
    await page.evaluate(() => {
      document.documentElement.classList.add('light-theme');
      document.documentElement.classList.remove('dark-theme');
      document.querySelector('[data-phase-ring]').setAttribute('data-ring-mode', 'large');
    });

    await TestHelpers.setDesktopViewport(page);

    await expect(page.locator('.header')).toBeVisible();
    await expect(page.locator('.sidebar')).toBeVisible();
    await expect(page.locator('[data-phase-ring]')).toBeVisible();

    mkdirSync(EVIDENCE_DIR, { recursive: true });
    await page.screenshot({ path: `${EVIDENCE_DIR}/task-1-light-theme.png` });
  });

  test('layout should render correctly in dark theme', async ({ page }) => {
    await page.evaluate(() => {
      document.documentElement.classList.add('dark-theme');
      document.documentElement.classList.remove('light-theme');
      document.querySelector('[data-phase-ring]').setAttribute('data-ring-mode', 'large');
    });

    await TestHelpers.setDesktopViewport(page);

    await expect(page.locator('.header')).toBeVisible();
    await expect(page.locator('.sidebar')).toBeVisible();
    await expect(page.locator('[data-phase-ring]')).toBeVisible();

    mkdirSync(EVIDENCE_DIR, { recursive: true });
    await page.screenshot({ path: `${EVIDENCE_DIR}/task-1-dark-theme.png` });
  });
});
