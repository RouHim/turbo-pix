import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';
import { SIDEBAR_COLLAPSED_STORAGE_KEY } from '../../../frontend/src/lib/sidebarPreference.js';

const sidebar = (page) => page.getByTestId('sidebar-nav');
const collapseToggle = (page) => page.getByTestId('sidebar-collapse-toggle');
const mobileMenu = (page) => page.getByTestId('sidebar-toggle');
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

  test('desktop starts expanded with the edge tab and no header toggle', async ({ page }) => {
    // GIVEN: A fresh desktop session (no stored preference)
    const sidebarWidth = await computedSidebarWidth(page);

    // THEN: The sidebar is visible and the content honours its width
    expect(parseFloat(sidebarWidth)).toBeGreaterThan(0);
    await expect(sidebar(page)).toBeVisible();
    await expect(mainContent(page)).toHaveCSS('margin-left', sidebarWidth);

    // AND: The collapse control sits on the sidebar edge, not in the header
    await expect(collapseToggle(page)).toBeVisible();
    await expect(collapseToggle(page)).toHaveAttribute('aria-expanded', 'true');
    await expect(mobileMenu(page)).toBeHidden();

    const tab = await page.evaluate(() => {
      const hit = document
        .querySelector('[data-testid="sidebar-collapse-toggle"]')
        .getBoundingClientRect();
      const inner = document
        .querySelector('[data-testid="sidebar-collapse-toggle"] .sidebar-toggle-tab')
        .getBoundingClientRect();
      const sidebarWidth = document
        .querySelector('[data-testid="sidebar-nav"]')
        .getBoundingClientRect().width;
      return { hitW: hit.width, hitH: hit.height, tabX: inner.x, tabW: inner.width, sidebarWidth };
    });
    expect(tab.hitW).toBe(44);
    expect(tab.hitH).toBe(48);
    expect(tab.tabW).toBe(24);
    expect(tab.tabX).toBeCloseTo(tab.sidebarWidth - 12, 0);

    // AND: The brand is the first header element again
    const layout = await page.evaluate(() => {
      const brand = document.querySelector('[data-testid="brand"]').getBoundingClientRect();
      const padding = getComputedStyle(
        document.querySelector('[data-testid="header-content"]')
      ).paddingLeft;
      return { brandX: brand.x, paddingLeft: parseFloat(padding) };
    });
    expect(layout.brandX).toBeCloseTo(layout.paddingLeft, 0);
    expect(await storedCollapsed(page)).toBeNull();
  });

  test('collapses, reclaims the content width, and expands again', async ({ page }) => {
    // GIVEN: The expanded desktop shell
    const sidebarWidth = await computedSidebarWidth(page);

    // WHEN: The edge tab is activated
    await collapseToggle(page).click();

    // THEN: The sidebar leaves the layout and the content fills the viewport
    await expect(sidebar(page)).toBeHidden();
    await expect(mainContent(page)).toHaveCSS('margin-left', '0px');
    await expect(collapseToggle(page)).toBeVisible();
    await expect(collapseToggle(page)).toHaveAttribute('aria-expanded', 'false');
    await expect(collapseToggle(page)).toHaveAttribute('aria-label', 'Expand sidebar');
    expect(await storedCollapsed(page)).toBe('true');

    // AND: the tab is now flush with the viewport edge
    const collapsedTabX = await page.evaluate(
      () =>
        document
          .querySelector('[data-testid="sidebar-collapse-toggle"] .sidebar-toggle-tab')
          .getBoundingClientRect().x
    );
    expect(collapsedTabX).toBe(0);

    // WHEN: The tab is activated again
    await collapseToggle(page).click();

    // THEN: The sidebar is back and the reserved width returns
    await expect(sidebar(page)).toBeVisible();
    await expect(mainContent(page)).toHaveCSS('margin-left', sidebarWidth);
    await expect(collapseToggle(page)).toHaveAttribute('aria-expanded', 'true');
    expect(await storedCollapsed(page)).toBe('false');
  });

  test('keeps the collapsed choice across a reload', async ({ page }) => {
    // GIVEN: A collapsed shell
    await collapseToggle(page).click();
    await expect(sidebar(page)).toBeHidden();

    // WHEN: The page is reloaded
    await page.reload({ waitUntil: 'domcontentloaded' });
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: It comes back collapsed, with the tab still offering the way out
    await expect(sidebar(page)).toBeHidden();
    await expect(mainContent(page)).toHaveCSS('margin-left', '0px');
    await expect(collapseToggle(page)).toBeVisible();
    await expect(collapseToggle(page)).toHaveAttribute('aria-expanded', 'false');

    // WHEN: The user expands it again
    await collapseToggle(page).click();

    // THEN: The expanded state is what the next load will see
    await expect(sidebar(page)).toBeVisible();
    expect(await storedCollapsed(page)).toBe('false');
  });

  test('mobile drawer is unaffected and keeps the desktop preference', async ({ page }) => {
    // GIVEN: A collapsed desktop shell
    await collapseToggle(page).click();
    await expect(sidebar(page)).toBeHidden();

    // WHEN: The viewport crosses into the mobile breakpoint
    await TestHelpers.setMobileViewport(page);

    // THEN: The edge tab belongs to the desktop shell; the hamburger owns mobile
    await expect(collapseToggle(page)).toBeHidden();
    await expect(mobileMenu(page)).toBeVisible();
    await expect(sidebar(page)).toBeHidden();

    await mobileMenu(page).click();
    await expect(sidebar(page)).toBeVisible();
    await expect(page.getByTestId('sidebar-overlay')).toBeVisible();

    // Escape is the drawer's own affordance; the drawer body covers the
    // overlay's centre at phone widths.
    await page.keyboard.press('Escape');
    await expect(sidebar(page)).toBeHidden();

    // WHEN: The drawer is open with focus inside and the viewport returns to desktop
    await mobileMenu(page).click();
    await expect(sidebar(page)).toBeVisible();
    await page.locator('button[data-view="favorites"]').focus();
    await expect(page.locator('button[data-view="favorites"]')).toBeFocused();
    await TestHelpers.setDesktopViewport(page);

    // THEN: The desktop preference still wins and focus lands on the edge tab
    await expect(collapseToggle(page)).toBeVisible();
    await expect(sidebar(page)).toBeHidden();
    expect(await storedCollapsed(page)).toBe('true');
    await expect
      .poll(() => page.evaluate(() => document.activeElement?.getAttribute('data-testid')), {
        timeout: 2000,
      })
      .toBe('sidebar-collapse-toggle');
  });
});
