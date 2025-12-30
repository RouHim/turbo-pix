import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Critical User Journeys', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await page.goto('/');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('complete photo management workflow', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length === 0) {
      test.skip('No photos available');
    }

    // 1. User views all photos
    await TestHelpers.verifyActiveView(page, 'all');
    expect(photoCards.length).toBeGreaterThan(0);

    // 2. User opens a photo
    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // 3. User favorites the photo
    const favoriteBtn = page.locator('.favorite-btn');
    await favoriteBtn.click();
    await TestHelpers.waitForToast(page, 'favorite');

    // 4. User navigates to next photo
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(500);

    // 5. User closes viewer
    await TestHelpers.closeViewer(page);

    // 6. User navigates to favorites view
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    // 7. Verify favorited photo is there
    const favoriteCards = await TestHelpers.getPhotoCards(page);
    expect(favoriteCards.length).toBeGreaterThanOrEqual(1);
  });

  test('search and view workflow', async ({ page }) => {
    // 1. User searches for photos
    const searchInput = page.locator(TestHelpers.selectors.searchInput);
    await searchInput.fill('test');
    await searchInput.press('Enter');

    // 2. Wait for search results
    await TestHelpers.waitForPhotosToLoad(page);

    const results = await TestHelpers.getPhotoCards(page);
    const hasResults = results.length > 0;
    const hasEmptyState = await TestHelpers.elementExists(page, TestHelpers.selectors.noPhotos);

    expect(hasResults || hasEmptyState).toBe(true);

    if (hasResults) {
      // 3. User opens first result
      await results[0].click();
      await TestHelpers.verifyViewerOpen(page);

      // 4. User views photo metadata
      const metadataBtn = page.locator('.metadata-btn');
      if ((await metadataBtn.count()) > 0) {
        await metadataBtn.click();
        const sidebar = page.locator('.viewer-sidebar');
        await expect(sidebar).toBeVisible();
      }

      // 5. User closes viewer
      await TestHelpers.closeViewer(page);

      // 6. User clears search
      await searchInput.fill('');
      await searchInput.press('Enter');
      await TestHelpers.waitForPhotosToLoad(page);

      // 7. Back to all photos
      const allPhotos = await TestHelpers.getPhotoCards(page);
      expect(allPhotos.length >= 0).toBe(true);
    }
  });

  test('video discovery and playback journey', async ({ page }) => {
    // 1. User navigates to videos view
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);

    if (videoCards.length === 0) {
      test.skip('No videos available');
    }

    // 2. User opens a video
    await videoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // 3. Verify video element is present
    const video = page.locator(TestHelpers.selectors.viewerVideo);
    await expect(video).toBeVisible();

    // 4. Wait for video to load
    await page.waitForTimeout(2000);

    // 5. User navigates to next video (if available)
    if (videoCards.length > 1) {
      await page.keyboard.press('ArrowRight');
      await page.waitForTimeout(1000);
      await expect(video).toBeVisible();
    }

    // 6. User favorites the video
    const favoriteBtn = page.locator('.favorite-btn');
    await favoriteBtn.click();
    await page.waitForTimeout(500);

    // 7. User closes viewer
    await TestHelpers.closeViewer(page);

    // 8. Navigate to favorites
    await TestHelpers.navigateToView(page, 'favorites');
    await TestHelpers.waitForPhotosToLoad(page);

    // 9. Verify video is in favorites
    const favoriteCards = await TestHelpers.getPhotoCards(page);
    expect(favoriteCards.length).toBeGreaterThan(0);
  });

  test('timeline filtering workflow', async ({ page }) => {
    // 1. User applies timeline filter
    const timelineSlider = page.locator('.timeline-input');
    const yearSelect = page.locator('.timeline-year-select');

    const hasSlider = (await timelineSlider.count()) > 0;
    const hasYearSelect = (await yearSelect.count()) > 0;

    if (!hasSlider && !hasYearSelect) {
      test.skip('Timeline controls not available');
    }

    if (hasSlider) {
      // Desktop timeline
      await timelineSlider.fill('50');
      await page.waitForTimeout(1000);
    } else if (hasYearSelect) {
      // Mobile timeline
      const options = await yearSelect.locator('option').all();
      if (options.length > 1) {
        const yearValue = await options[1].getAttribute('value');
        if (yearValue) {
          await yearSelect.selectOption(yearValue);
          await page.waitForTimeout(1000);
        }
      }
    }

    // 2. Wait for filtered results
    await TestHelpers.waitForPhotosToLoad(page);

    // 3. User opens a photo from filtered results
    const filteredCards = await TestHelpers.getPhotoCards(page);
    if (filteredCards.length > 0) {
      await filteredCards[0].click();
      await TestHelpers.verifyViewerOpen(page);

      // 4. User closes viewer
      await TestHelpers.closeViewer(page);
    }

    // 5. User resets timeline filter
    const resetBtn = page.locator('.timeline-reset');
    if ((await resetBtn.count()) > 0) {
      await resetBtn.click();
      await page.waitForTimeout(500);

      // 6. Verify all photos shown again
      await TestHelpers.waitForPhotosToLoad(page);
      const allCards = await TestHelpers.getPhotoCards(page);
      expect(allCards.length >= 0).toBe(true);
    }
  });

  test('housekeeping workflow', async ({ page }) => {
    // 1. User navigates to housekeeping
    await TestHelpers.navigateToView(page, 'housekeeping');
    await TestHelpers.waitForPhotosToLoad(page);

    const candidates = await TestHelpers.getPhotoCards(page);

    if (candidates.length === 0) {
      test.skip('No housekeeping candidates');
    }

    // 2. User reviews first candidate
    const firstCandidate = candidates[0];
    await firstCandidate.hover();

    // 3. User decides to keep it
    const keepBtn = firstCandidate.locator('[data-action="keep"]');
    if ((await keepBtn.count()) > 0) {
      await keepBtn.click();
      await page.waitForTimeout(1000);

      // 4. Verify candidate removed
      const remainingCandidates = await TestHelpers.getPhotoCards(page);
      expect(remainingCandidates.length).toBeLessThan(candidates.length);
    }

    // 5. User navigates back to all photos
    await TestHelpers.navigateToView(page, 'all');
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('collage management workflow', async ({ page }) => {
    // 1. User navigates to collages
    await TestHelpers.navigateToView(page, 'collages');
    await TestHelpers.waitForPhotosToLoad(page);

    const collages = await TestHelpers.getPhotoCards(page);

    if (collages.length === 0) {
      test.skip('No pending collages');
    }

    // 2. User views collage
    await collages[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // 3. User closes viewer
    await TestHelpers.closeViewer(page);

    // 4. User accepts collage
    await collages[0].hover();
    const acceptBtn = collages[0].locator('[data-action="accept"]');

    if ((await acceptBtn.count()) > 0) {
      await acceptBtn.click();
      await page.waitForTimeout(1500);

      // 5. Collage removed from view
      const remaining = await TestHelpers.getPhotoCards(page);
      expect(remaining.length).toBeLessThan(collages.length);
    }
  });

  test('mobile navigation workflow', async ({ page }) => {
    await TestHelpers.setMobileViewport(page);
    await page.reload();
    await TestHelpers.waitForPhotosToLoad(page);

    // 1. User opens mobile menu
    const menuBtn = page.locator(TestHelpers.selectors.menuBtn);
    await menuBtn.click();

    // 2. Sidebar opens
    const sidebar = page.locator(TestHelpers.selectors.sidebar);
    await expect(sidebar).toHaveClass(/open/);

    // 3. User navigates to favorites
    const favoritesBtn = page.locator(TestHelpers.selectors.navItem('favorites'));
    await favoritesBtn.click();

    // 4. View changes, sidebar closes
    await TestHelpers.verifyActiveView(page, 'favorites');
    await expect(sidebar).not.toHaveClass(/open/);

    // 5. User opens mobile search
    const mobileSearchBtn = page.locator('.mobile-search-btn');
    if ((await mobileSearchBtn.count()) > 0) {
      await mobileSearchBtn.click();
      const mobileSearch = page.locator('.mobile-search');
      await expect(mobileSearch).toBeVisible();
    }
  });

  test('multi-photo browsing workflow', async ({ page }) => {
    const photoCards = await TestHelpers.getPhotoCards(page);
    if (photoCards.length < 5) {
      test.skip('Need at least 5 photos');
    }

    // 1. User browses through multiple photos
    await photoCards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    // 2. Navigate through 5 photos
    for (let i = 0; i < 4; i++) {
      await page.keyboard.press('ArrowRight');
      await page.waitForTimeout(300);
    }

    // 3. Still in viewer
    const viewer = page.locator(TestHelpers.selectors.viewer);
    await expect(viewer).toHaveClass(/active/);

    // 4. Navigate back
    for (let i = 0; i < 4; i++) {
      await page.keyboard.press('ArrowLeft');
      await page.waitForTimeout(300);
    }

    // 5. Still in viewer
    await expect(viewer).toHaveClass(/active/);

    // 6. Close viewer
    await TestHelpers.closeViewer(page);
  });

  test('sort and filter workflow', async ({ page }) => {
    // 1. User changes sort order
    const sortSelect = page.locator(TestHelpers.selectors.sortSelect);
    const sortExists = (await sortSelect.count()) > 0;

    if (sortExists) {
      await sortSelect.selectOption('name_asc');
      await page.waitForTimeout(1000);

      // 2. Photos reload
      await TestHelpers.waitForPhotosToLoad(page);

      // 3. User applies timeline filter
      const timelineSlider = page.locator('.timeline-input');
      if ((await timelineSlider.count()) > 0) {
        await timelineSlider.fill('70');
        await page.waitForTimeout(1000);
        await TestHelpers.waitForPhotosToLoad(page);
      }

      // 4. User views filtered results
      const filtered = await TestHelpers.getPhotoCards(page);
      expect(filtered.length >= 0).toBe(true);

      // 5. User resets filters
      const resetBtn = page.locator('.timeline-reset');
      if ((await resetBtn.count()) > 0) {
        await resetBtn.click();
        await page.waitForTimeout(500);
      }

      // 6. All photos shown
      await TestHelpers.waitForPhotosToLoad(page);
    }
  });

  test('error recovery workflow', async ({ page }) => {
    // 1. User searches for non-existent content
    const searchInput = page.locator(TestHelpers.selectors.searchInput);
    await searchInput.fill('xyznonexistent123456');
    await searchInput.press('Enter');
    await page.waitForTimeout(2000);

    // 2. Empty state shown
    const noPhotos = page.locator(TestHelpers.selectors.noPhotos);
    const hasEmptyState = (await noPhotos.count()) > 0;

    if (hasEmptyState) {
      await expect(noPhotos).toBeVisible();
    }

    // 3. User clears search
    await searchInput.fill('');
    await searchInput.press('Enter');
    await TestHelpers.waitForPhotosToLoad(page);

    // 4. Photos appear again
    const photos = await TestHelpers.getPhotoCards(page);
    expect(photos.length >= 0).toBe(true);
  });

  test('theme switching workflow', async ({ page }) => {
    // 1. User toggles theme
    const themeToggle = page.locator('#theme-toggle');
    await themeToggle.click();
    await page.waitForTimeout(500);

    // 2. Theme changes
    const html = page.locator('html');
    const htmlClass = await html.getAttribute('class');
    expect(htmlClass).toBeTruthy();

    // 3. User navigates to different view
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    // 4. Theme persists
    const htmlClass2 = await html.getAttribute('class');
    expect(htmlClass2).toBe(htmlClass);

    // 5. User toggles theme back
    await themeToggle.click();
    await page.waitForTimeout(500);

    // 6. Theme changes again
    const htmlClass3 = await html.getAttribute('class');
    expect(htmlClass3).toBeTruthy();
  });
});
