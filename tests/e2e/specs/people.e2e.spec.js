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

async function mockPeople(page, initial) {
  let people = initial;
  await page.route('**/api/people**', async (route) => {
    const request = route.request();
    if (request.method() === 'PATCH') {
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
    await input.fill('Anna Schmidt');
    await page.getByRole('button', { name: 'Save' }).click();

    // THEN the card shows the new name
    await expect(page.locator('[data-testid="person-card"]').first()).toContainText('Anna Schmidt');
  });

  test('hides a person and can show hidden people again', async ({ page }) => {
    // GIVEN one visible person
    await mockPeople(page, [person(3, 'Anna')]);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    await expect(page.locator('[data-testid="person-card"]')).toHaveCount(1);

    // WHEN the user hides the person
    await page.getByRole('button', { name: 'Hide' }).click();

    // THEN the default list is empty
    await expect(page.locator('[data-testid="person-card"]')).toHaveCount(0);

    // AND enabling hidden people reveals the card with its badge
    await page.getByLabel('Show hidden people').check();
    await expect(page.locator('[data-testid="person-card"]')).toHaveCount(1);
    await expect(page.locator('[data-testid="person-card"]')).toContainText('Hidden');
  });

  test('shows the one-time non-commercial model notice', async ({ page }) => {
    await mockPeople(page, [person(3, 'Anna')]);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');

    const notice = page.locator('.people-notice');
    await expect(notice).toContainText('non-commercial');
    await notice.getByRole('button', { name: 'Dismiss' }).click();
    await expect(notice).toHaveCount(0);
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
