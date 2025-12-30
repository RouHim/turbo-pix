import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Navigation', () => {
  test.beforeEach(async ({ page }) => {
    // Setup console monitoring
    TestHelpers.setupConsoleMonitoring(page);

    // Navigate to home page
    await page.goto('/');
  });

  test('should load the homepage successfully', async ({ page }) => {
    // Verify page loaded
    await expect(page).toHaveTitle('TurboPix');

    // Verify header is present
    const header = page.locator('.header');
    await expect(header).toBeVisible();

    // Verify default view is "All Photos"
    await TestHelpers.verifyActiveView(page, 'all');
  });

  test('should navigate to favorites view', async ({ page }) => {
    // Navigate to favorites
    await TestHelpers.navigateToView(page, 'favorites');

    // Verify URL updated (if using hash routing)
    // expect(page.url()).toContain('favorites');

    // Verify active view
    await TestHelpers.verifyActiveView(page, 'favorites');

    // Verify view title updated
    const viewTitle = page.locator(TestHelpers.selectors.viewTitle);
    await expect(viewTitle).toBeVisible();
  });

  test('should navigate to videos view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.verifyActiveView(page, 'videos');
  });

  test('should navigate to collages view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.verifyActiveView(page, 'collages');
  });

  test('should navigate to housekeeping view', async ({ page }) => {
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.verifyActiveView(page, 'housekeeping');
  });

  test('should navigate between multiple views', async ({ page }) => {
    // Navigate through all views
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.verifyActiveView(page, 'favorites');

    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.verifyActiveView(page, 'videos');

    await TestHelpers.navigateToView(page, 'all');
    await TestHelpers.verifyActiveView(page, 'all');

    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.verifyActiveView(page, 'collages');

    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.verifyActiveView(page, 'housekeeping');
  });

  test('should return to all photos view when clicking logo', async ({ page }) => {
    // Navigate away from home
    await TestHelpers.navigateToView(page, 'favorites');

    // Click logo
    const logoLink = page.locator('#logo-link');
    await logoLink.click();

    // Verify back to all photos
    await TestHelpers.verifyActiveView(page, 'all');
  });

  test('should display photo grid in all photos view', async ({ page }) => {
    // Wait for photos to load
    await TestHelpers.waitForPhotosToLoad(page);

    // Verify grid is visible
    const photoGrid = page.locator(TestHelpers.selectors.photoGrid);
    await expect(photoGrid).toBeVisible();

    // Verify at least one photo card exists or empty state
    const hasPhotos = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasPhotos || hasEmptyState).toBe(true);
  });

  test('should toggle theme', async ({ page }) => {
    // Get theme toggle button
    const themeToggle = page.locator('#theme-toggle');
    await expect(themeToggle).toBeVisible();

    // Click to toggle theme
    await themeToggle.click();

    // Wait a bit for transition
    await page.waitForTimeout(300);

    // Theme class should be on html element
    const htmlElement = page.locator('html');
    const htmlClass = await htmlElement.getAttribute('class');
    expect(htmlClass).toBeTruthy();
  });
});

test.describe('Mobile Navigation', () => {
  test.beforeEach(async ({ page, browserName }) => {
    // Skip if not webkit (Mobile Safari)
    test.skip(browserName !== 'webkit', 'Mobile navigation tests only run on webkit');
    
    // Setup console monitoring
    TestHelpers.setupConsoleMonitoring(page);

    // Set mobile viewport
    await TestHelpers.setMobileViewport(page);

    // Navigate to home page
    await page.goto('/');
  });

  test('should open and close mobile sidebar', async ({ page }) => {
    // Find menu button
    const menuBtn = page.locator(TestHelpers.selectors.menuBtn);

    // Click to open sidebar
    await menuBtn.click();

    // Sidebar should be open
    const sidebar = page.locator(TestHelpers.selectors.sidebar);
    await expect(sidebar).toHaveClass(/open/);

    // Click overlay to close
    const overlay = page.locator('.sidebar-overlay');
    await overlay.click();

    // Sidebar should be closed
    await expect(sidebar).not.toHaveClass(/open/);
  });

  test('should navigate using mobile sidebar', async ({ page }) => {
    // Open mobile sidebar
    const menuBtn = page.locator(TestHelpers.selectors.menuBtn);
    await menuBtn.click();

    // Navigate to favorites
    const favoritesBtn = page.locator(TestHelpers.selectors.navItem('favorites'));
    await favoritesBtn.click();

    // Verify navigation happened
    await TestHelpers.verifyActiveView(page, 'favorites');

    // Sidebar should auto-close after navigation
    const sidebar = page.locator(TestHelpers.selectors.sidebar);
    await expect(sidebar).not.toHaveClass(/open/);
  });
});
