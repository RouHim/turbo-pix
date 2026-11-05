import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';
import { SIDEBAR_COLLAPSED_STORAGE_KEY } from '../../../frontend/src/lib/sidebarPreference.js';

const sidebar = (page) => page.getByTestId('sidebar-nav');
const menuBtn = (page) => page.getByTestId('sidebar-toggle');
const mainContent = (page) => page.getByTestId('main-content');

const storedCollapsed = (page) =>
  page.evaluate((key) => localStorage.getItem(key), SIDEBAR_COLLAPSED_STORAGE_KEY);

const computedSidebarWidth = (page) =>
  page.evaluate(
    () => window.getComputedStyle(document.querySelector('[data-testid="sidebar-nav"]')).width
  );

test.describe('Sidebar collapse', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.setDesktopViewport(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('desktop starts expanded with the sidebar width reserved', async ({ page }) => {
    // GIVEN: A fresh desktop session (no stored preference)
    const sidebarWidth = await computedSidebarWidth(page);

    // THEN: The sidebar is visible and the content honours its width
    expect(parseFloat(sidebarWidth)).toBeGreaterThan(0);
    await expect(sidebar(page)).toBeVisible();
    await expect(mainContent(page)).toHaveCSS('margin-left', sidebarWidth);
    await expect(menuBtn(page)).toBeVisible();
    await expect(menuBtn(page)).toHaveAttribute('aria-expanded', 'true');
    expect(await storedCollapsed(page)).toBeNull();
  });

  test('collapses, reclaims the content width, and expands again', async ({ page }) => {
    // GIVEN: The expanded desktop shell
    const sidebarWidth = await computedSidebarWidth(page);

    // WHEN: The header toggle is activated
    await menuBtn(page).click();

    // THEN: The sidebar leaves the layout and the content fills the viewport
    await expect(sidebar(page)).toBeHidden();
    await expect(mainContent(page)).toHaveCSS('margin-left', '0px');
    await expect(menuBtn(page)).toHaveAttribute('aria-expanded', 'false');
    await expect(menuBtn(page)).toHaveAttribute('aria-label', 'Expand sidebar');
    expect(await storedCollapsed(page)).toBe('true');

    // WHEN: The toggle is activated again
    await menuBtn(page).click();

    // THEN: The sidebar is back and the reserved width returns
    await expect(sidebar(page)).toBeVisible();
    await expect(mainContent(page)).toHaveCSS('margin-left', sidebarWidth);
    await expect(menuBtn(page)).toHaveAttribute('aria-expanded', 'true');
    expect(await storedCollapsed(page)).toBe('false');
  });

  test('keeps the collapsed choice across a reload', async ({ page }) => {
    // GIVEN: A collapsed shell
    await menuBtn(page).click();
    await expect(sidebar(page)).toBeHidden();

    // WHEN: The page is reloaded
    await page.reload({ waitUntil: 'domcontentloaded' });
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: It comes back collapsed
    await expect(sidebar(page)).toBeHidden();
    await expect(mainContent(page)).toHaveCSS('margin-left', '0px');
    await expect(menuBtn(page)).toHaveAttribute('aria-expanded', 'false');

    // WHEN: The user expands it again
    await menuBtn(page).click();

    // THEN: The expanded state is what the next load will see
    await expect(sidebar(page)).toBeVisible();
    expect(await storedCollapsed(page)).toBe('false');
  });

  test('moves focus to the toggle when collapsing a focused sidebar', async ({ page }) => {
    // GIVEN: Keyboard focus inside the sidebar
    await page.locator('button[data-view="favorites"]').focus();
    expect(await page.evaluate(() => document.activeElement?.dataset.view)).toBe('favorites');

    // WHEN: The sidebar collapses without the click itself moving focus
    await page.evaluate(() => document.querySelector('[data-testid="sidebar-toggle"]').click());
    await expect(sidebar(page)).toBeHidden();

    // THEN: Focus is not lost to the document body
    expect(
      await page.evaluate(
        () => document.activeElement?.getAttribute('data-testid') === 'sidebar-toggle'
      )
    ).toBe(true);
  });

  test('mobile drawer is unaffected and keeps the desktop preference', async ({ page }) => {
    // GIVEN: A collapsed desktop shell
    await menuBtn(page).click();
    await expect(sidebar(page)).toBeHidden();

    // WHEN: The viewport crosses into the mobile breakpoint
    await TestHelpers.setMobileViewport(page);

    // THEN: The drawer stays closed until the hamburger opens it
    await expect(sidebar(page)).toBeHidden();
    await menuBtn(page).click();
    await expect(sidebar(page)).toBeVisible();
    await expect(page.getByTestId('sidebar-overlay')).toBeVisible();

    // Escape is the drawer's own affordance; the drawer body covers the
    // overlay's centre at phone widths.
    await page.keyboard.press('Escape');
    await expect(sidebar(page)).toBeHidden();

    // WHEN: The viewport returns to desktop
    await TestHelpers.setDesktopViewport(page);

    // THEN: The stored desktop preference still wins
    await expect(sidebar(page)).toBeHidden();
    expect(await storedCollapsed(page)).toBe('true');
  });
});
