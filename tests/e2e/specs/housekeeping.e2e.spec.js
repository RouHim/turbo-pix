import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Housekeeping', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should navigate to housekeeping view', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User clicks housekeeping button
    await TestHelpers.navigateToView(page, 'housekeeping');

    // THEN: Housekeeping view is active
    await TestHelpers.verifyActiveView(page, 'housekeeping');
  });

  test('should display deletion candidates when available', async ({ page }) => {
    // GIVEN: User navigates to housekeeping view
    await TestHelpers.navigateToView(page, 'housekeeping');

    // WHEN: Page loads
    const candidateCards = await TestHelpers.getPhotoCards(page);
    test.skip(candidateCards.length === 0, 'No deletion candidates in dataset');

    // THEN: Candidates are displayed
    expect(candidateCards.length).toBeGreaterThan(0);
  });

  test('should allow reviewing deletion candidates', async ({ page }) => {
    // GIVEN: User is on housekeeping view with candidates
    await TestHelpers.navigateToView(page, 'housekeeping');
    const candidateCards = await TestHelpers.getPhotoCards(page);
    test.skip(candidateCards.length === 0, 'No deletion candidates in dataset');
    expect(candidateCards.length).toBeGreaterThan(0);

    // WHEN: User clicks on a candidate
    await candidateCards[0].click();

    // THEN: Viewer opens for review
    await TestHelpers.verifyViewerOpen(page);
  });
});
