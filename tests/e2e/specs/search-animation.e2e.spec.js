import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Search animation', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForSearchReady(page);
  });

  test('button keeps its label and input shows a spinner while searching', async ({ page }) => {
    // GIVEN: User is on the homepage with a searchable library

    // WHEN: User starts a semantic search (slow: ~3s embedding generation)
    const searchResponse = page.waitForResponse(
      (response) => response.url().includes('/api/search/semantic') && response.status() === 200
    );
    await TestHelpers.performSearch(page, 'car');

    // AND: A spinner is visible inside the input while the search is in flight.
    // Asserted before awaiting the response: once the response resolves the
    // spinner can unmount almost immediately, so asserting afterwards races it.
    const spinner = page.locator('[data-testid="search-spinner"]');
    await expect(spinner).toBeVisible();
    await searchResponse;

    // THEN: The search button keeps its label (no collapse / layout shift)
    await expect(page.locator(TestHelpers.selectors.searchBtn)).toHaveText('Search');

    // AND: The spinner disappears once results are rendered
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(spinner).not.toBeVisible();
  });
});
