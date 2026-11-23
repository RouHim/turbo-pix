import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const firstCard = (page) => page.locator(TestHelpers.selectors.photoCardAny).first();
const cardOverlay = (page) => firstCard(page).locator('[data-card-overlay]');
const cardActions = (page) => firstCard(page).locator('[data-card-actions]');
const viewerDate = (page) => page.locator('[data-viewer-date]');
const detailsSheet = (page) => page.getByTestId('viewer-sidebar');

async function cardIsSquare(card) {
  const box = await card.boundingBox();
  expect(box).not.toBeNull();
  return Math.abs(box.width - box.height) <= 1;
}

async function setFavorite(page, hash, isFavorite) {
  const response = await page.request.put(`/api/photos/${hash}/favorite`, {
    data: { is_favorite: isFavorite },
  });
  expect(response.ok()).toBeTruthy();
}

async function favoriteFirstCard(page) {
  const hash = await firstCard(page).getAttribute('data-photo-id');
  expect(hash).not.toBeNull();
  await setFavorite(page, hash, true);
  await page.reload({ waitUntil: 'domcontentloaded' });
  await TestHelpers.waitForPhotosToLoad(page);
  return hash;
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

  test('favorited phone tile carries the brass favorite mark', async ({ page }) => {
    await TestHelpers.setMobileViewport(page);
    const hash = await favoriteFirstCard(page);

    try {
      const card = page.locator(`[data-photo-id="${hash}"]`);
      await expect(card.locator('[data-favorite-mark]')).toBeVisible();
      await expect(card.locator('.photo-card-open-layer')).toHaveAttribute(
        'aria-label',
        /Favorite/
      );
    } finally {
      await setFavorite(page, hash, false);
    }
  });

  test('viewer shows the capture date without opening the details sheet', async ({ page }) => {
    await TestHelpers.setMobileViewport(page);
    await expect(detailsSheet(page)).not.toHaveClass(/show/);

    await firstCard(page).click();
    await TestHelpers.verifyViewerOpen(page);

    await expect(viewerDate(page)).toBeVisible();
    const captionText = (await viewerDate(page).textContent()).trim();
    expect(captionText).not.toBe('');

    await page.locator('.metadata-btn').click();
    await expect(detailsSheet(page)).toHaveClass(/show/);
    await expect(page.locator('#photo-date')).toHaveText(captionText);
    await expect(viewerDate(page)).toBeHidden();

    await page.keyboard.press('Escape');
  });

  test('desktop reveals the date and actions on hover and focus', async ({ page }) => {
    await expect(cardOverlay(page)).toHaveCSS('opacity', '0');
    await expect(cardActions(page)).toHaveCSS('opacity', '0');
    await expect(firstCard(page).locator('[data-favorite-mark]')).toBeHidden();

    await firstCard(page).locator('.favorite-btn').focus();
    await expect(cardOverlay(page)).toHaveCSS('opacity', '1');
    await expect(cardActions(page)).toHaveCSS('opacity', '1');

    await page.locator('#search-input').focus();
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

  test('coarse-pointer tablet marks a favorited tile', async ({ page }) => {
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    const hash = await favoriteFirstCard(page);

    try {
      const card = page.locator(`[data-photo-id="${hash}"]`);
      await expect(card.locator('[data-favorite-mark]')).toBeVisible();
    } finally {
      await setFavorite(page, hash, false);
    }
  });
});
