import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Video Play Icon Indicator', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('GIVEN videos view WHEN video cards load THEN every video card has a visible play icon', async ({
    page,
  }) => {
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);

    const videoCards = await TestHelpers.getPhotoCards(page);
    expect(videoCards.length).toBeGreaterThan(0);

    for (const card of videoCards) {
      const playIcon = card.locator('.video-play-icon');
      await expect(playIcon).toHaveCount(1);
      await expect(playIcon).toBeVisible();

      const iconSvg = playIcon.locator('svg');
      await expect(iconSvg).toHaveCount(1);
      await expect(iconSvg).toBeVisible();
    }
  });

  test('GIVEN all photos view WHEN photo cards are displayed THEN photo cards do NOT have a play icon', async ({
    page,
  }) => {
    const allCards = await TestHelpers.getPhotoCards(page);
    expect(allCards.length).toBeGreaterThan(0);

    let photoCardCount = 0;
    for (const card of allCards) {
      const playIconCount = await card.locator('.video-play-icon').count();
      if (playIconCount === 0) {
        photoCardCount += 1;
      }
    }

    expect(photoCardCount).toBeGreaterThan(0);
  });
});
