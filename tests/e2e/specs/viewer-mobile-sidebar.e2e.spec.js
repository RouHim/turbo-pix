import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { TestHelpers } from '../setup/test-helpers.js';

const AXE_RULES = [
  'color-contrast',
  'target-size',
  'aria-valid-attr-value',
  'aria-prohibited-attr',
];

test.describe('Mobile Viewer Sidebar', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  const openViewerOnFirstPhoto = async (page) => {
    const photos = await TestHelpers.getPhotoCards(page);
    if (!photos.length) {
      test.skip('No photos available');
    }
    expect(photos.length).toBeGreaterThan(0);
    await photos[0].click();
    await TestHelpers.verifyViewerOpen(page);
  };

  test('sidebar should float above viewer controls on mobile viewport', async ({ page }) => {
    // GIVEN: Mobile viewport + viewer open
    await TestHelpers.setMobileViewport(page);
    await openViewerOnFirstPhoto(page);

    // WHEN: Metadata button clicked to open sidebar
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();

    // THEN: Sidebar z-index is greater than the controls' z-index
    const sidebarZIndex = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-sidebar')).zIndex
    );
    const controlsZIndex = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-controls')).zIndex
    );
    expect(parseInt(sidebarZIndex, 10)).toBeGreaterThan(parseInt(controlsZIndex, 10));
  });

  test('close button should dismiss sidebar without closing viewer', async ({ page }) => {
    // GIVEN: Mobile viewport + viewer open + sidebar visible
    await TestHelpers.setMobileViewport(page);
    await openViewerOnFirstPhoto(page);
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();

    // WHEN: Close button clicked
    await page.locator('#sidebar-close-btn').click();

    // THEN: Sidebar no longer has .show class
    await expect(page.locator('.viewer-sidebar.show')).not.toBeVisible();
    // AND: Viewer is still open
    await expect(page.locator('#photo-viewer')).toBeVisible();
  });

  test('desktop sidebar should not have z-index 15 or absolute positioning', async ({ page }) => {
    // GIVEN: Desktop viewport + viewer open
    await TestHelpers.setDesktopViewport(page);
    await openViewerOnFirstPhoto(page);

    // WHEN: Check computed styles
    const sidebarPosition = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-sidebar')).position
    );
    const sidebarZIndex = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-sidebar')).zIndex
    );

    // THEN: Position is not absolute (should be static from grid flow)
    expect(sidebarPosition).not.toBe('absolute');
    // AND: z-index is not 15
    expect(sidebarZIndex).not.toBe('15');
  });

  test('sidebar should have backdrop-filter blur on mobile', async ({ page }) => {
    // GIVEN: Mobile viewport + viewer open + sidebar visible
    await TestHelpers.setMobileViewport(page);
    await openViewerOnFirstPhoto(page);
    await page.locator('.metadata-btn').click();
    await page.locator('.viewer-sidebar.show').waitFor();

    // WHEN: Check computed styles
    const backdropFilter = await page.evaluate(
      () => window.getComputedStyle(document.querySelector('.viewer-sidebar')).backdropFilter
    );

    // THEN: backdrop-filter contains blur
    expect(backdropFilter).toContain('blur');
  });

  test('viewer and details sheet should have no axe violations on mobile', async ({ page }) => {
    test.setTimeout(60000);
    await TestHelpers.setMobileViewport(page);
    await openViewerOnFirstPhoto(page);
    await expect(page.locator('#viewer-image')).toHaveClass(/loaded/, { timeout: 20000 });

    // The viewer chrome itself, including the pinned details button and the
    // 44 px mobile controls.
    const viewer = await new AxeBuilder({ page })
      .include('#photo-viewer')
      .withRules(AXE_RULES)
      .analyze();
    expect(viewer.violations).toEqual([]);

    // The sheet is a separate rendered surface; contrast and target-size
    // regressions there were invisible to the map/timeline axe specs.
    await page.locator('.metadata-btn').click();
    await expect(page.getByTestId('viewer-sidebar')).toHaveClass(/show/);
    const sheet = await new AxeBuilder({ page })
      .include('[data-testid="viewer-sidebar"]')
      .withRules(AXE_RULES)
      .analyze();
    expect(sheet.violations).toEqual([]);
  });
});
