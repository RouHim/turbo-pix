import { test, expect } from '@playwright/test';
import { mkdirSync } from 'fs';
import { TestHelpers } from '../setup/test-helpers.js';

const EVIDENCE_DIR = '.sisyphus/evidence';

test.describe('Viewer Glassmorphism', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  const openViewerOnFirstPhoto = async (page) => {
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);
  };

  test('viewer-controls should have saturate in backdrop-filter', async ({ page }) => {
    // GIVEN: Viewer is open
    await openViewerOnFirstPhoto(page);

    // WHEN: We check computed styles on .viewer-controls
    const backdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-controls')).backdropFilter
    );

    // THEN: backdrop-filter includes both saturate and blur
    expect(backdropFilter).toContain('saturate');
    expect(backdropFilter).toContain('blur');
  });

  test('viewer-close should have saturate in backdrop-filter', async ({ page }) => {
    // GIVEN: Viewer is open
    await openViewerOnFirstPhoto(page);

    // WHEN: We check computed styles on .viewer-close
    const backdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-close')).backdropFilter
    );

    // THEN: backdrop-filter includes both saturate and blur
    expect(backdropFilter).toContain('saturate');
    expect(backdropFilter).toContain('blur');
  });

  test('viewer-prev and viewer-next should have saturate in backdrop-filter', async ({ page }) => {
    // GIVEN: Viewer is open
    await openViewerOnFirstPhoto(page);

    // WHEN: We check computed styles on navigation buttons
    const prevBackdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-prev')).backdropFilter
    );
    const nextBackdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-next')).backdropFilter
    );

    // THEN: Both buttons have saturate and blur in backdrop-filter
    expect(prevBackdropFilter).toContain('saturate');
    expect(prevBackdropFilter).toContain('blur');
    expect(nextBackdropFilter).toContain('saturate');
    expect(nextBackdropFilter).toContain('blur');
  });

  test('zoom buttons should have saturate in backdrop-filter', async ({ page }) => {
    // GIVEN: Viewer is open
    await openViewerOnFirstPhoto(page);

    // WHEN: We check computed styles on first .zoom-btn
    const backdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.zoom-btn')).backdropFilter
    );

    // THEN: backdrop-filter includes saturate and blur
    expect(backdropFilter).toContain('saturate');
    expect(backdropFilter).toContain('blur');
  });

  test('viewer-overlay backdrop-filter should include saturate', async ({ page }) => {
    // GIVEN: Viewer is open
    await openViewerOnFirstPhoto(page);

    // WHEN: We check computed styles on .viewer-overlay
    const backdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-overlay')).backdropFilter
    );

    // THEN: backdrop-filter includes saturate
    expect(backdropFilter).toContain('saturate');
  });

  test('viewer-controls should have -webkit-backdrop-filter declared in CSS', async ({ page }) => {
    // GIVEN: Viewer is open
    await openViewerOnFirstPhoto(page);

    // WHEN: We fetch the raw CSS source file
    // Chromium aliases -webkit-backdrop-filter to backdropFilter in computed styles and CSSOM,
    // so we verify presence by reading the raw CSS text instead.
    const cssResponse = await page.request.get('/css/components.css');
    const cssText = await cssResponse.text();

    // THEN: The CSS source includes both .viewer-controls selector and -webkit-backdrop-filter
    expect(cssText).toContain('.viewer-controls');
    expect(cssText).toContain('-webkit-backdrop-filter');
  });

  test('viewer button colors should not be hardcoded white in dark theme', async ({ page }) => {
    // GIVEN: Dark theme is active
    await page.evaluate(() => {
      document.documentElement.classList.add('dark-theme');
      document.documentElement.classList.remove('light-theme');
    });

    // AND: Viewer is open
    await openViewerOnFirstPhoto(page);

    // WHEN: We check the color of .viewer-close
    const closeButtonColor = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-close')).color
    );

    // THEN: Color should not be hardcoded pure white
    // It may resolve to near-white via CSS variable, but should not be literal rgb(255, 255, 255)
    expect(closeButtonColor).not.toBe('rgb(255, 255, 255)');
  });

  test('viewer-controls should have box-shadow', async ({ page }) => {
    // GIVEN: Viewer is open
    await openViewerOnFirstPhoto(page);

    // WHEN: We check box-shadow on .viewer-controls
    const boxShadow = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-controls')).boxShadow
    );

    // THEN: box-shadow is set (not 'none')
    expect(boxShadow).not.toBe('none');
  });

  test('viewer should be functional in light theme', async ({ page }) => {
    // GIVEN: Light theme is active
    await page.evaluate(() => {
      document.documentElement.classList.add('light-theme');
      document.documentElement.classList.remove('dark-theme');
    });

    // WHEN: Viewer is opened
    await openViewerOnFirstPhoto(page);

    // THEN: Viewer is visible and close button exists
    await expect(page.locator('#photo-viewer.active')).toBeVisible();
    await expect(page.locator('.viewer-close')).toBeVisible();

    mkdirSync(EVIDENCE_DIR, { recursive: true });
    await page.screenshot({ path: `${EVIDENCE_DIR}/task-2-viewer-light-theme.png` });
  });

  test('viewer should be functional in dark theme', async ({ page }) => {
    // GIVEN: Dark theme is active
    await page.evaluate(() => {
      document.documentElement.classList.add('dark-theme');
      document.documentElement.classList.remove('light-theme');
    });

    // WHEN: Viewer is opened
    await openViewerOnFirstPhoto(page);

    // THEN: Viewer is visible and close button exists
    await expect(page.locator('#photo-viewer.active')).toBeVisible();
    await expect(page.locator('.viewer-close')).toBeVisible();

    mkdirSync(EVIDENCE_DIR, { recursive: true });
    await page.screenshot({ path: `${EVIDENCE_DIR}/task-2-viewer-dark-theme.png` });
  });

  test('viewer controls visible on mobile viewport', async ({ page }) => {
    // GIVEN: Mobile viewport
    await TestHelpers.setMobileViewport(page);

    // WHEN: Viewer is opened
    await openViewerOnFirstPhoto(page);

    // THEN: Viewer controls are visible on small screen
    await expect(page.locator('.viewer-controls')).toBeVisible();
  });
});
