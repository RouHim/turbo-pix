import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

async function deleteAllSavedSearches(page) {
  const res = await page.request.get('/api/saved-searches');
  const { saved_searches = [] } = await res.json();
  for (const item of saved_searches) {
    await page.request.delete(`/api/saved-searches/${item.id}`);
  }
}

async function seedSavedSearch(page, name, overrides = {}) {
  const res = await page.request.post('/api/saved-searches', {
    data: {
      name,
      query: 'type:video',
      view: 'all',
      sort: 'date_desc',
      year: null,
      month: null,
      ...overrides,
    },
  });
  expect(res.status()).toBe(201);
  return res.json();
}

test.describe('Saved Searches', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    // Delete BEFORE navigating: the sidebar fetches saved searches on mount,
    // so a post-load delete would leave stale rows in the in-memory list.
    await deleteAllSavedSearches(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForSearchReady(page);
  });

  test('should save the current view and restore it from the sidebar', async ({ page }) => {
    // GIVEN: User has performed a prefix search
    await TestHelpers.performSearch(page, 'type:video');
    await TestHelpers.waitForSearchParam(page, 'type:video');

    // WHEN: User saves the current view
    await page.click('[data-testid="save-search-btn"]');

    // THEN: A saved-search row appears in the sidebar, auto-named from the query
    const rows = page.locator('[data-testid="saved-search-row"]');
    await expect(rows).toHaveCount(1);
    await expect(rows).toContainText('type:video');

    // WHEN: User navigates away and clicks the saved search
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.verifyActiveView(page, 'favorites');
    await page.click('[data-testid="saved-search-open"]');

    // THEN: The exact query state is restored and results re-run live
    await TestHelpers.waitForSearchParam(page, 'type:video');
    await page.waitForFunction(() => location.pathname === '/');
    await TestHelpers.waitForPhotosToLoad(page);
    const cards = await TestHelpers.getPhotoCards(page);
    expect(cards.length).toBeGreaterThanOrEqual(1);
  });

  test('should block duplicate saves with a notice', async ({ page }) => {
    // GIVEN: A view state is already saved
    await TestHelpers.performSearch(page, 'type:video');
    await TestHelpers.waitForSearchParam(page, 'type:video');
    await page.click('[data-testid="save-search-btn"]');
    await expect(page.locator('[data-testid="saved-search-row"]')).toHaveCount(1);

    // WHEN: The same view is saved again
    await page.click('[data-testid="save-search-btn"]');

    // THEN: No new entry is created and a friendly notice explains the duplicate
    await expect(page.locator('.toast-info .toast-title')).toHaveText('Search already saved');
    await expect(page.locator('[data-testid="saved-search-row"]')).toHaveCount(1);
  });

  test('should rename a saved search and reject empty names', async ({ page }) => {
    // GIVEN: A saved search exists
    await seedSavedSearch(page, 'E2E temp');
    await page.reload();
    await TestHelpers.waitForSearchReady(page);

    // WHEN: The user renames it
    await page.click('[data-testid="saved-search-rename"]');
    await page.fill('[data-testid="saved-search-name-input"]', 'Beach 2023');
    await page.keyboard.press('Enter');

    // THEN: The new name is shown in the sidebar immediately
    const row = page.locator('[data-testid="saved-search-row"]');
    await expect(row).toContainText('Beach 2023');

    // WHEN: The user tries an empty rename
    await page.click('[data-testid="saved-search-rename"]');
    await page.fill('[data-testid="saved-search-name-input"]', '   ');
    await page.keyboard.press('Enter');

    // THEN: The rename is rejected and the previous name is kept
    await expect(page.locator('.toast-error .toast-title')).toHaveText('Name cannot be empty');
    await expect(row).toContainText('Beach 2023');
    await page.keyboard.press('Escape');
  });

  test('should delete a saved search without affecting the current view', async ({ page }) => {
    // GIVEN: A saved search exists
    await seedSavedSearch(page, 'E2E temp');
    await page.reload();
    await TestHelpers.waitForSearchReady(page);
    await expect(page.locator('[data-testid="saved-search-row"]')).toHaveCount(1);

    // WHEN: The user deletes it
    await page.click('[data-testid="saved-search-delete"]');

    // THEN: The entry disappears and the view is unchanged
    await expect(page.locator('[data-testid="saved-search-row"]')).toHaveCount(0);
    expect(new URL(page.url()).pathname).toBe('/');
    const res = await page.request.get('/api/saved-searches');
    const { saved_searches = [] } = await res.json();
    expect(saved_searches.length).toBe(0);
  });
});
