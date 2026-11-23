import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const firstCard = (page) => page.locator(TestHelpers.selectors.photoCardAny).first();
const cardOverlay = (page) => firstCard(page).locator('[data-card-overlay]');
const cardActions = (page) => firstCard(page).locator('[data-card-actions]');

async function cardIsSquare(card) {
  const box = await card.boundingBox();
  expect(box).not.toBeNull();
  return Math.abs(box.width - box.height) <= 1;
}

test.describe('mobile card surface', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('phone grid keeps only the preview tile', async ({ page }) => {
    await TestHelpers.setMobileViewport(page);

    await expect(cardOverlay(page)).toBeHidden();
    await expect(cardActions(page)).toBeHidden();
    await expect.poll(() => cardIsSquare(firstCard(page))).toBe(true);
  });

  test('desktop still reveals the date and actions on hover', async ({ page }) => {
    await expect(cardOverlay(page)).toHaveCSS('opacity', '0');
    await expect(cardActions(page)).toHaveCSS('opacity', '0');

    await firstCard(page).hover();

    await expect(cardOverlay(page)).toHaveCSS('opacity', '1');
    await expect(cardActions(page)).toHaveCSS('opacity', '1');
  });
});

test.describe('touch tablet card surface', () => {
  test.use({ hasTouch: true, viewport: { width: 1440, height: 900 } });

  test('coarse-pointer tablet keeps only the preview tile', async ({ page }) => {
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);

    await expect(cardOverlay(page)).toBeHidden();
    await expect(cardActions(page)).toBeHidden();
    await expect.poll(() => cardIsSquare(firstCard(page))).toBe(true);
  });
});
