import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const CLUSTER_DAYS_AGO = 7;
const ARCHIVE_DAYS_AGO = 400;

test.describe('URL Routing', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
  });

  test.describe('View routing', () => {
    test('should default to "all" view on root URL', async ({ page }) => {
      // GIVEN: No specific view in URL
      // WHEN: User navigates to root
      await TestHelpers.goto(page);
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: "all" view is active and URL reflects it
      await TestHelpers.verifyActiveView(page, 'all');
    });

    for (const view of ['favorites', 'videos', 'collages', 'housekeeping']) {
      test.fixme(`should navigate directly to /${view}`, async ({ page }) => {
        // GIVEN: User enters a direct view URL
        // WHEN: Page loads
        await TestHelpers.goto(page, `/${view}`);

        // THEN: Correct view is active
        await TestHelpers.verifyActiveView(page, view);
        const state = TestHelpers.getUrlState(page);
        expect(state.view).toBe(view);
      });
    }

    test.fixme('should preserve view on page refresh', async ({ page }) => {
      // GIVEN: User is on the favorites view
      await TestHelpers.goto(page, '/favorites');
      await TestHelpers.verifyActiveView(page, 'favorites');

      // WHEN: User refreshes the page
      await page.reload({ waitUntil: 'domcontentloaded' });

      // THEN: View is still favorites
      await TestHelpers.verifyActiveView(page, 'favorites');
      const state = TestHelpers.getUrlState(page);
      expect(state.view).toBe('favorites');
    });

    test.fixme('should update URL when switching views via nav', async ({ page }) => {
      // GIVEN: User is on the homepage
      await TestHelpers.goto(page);
      await TestHelpers.waitForPhotosToLoad(page);

      // WHEN: User clicks favorites nav button
      await TestHelpers.navigateToView(page, 'favorites');

      // THEN: URL reflects the new view
      const state = TestHelpers.getUrlState(page);
      expect(state.view).toBe('favorites');
    });
  });

  test.describe('Search URL', () => {
    test.fixme('should write ?q= to URL on search', async ({ page }) => {
      // GIVEN: User is on the homepage
      await TestHelpers.goto(page);
      await TestHelpers.waitForSearchReady(page);

      // WHEN: User performs a search
      await TestHelpers.performSearch(page, 'car');

      // THEN: URL contains ?q=car
      await TestHelpers.waitForUrlParam(page, 'q', 'car');
      const state = TestHelpers.getUrlState(page);
      expect(state.query).toBe('car');
    });

    test.fixme('should restore search from ?q= on page load', async ({ page }) => {
      // GIVEN: URL contains a search query
      // WHEN: User navigates directly
      await TestHelpers.goto(page, '/?q=car');
      await TestHelpers.waitForSearchReady(page);

      // THEN: Search input is populated with the query
      const searchValue = await page.locator(TestHelpers.selectors.searchInput).inputValue();
      expect(searchValue).toBe('car');

      // AND: URL still has the param
      const state = TestHelpers.getUrlState(page);
      expect(state.query).toBe('car');
    });

    test.fixme('should remove ?q= when search is cleared', async ({ page }) => {
      // GIVEN: User has an active search
      await TestHelpers.goto(page, '/?q=car');
      await TestHelpers.waitForSearchReady(page);

      // WHEN: User clears the search
      await TestHelpers.clearSearch(page);

      // THEN: ?q= is removed from URL
      await TestHelpers.waitForUrlParam(page, 'q', null);
      const state = TestHelpers.getUrlState(page);
      expect(state.query).toBeNull();
    });
  });

  test.describe('Sort URL', () => {
    test.fixme('should write ?sort= to URL when sort changes', async ({ page }) => {
      // GIVEN: User is on the homepage
      await TestHelpers.goto(page);
      await TestHelpers.waitForPhotosToLoad(page);

      // WHEN: User changes sort order
      await page.selectOption(TestHelpers.selectors.sortSelect, 'oldest');

      // THEN: URL contains ?sort=oldest
      await TestHelpers.waitForUrlParam(page, 'sort', 'oldest');
      const state = TestHelpers.getUrlState(page);
      expect(state.sort).toBe('oldest');
    });

    test.fixme('should restore sort from ?sort= on page load', async ({ page }) => {
      // GIVEN: URL contains a sort param
      // WHEN: User navigates directly
      await TestHelpers.goto(page, '/?sort=oldest');
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: Sort select reflects the URL param
      const sortValue = await page.locator(TestHelpers.selectors.sortSelect).inputValue();
      expect(sortValue).toBe('oldest');

      // AND: URL still has the param
      const state = TestHelpers.getUrlState(page);
      expect(state.sort).toBe('oldest');
    });

    test.fixme('should remove ?sort= when reset to default', async ({ page }) => {
      // GIVEN: User has a custom sort active
      await TestHelpers.goto(page, '/?sort=oldest');
      await TestHelpers.waitForPhotosToLoad(page);

      // WHEN: User resets sort to default (newest)
      await page.selectOption(TestHelpers.selectors.sortSelect, 'newest');

      // THEN: ?sort= is removed (newest is default)
      await TestHelpers.waitForUrlParam(page, 'sort', null);
      const state = TestHelpers.getUrlState(page);
      expect(state.sort).toBeNull();
    });
  });

  test.describe('Timeline URL', () => {
    const recentDate = new Date(Date.now() - CLUSTER_DAYS_AGO * 24 * 60 * 60 * 1000);
    const archiveDate = new Date(Date.now() - ARCHIVE_DAYS_AGO * 24 * 60 * 60 * 1000);
    const recentYear = recentDate.getFullYear();
    const recentMonth = recentDate.getMonth() + 1;
    const archiveYear = archiveDate.getFullYear();
    const archiveMonth = archiveDate.getMonth() + 1;

    test.fixme('should write ?year= and ?month= on timeline selection', async ({ page }) => {
      // GIVEN: User is on the homepage with timeline visible
      await TestHelpers.goto(page);
      await TestHelpers.waitForPhotosToLoad(page);

      // WHEN: User selects a year/month on the timeline
      // (Simulate via URL since timeline interaction is complex)
      await TestHelpers.goto(page, `/?year=${archiveYear}&month=${archiveMonth}`);
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: URL contains year and month params
      const state = TestHelpers.getUrlState(page);
      expect(state.year).toBe(archiveYear);
      expect(state.month).toBe(archiveMonth);
    });

    test.fixme('should restore timeline from URL on page load', async ({ page }) => {
      // GIVEN: URL contains year and month params
      // WHEN: User navigates directly
      await TestHelpers.goto(page, `/?year=${recentYear}&month=${recentMonth}`);
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: Timeline state reflects the URL params
      const state = TestHelpers.getUrlState(page);
      expect(state.year).toBe(recentYear);
      expect(state.month).toBe(recentMonth);
    });

    test.fixme('should clear timeline params when reset', async ({ page }) => {
      // GIVEN: User has timeline filter active
      await TestHelpers.goto(page, `/?year=${recentYear}&month=${recentMonth}`);
      await TestHelpers.waitForPhotosToLoad(page);

      // WHEN: User clears the timeline filter (navigates to root)
      await TestHelpers.goto(page);
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: year and month params are removed
      const state = TestHelpers.getUrlState(page);
      expect(state.year).toBeNull();
      expect(state.month).toBeNull();
    });
  });

  test.describe('Back/Forward', () => {
    test.fixme('should create history entry when switching views', async ({ page }) => {
      // GIVEN: User starts on homepage
      await TestHelpers.goto(page);
      await TestHelpers.waitForPhotosToLoad(page);

      // WHEN: User navigates to favorites
      await TestHelpers.navigateToView(page, 'favorites');

      // AND: User presses Back
      await page.goBack();

      // THEN: URL returns to previous state
      const state = TestHelpers.getUrlState(page);
      expect(state.view).toBe('all');
    });

    test.fixme('should create history entry on search', async ({ page }) => {
      // GIVEN: User starts on homepage
      await TestHelpers.goto(page);
      await TestHelpers.waitForSearchReady(page);

      // WHEN: User searches for "car"
      await TestHelpers.performSearch(page, 'car');
      await TestHelpers.waitForUrlParam(page, 'q', 'car');

      // AND: User presses Back
      await page.goBack();

      // THEN: Search param is removed
      const state = TestHelpers.getUrlState(page);
      expect(state.query).toBeNull();
    });

    test.fixme('should support Forward after Back', async ({ page }) => {
      // GIVEN: User navigated and went back
      await TestHelpers.goto(page);
      await TestHelpers.waitForPhotosToLoad(page);
      await TestHelpers.navigateToView(page, 'favorites');
      await page.goBack();

      // WHEN: User presses Forward
      await page.goForward();

      // THEN: URL returns to favorites
      const state = TestHelpers.getUrlState(page);
      expect(state.view).toBe('favorites');
    });
  });

  test.describe('Param preservation', () => {
    test.fixme('should preserve ?q= when switching views', async ({ page }) => {
      // GIVEN: User has an active search
      await TestHelpers.goto(page, '/?q=car');
      await TestHelpers.waitForSearchReady(page);

      // WHEN: User switches to favorites view
      await TestHelpers.navigateToView(page, 'favorites');

      // THEN: Search param is preserved in URL
      const state = TestHelpers.getUrlState(page);
      expect(state.query).toBe('car');
      expect(state.view).toBe('favorites');
    });

    test.fixme('should preserve ?sort= when switching views', async ({ page }) => {
      // GIVEN: User has a custom sort
      await TestHelpers.goto(page, '/?sort=oldest');
      await TestHelpers.waitForPhotosToLoad(page);

      // WHEN: User switches to videos view
      await TestHelpers.navigateToView(page, 'videos');

      // THEN: Sort param is preserved in URL
      const state = TestHelpers.getUrlState(page);
      expect(state.sort).toBe('oldest');
      expect(state.view).toBe('videos');
    });

    test.fixme('should preserve timeline params when switching views', async ({ page }) => {
      const recentDate = new Date(Date.now() - CLUSTER_DAYS_AGO * 24 * 60 * 60 * 1000);
      const year = recentDate.getFullYear();
      const month = recentDate.getMonth() + 1;

      // GIVEN: User has timeline filter active
      await TestHelpers.goto(page, `/?year=${year}&month=${month}`);
      await TestHelpers.waitForPhotosToLoad(page);

      // WHEN: User switches to favorites view
      await TestHelpers.navigateToView(page, 'favorites');

      // THEN: Timeline params are preserved
      const state = TestHelpers.getUrlState(page);
      expect(state.year).toBe(year);
      expect(state.month).toBe(month);
      expect(state.view).toBe('favorites');
    });
  });

  test.describe('Deep linking', () => {
    test.fixme('should handle view + search combined URL', async ({ page }) => {
      // GIVEN: A deep link with view and search
      // WHEN: User navigates to combined URL
      await TestHelpers.goto(page, '/favorites?q=car');
      await TestHelpers.waitForSearchReady(page);

      // THEN: Both view and search are active
      await TestHelpers.verifyActiveView(page, 'favorites');
      const searchValue = await page.locator(TestHelpers.selectors.searchInput).inputValue();
      expect(searchValue).toBe('car');

      const state = TestHelpers.getUrlState(page);
      expect(state.view).toBe('favorites');
      expect(state.query).toBe('car');
    });

    test.fixme('should handle view + sort + search combined URL', async ({ page }) => {
      // GIVEN: A deep link with view, sort, and search
      // WHEN: User navigates to combined URL
      await TestHelpers.goto(page, '/videos?q=car&sort=oldest');
      await TestHelpers.waitForSearchReady(page);

      // THEN: All params are active
      await TestHelpers.verifyActiveView(page, 'videos');

      const state = TestHelpers.getUrlState(page);
      expect(state.view).toBe('videos');
      expect(state.query).toBe('car');
      expect(state.sort).toBe('oldest');
    });

    test.fixme('should handle full URL with all params', async ({ page }) => {
      const recentDate = new Date(Date.now() - CLUSTER_DAYS_AGO * 24 * 60 * 60 * 1000);
      const year = recentDate.getFullYear();
      const month = recentDate.getMonth() + 1;

      // GIVEN: A deep link with every param
      // WHEN: User navigates to full URL
      await TestHelpers.goto(page, `/favorites?q=car&sort=oldest&year=${year}&month=${month}`);
      await TestHelpers.waitForSearchReady(page);

      // THEN: All state is restored
      await TestHelpers.verifyActiveView(page, 'favorites');

      const state = TestHelpers.getUrlState(page);
      expect(state.view).toBe('favorites');
      expect(state.query).toBe('car');
      expect(state.sort).toBe('oldest');
      expect(state.year).toBe(year);
      expect(state.month).toBe(month);
    });
  });

  test.describe('Invalid params', () => {
    test.fixme('should fallback to "all" for invalid view path', async ({ page }) => {
      // GIVEN: URL has an invalid view
      // WHEN: User navigates to invalid path
      await TestHelpers.goto(page, '/nonexistent');

      // THEN: Falls back to "all" view
      await TestHelpers.verifyActiveView(page, 'all');
    });

    test.fixme('should ignore invalid ?year= value', async ({ page }) => {
      // GIVEN: URL has a non-numeric year
      // WHEN: User navigates with invalid param
      await TestHelpers.goto(page, '/?year=abc');
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: Year param is ignored, no crash
      const photos = await TestHelpers.getPhotoCards(page);
      expect(photos.length).toBeGreaterThan(0);
    });

    test.fixme('should ignore invalid ?month= value', async ({ page }) => {
      // GIVEN: URL has an out-of-range month
      // WHEN: User navigates with invalid param
      await TestHelpers.goto(page, '/?month=13');
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: Month param is ignored, no crash
      const photos = await TestHelpers.getPhotoCards(page);
      expect(photos.length).toBeGreaterThan(0);
    });

    test.fixme('should ignore invalid ?sort= value', async ({ page }) => {
      // GIVEN: URL has an unrecognized sort value
      // WHEN: User navigates with invalid param
      await TestHelpers.goto(page, '/?sort=random');
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: Sort falls back to default, no crash
      const photos = await TestHelpers.getPhotoCards(page);
      expect(photos.length).toBeGreaterThan(0);
    });
  });

  test.describe('Combined params', () => {
    test.fixme('should handle search + sort together', async ({ page }) => {
      // GIVEN: User is on the homepage
      await TestHelpers.goto(page);
      await TestHelpers.waitForSearchReady(page);

      // WHEN: User searches and sorts
      await TestHelpers.performSearch(page, 'car');
      await TestHelpers.waitForUrlParam(page, 'q', 'car');
      await page.selectOption(TestHelpers.selectors.sortSelect, 'oldest');
      await TestHelpers.waitForUrlParam(page, 'sort', 'oldest');

      // THEN: Both params are in URL
      const state = TestHelpers.getUrlState(page);
      expect(state.query).toBe('car');
      expect(state.sort).toBe('oldest');
    });

    test.fixme('should handle search + timeline together', async ({ page }) => {
      const recentDate = new Date(Date.now() - CLUSTER_DAYS_AGO * 24 * 60 * 60 * 1000);
      const year = recentDate.getFullYear();
      const month = recentDate.getMonth() + 1;

      // GIVEN: URL has both search and timeline
      // WHEN: User navigates with combined params
      await TestHelpers.goto(page, `/?q=car&year=${year}&month=${month}`);
      await TestHelpers.waitForSearchReady(page);

      // THEN: Both params are active
      const state = TestHelpers.getUrlState(page);
      expect(state.query).toBe('car');
      expect(state.year).toBe(year);
      expect(state.month).toBe(month);
    });

    test.fixme('should handle sort + timeline together', async ({ page }) => {
      const archiveDate = new Date(Date.now() - ARCHIVE_DAYS_AGO * 24 * 60 * 60 * 1000);
      const year = archiveDate.getFullYear();
      const month = archiveDate.getMonth() + 1;

      // GIVEN: URL has both sort and timeline
      // WHEN: User navigates with combined params
      await TestHelpers.goto(page, `/?sort=oldest&year=${year}&month=${month}`);
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: Both params are active
      const state = TestHelpers.getUrlState(page);
      expect(state.sort).toBe('oldest');
      expect(state.year).toBe(year);
      expect(state.month).toBe(month);
    });

    test.fixme('should clear one param without affecting others', async ({ page }) => {
      // GIVEN: Multiple params are active
      await TestHelpers.goto(page, '/?q=car&sort=oldest');
      await TestHelpers.waitForSearchReady(page);

      // WHEN: User clears search
      await TestHelpers.clearSearch(page);
      await TestHelpers.waitForUrlParam(page, 'q', null);

      // THEN: Sort param is still present
      const state = TestHelpers.getUrlState(page);
      expect(state.query).toBeNull();
      expect(state.sort).toBe('oldest');
    });
  });
});
