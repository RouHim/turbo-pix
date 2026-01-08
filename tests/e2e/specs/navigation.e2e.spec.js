import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Navigation', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('should load homepage successfully', async ({ page }) => {
    // GIVEN: User navigates to the homepage
    // WHEN: Page loads
    // THEN: Photos are displayed
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);

    // AND: All view button is active
    await TestHelpers.verifyActiveView(page, 'all');
  });

  test('should navigate to favorites view', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User clicks the favorites button
    await TestHelpers.navigateToView(page, 'favorites');

    // THEN: Favorites view is active
    await TestHelpers.verifyActiveView(page, 'favorites');
  });

  test('should navigate to videos view', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User clicks the videos button
    await TestHelpers.navigateToView(page, 'videos');

    // THEN: Videos view is active
    await TestHelpers.verifyActiveView(page, 'videos');
  });

  test('should navigate to collages view', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User clicks the collages button
    await TestHelpers.navigateToView(page, 'collages');

    // THEN: Collages view is active
    await TestHelpers.verifyActiveView(page, 'collages');
  });

  test('should navigate to housekeeping view', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User clicks the housekeeping button
    await TestHelpers.navigateToView(page, 'housekeeping');

    // THEN: Housekeeping view is active
    await TestHelpers.verifyActiveView(page, 'housekeeping');
  });

  test('should navigate between multiple views', async ({ page }) => {
    // GIVEN: User is on the homepage
    // WHEN: User navigates through different views
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.verifyActiveView(page, 'favorites');

    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.verifyActiveView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    await TestHelpers.navigateToView(page, 'all');
    await TestHelpers.verifyActiveView(page, 'all');
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: Each view transition is successful
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length).toBeGreaterThan(0);
  });
});
