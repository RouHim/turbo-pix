import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Search', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should display search input', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: Page loads
    // THEN: Search input is visible
    await expect(page.locator(TestHelpers.selectors.searchInput)).toBeVisible();
  });

  test('should perform semantic search', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User enters a search term and clicks search
    await TestHelpers.performSearch(page, 'cat');

    // THEN: Search is performed
    await TestHelpers.waitForSearchParam(page, 'cat');

    // AND: URL contains search query
    const url = new URL(page.url());
    expect(url.searchParams.get('q')).toBe('cat');
  });

  test('should search with Enter key', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User enters search term and presses Enter
    await page.fill(TestHelpers.selectors.searchInput, 'dog');
    await page.keyboard.press('Enter');

    // THEN: Search is performed
    await TestHelpers.waitForSearchParam(page, 'dog');

    // AND: URL contains search query
    const url = new URL(page.url());
    expect(url.searchParams.get('q')).toBe('dog');
  });

  test('should clear search with Escape', async ({ page }) => {
    // GIVEN: User has performed a search
    await TestHelpers.performSearch(page, 'test');
    await TestHelpers.waitForSearchParam(page, 'test');

    // WHEN: User presses Escape
    await TestHelpers.clearSearch(page);

    // THEN: Search input is cleared
    const searchValue = await page.locator(TestHelpers.selectors.searchInput).inputValue();
    expect(searchValue).toBe('');
  });

  test('should support URL query params', async ({ page }) => {
    // GIVEN: User navigates with search query in URL
    // WHEN: Page loads with query param
    await TestHelpers.goto(page, '/?q=cat');
    await TestHelpers.waitForSearchParam(page, 'cat');

    // THEN: Search input contains the query
    const searchValue = await page.locator(TestHelpers.selectors.searchInput).inputValue();
    expect(searchValue).toBe('cat');
  });
});
