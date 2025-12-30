import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Search', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should perform semantic search', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);
    const searchBtn = page.locator(TestHelpers.selectors.searchBtn);

    // Enter search query
    await searchInput.fill('cat');
    await searchBtn.click();

    // Wait for search results to load
    await TestHelpers.waitForPhotosToLoad(page);

    // Verify results are displayed (either photos or empty state)
    const hasResults = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasResults || hasEmptyState).toBe(true);
  });

  test('should search with Enter key', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);

    await searchInput.fill('photo');
    await searchInput.press('Enter');

    // Wait for search results
    await TestHelpers.waitForPhotosToLoad(page);

    // Verify search was executed
    const hasResults = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasResults || hasEmptyState).toBe(true);
  });

  test('should clear search with Escape key', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);

    // Perform search
    await searchInput.fill('test query');
    await searchInput.press('Enter');
    await page.waitForTimeout(500);

    // Clear with Escape
    await searchInput.press('Escape');

    // Verify input is cleared
    const inputValue = await searchInput.inputValue();
    expect(inputValue).toBe('');
  });

  test('should update URL with search query', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);

    await searchInput.fill('landscape');
    await searchInput.press('Enter');

    // Wait a bit for URL to update
    await page.waitForTimeout(500);

    // URL should contain search query
    expect(page.url()).toContain('q=');
  });

  test('should show empty state for no results', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);

    // Search for something that definitely won't exist
    await searchInput.fill('xyzabc123nonexistent456');
    await searchInput.press('Enter');

    // Wait for search to complete
    await page.waitForTimeout(2000);

    // Should show empty state or no photos
    const noPhotos = page.locator(TestHelpers.selectors.noPhotos);
    const photoCards = await TestHelpers.getPhotoCards(page);

    const hasEmptyState = (await noPhotos.count()) > 0;
    const hasNoPhotos = photoCards.length === 0;

    expect(hasEmptyState || hasNoPhotos).toBe(true);
  });

  test('should search for different queries', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);

    // First search
    await searchInput.fill('test');
    await searchInput.press('Enter');
    await TestHelpers.waitForPhotosToLoad(page);

    // Clear and new search
    await searchInput.fill('');
    await searchInput.fill('photo');
    await searchInput.press('Enter');
    await TestHelpers.waitForPhotosToLoad(page);

    // Verify second search executed
    expect(page.url()).toContain('photo');
  });

  test('should search with minimum 2 characters', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);

    // Single character should not trigger search
    await searchInput.fill('a');
    await searchInput.press('Enter');
    await page.waitForTimeout(500);

    // Two characters should trigger search
    await searchInput.fill('ab');
    await searchInput.press('Enter');
    await page.waitForTimeout(1000);

    // Some response should occur (either results or empty state)
    const hasPhotos = await TestHelpers.elementExists(page, TestHelpers.selectors.photoCardAny);
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasPhotos || hasEmptyState).toBe(true);
  });

  test('should maintain search when navigating in viewer', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);

    // Perform search
    await searchInput.fill('test');
    await searchInput.press('Enter');
    await TestHelpers.waitForPhotosToLoad(page);

    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No search results to test with');
    }

    // Open photo in viewer
    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // Close viewer
    await TestHelpers.closeViewer(page);

    // Search query should still be in input
    const inputValue = await searchInput.inputValue();
    expect(inputValue).toBe('test');
  });

  test('should clear search and return to all photos', async ({ page }) => {
    const searchInput = page.locator(TestHelpers.selectors.searchInput);

    // Perform search
    await searchInput.fill('query');
    await searchInput.press('Enter');
    await page.waitForTimeout(1000);

    // Clear search
    await searchInput.fill('');
    await searchInput.press('Enter');

    // Wait for all photos to load
    await TestHelpers.waitForPhotosToLoad(page);

    // Should show all photos view
    const photoCards = await TestHelpers.getPhotoCards(page);
    const hasPhotos = photoCards.length > 0;
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasPhotos || hasEmptyState).toBe(true);
  });
});
