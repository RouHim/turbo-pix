import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const CONFIG_ENABLED = {
  default_locale: 'en',
  tile_url: null,
  face_recognition_enabled: true,
};

const CONFIG_DISABLED = { ...CONFIG_ENABLED, face_recognition_enabled: false };

function person(id, name, faceCount = 3, hidden = false) {
  return {
    id,
    name,
    hidden,
    face_count: faceCount,
    cover_face_id: null,
  };
}

async function mockConfig(page, config) {
  await page.route('**/api/config', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(config),
    });
  });
}

async function mockPeople(page, initial, { delayMs = 0, failPatch = false } = {}) {
  let people = initial;
  await page.route('**/api/people**', async (route) => {
    const request = route.request();
    if (request.method() === 'PATCH') {
      if (failPatch) {
        await route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'rename failed' }),
        });
        return;
      }
      const id = Number(request.url().match(/\/api\/people\/(\d+)/)?.[1]);
      const body = request.postDataJSON();
      people = people.map((item) => (item.id === id ? { ...item, ...body } : item));
      const updated = people.find((item) => item.id === id);
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ person: updated }),
      });
      return;
    }

    if (delayMs) await new Promise((resolve) => setTimeout(resolve, delayMs));
    const includeHidden = request.url().includes('include_hidden=true');
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        people: includeHidden ? people : people.filter((item) => !item.hidden),
      }),
    });
  });
}

async function mockEmptyGrid(page) {
  await page.route('**/api/photos?**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ photos: [], total: 0 }),
    });
  });
  await page.route('**/api/photos', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ photos: [], total: 0 }),
    });
  });
}

test.describe('People view', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await mockConfig(page, CONFIG_ENABLED);
    await mockEmptyGrid(page);
  });

  test('lists people, filters the grid by person and renames a cluster', async ({ page }) => {
    // GIVEN two visible people clusters
    await mockPeople(page, [person(3, 'Anna'), person(8, 'Ben')]);

    // WHEN the user opens the People view
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');

    // THEN both cards render
    const cards = page.locator('[data-testid="person-card"]');
    await expect(cards).toHaveCount(2);
    await expect(cards.first()).toContainText('Anna');

    // WHEN the user clicks Anna's card
    await cards.first().click();

    // THEN the URL scopes the grid to that person and the request carries it
    await expect(page).toHaveURL(/\/people\?person=3$/);
    await expect(page.locator('#current-view-title')).toHaveText('Anna');
    await expect.poll(() => page.url()).toContain('person=3');

    // WHEN the user goes back to the People list and renames Anna
    await TestHelpers.navigateToView(page, 'people');
    await page.getByRole('button', { name: 'Rename' }).first().click();
    const input = page.locator('#person-name-input');
    await expect(input).toBeFocused();
    await input.fill('Anna Schmidt');
    await page.getByRole('button', { name: 'Save' }).click();

    // THEN the card shows the new name
    await expect(page.locator('[data-testid="person-card"]').first()).toContainText('Anna Schmidt');
  });

  test('keeps the rename dialog open and preserves input when saving fails', async ({ page }) => {
    await mockPeople(page, [person(3, 'Anna')], { failPatch: true });
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');

    await page.getByRole('button', { name: 'Rename' }).click();
    const dialog = page.locator('dialog.person-dialog');
    const input = page.locator('#person-name-input');
    await input.fill('Anna Schmidt');
    await page.getByRole('button', { name: 'Save' }).click();

    // THEN the dialog stays open, keeps the typed value, and shows the error inline
    await expect(dialog).toBeVisible();
    await expect(input).toHaveValue('Anna Schmidt');
    await expect(dialog.getByRole('alert')).toContainText('Could not rename this person');

    // AND Escape closes it and returns focus to the invoking Rename button
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden();
    await expect(page.getByRole('button', { name: 'Rename' })).toBeFocused();
  });

  test('shows a skeleton instead of the empty state while people load', async ({ page }) => {
    await mockPeople(page, [person(3, 'Anna')], { delayMs: 1200 });
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');

    await expect(page.getByTestId('people-skeleton')).toBeVisible();
    await expect(
      page.getByText('No people yet. Face indexing runs in the background.')
    ).toHaveCount(0);
    await expect(page.locator('[data-testid="person-card"]')).toHaveCount(1);
  });

  test('hides a person without dropping the card or its toast action', async ({ page }) => {
    // GIVEN one visible person
    await mockPeople(page, [person(3, 'Anna')]);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    const cards = page.locator('[data-testid="person-card"]');
    const hiddenToggle = page.getByTestId('people-hidden-toggle');
    await expect(cards).toHaveCount(1);
    await expect(hiddenToggle).toHaveAttribute('aria-pressed', 'false');

    // WHEN the user hides the person
    await page.getByRole('button', { name: 'Hide' }).click();

    // THEN the card stays in place with a visible hidden state and a recovery action
    await expect(cards).toHaveCount(1);
    await expect(cards).toContainText('Hidden');
    const toastAction = page.locator('.toast-action');
    await expect(toastAction).toHaveText('Show hidden');

    // AND the toast action enables the hidden filter
    await toastAction.click();
    await expect(hiddenToggle).toHaveAttribute('aria-pressed', 'true');
    await expect(cards).toHaveCount(1);

    // AND switching the filter off uses server truth and removes the hidden card
    await hiddenToggle.click();
    await expect(cards).toHaveCount(0);
  });

  test('shows the one-time non-commercial model notice', async ({ page }) => {
    await mockPeople(page, [person(3, 'Anna')]);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');

    const notice = page.locator('.people-notice');
    await expect(notice).toContainText('non-commercial');
    await notice.getByRole('button', { name: 'Dismiss' }).click();
    await expect(notice).toHaveCount(0);
    await expect(page.getByTestId('people-hidden-toggle')).toBeFocused();
  });

  test('hides the People nav and redirects a deep link when disabled', async ({ page }) => {
    // GIVEN the backend reports face recognition disabled
    await page.unroute('**/api/config');
    await mockConfig(page, CONFIG_DISABLED);

    // WHEN the user opens /people directly
    await TestHelpers.goto(page, '/people');

    // THEN the app lands on All Photos and the nav item is absent
    await expect(page).toHaveURL(/\/$/);
    await expect(page.locator('[data-view="people"]')).toHaveCount(0);
  });
});
