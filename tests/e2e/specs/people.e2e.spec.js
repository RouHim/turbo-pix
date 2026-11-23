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

async function mockPeople(
  page,
  initial,
  { delayMs = 0, failPatch = false, failHide = false, dropOnMerge = null, staleUndo = false } = {}
) {
  let people = initial;
  let lastMerge = null;
  let mergeSnapshot = null;
  let nextUndoId = 1;
  await page.route('**/api/people**', async (route) => {
    const request = route.request();
    const url = request.url();

    if (request.method() === 'POST' && url.endsWith('/api/people/merge/undo')) {
      const body = request.postDataJSON();
      if (staleUndo || !lastMerge || lastMerge.undo_id !== body.undo_id) {
        await route.fulfill({
          status: 409,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'merge_slot_stale' }),
        });
        return;
      }
      const moved = mergeSnapshot.sources.reduce((sum, item) => sum + (item.face_count || 0), 0);
      const restoredPeople = mergeSnapshot.sources.length;
      people = people
        .map((item) =>
          item.id === mergeSnapshot.keeperId
            ? { ...item, face_count: item.face_count - moved }
            : item
        )
        .concat(mergeSnapshot.sources);
      lastMerge = null;
      mergeSnapshot = null;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          restored_faces: moved,
          total_faces: moved,
          restored_people: restoredPeople,
        }),
      });
      return;
    }

    if (request.method() === 'POST' && url.endsWith('/api/people/merge')) {
      const body = request.postDataJSON();
      if (dropOnMerge != null && body.source_ids.includes(dropOnMerge)) {
        people = people.filter((item) => item.id !== dropOnMerge);
        await route.fulfill({
          status: 404,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'Not Found' }),
        });
        return;
      }
      const keeper = people.find((item) => item.id === body.target_id);
      const sources = people.filter((item) => body.source_ids.includes(item.id));
      const moved = sources.reduce((sum, item) => sum + (item.face_count || 0), 0);
      const keeperName = body.name ?? keeper?.name ?? null;
      people = people
        .filter((item) => !body.source_ids.includes(item.id))
        .map((item) =>
          item.id === body.target_id
            ? { ...item, name: keeperName, face_count: (item.face_count || 0) + moved }
            : item
        );
      mergeSnapshot = { keeperId: body.target_id, sources };
      lastMerge = {
        undo_id: nextUndoId++,
        target_id: body.target_id,
        target_name: keeperName,
        source_count: sources.length,
      };
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          moved_faces: moved,
          removed_people: sources.length,
          undo_id: lastMerge.undo_id,
        }),
      });
      return;
    }

    if (request.method() === 'POST' && url.endsWith('/api/people/batch/hide')) {
      if (failHide) {
        await route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'hide failed' }),
        });
        return;
      }
      const body = request.postDataJSON();
      people = people.map((item) =>
        body.ids.includes(item.id) ? { ...item, hidden: body.hidden } : item
      );
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ updated: body.ids.length }),
      });
      return;
    }

    if (request.method() === 'PATCH') {
      if (failPatch) {
        await route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'rename failed' }),
        });
        return;
      }
      const id = Number(url.match(/\/api\/people\/(\d+)/)?.[1]);
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
    const includeHidden = url.includes('include_hidden=true');
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        people: includeHidden ? people : people.filter((item) => !item.hidden),
        last_merge: lastMerge,
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
    const dialog = page.getByTestId('people-rename-dialog');
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

  test('selects two persons and merges them into the keeper with the summed faces', async ({
    page,
  }) => {
    // GIVEN two clusters of the same person
    await mockPeople(page, [person(3, 'Anna', 40), person(8, 'Ben', 6)]);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    const cards = page.locator('[data-testid="person-card"]');
    const selectMode = page.locator('[data-action="select-mode"]');
    await expect(cards).toHaveCount(2);

    // WHEN the user enters selection mode and picks both cards
    await selectMode.click();
    await expect(selectMode).toHaveAttribute('aria-pressed', 'true');
    await cards.nth(0).locator('[data-testid="person-open"]').click();
    // Merge needs a keeper and at least one source.
    await expect(page.locator('[data-action="batch-merge-people"]')).toBeDisabled();
    await cards.nth(1).locator('[data-testid="person-open"]').click();

    // THEN the cards toggle instead of opening the grid
    await expect(cards.nth(0).locator('[data-testid="person-open"]')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    await expect(page).toHaveURL(/\/people$/);

    // WHEN the user merges
    await page.locator('[data-action="batch-merge-people"]').click();
    const dialog = page.getByTestId('people-merge-dialog');
    await expect(dialog).toBeVisible();
    const keeperRadio = dialog.getByTestId('merge-keeper').nth(0);
    await expect(keeperRadio).toBeChecked();
    // The dialog's text-input rule must not stretch the keeper radios, and
    // the merge dialog keeps its own (wider) width.
    expect((await keeperRadio.boundingBox()).width).toBeLessThan(24);
    expect((await dialog.boundingBox()).width).toBeGreaterThan(430);
    await expect(dialog.getByTestId('merge-name-input')).toHaveValue('Anna');
    await expect(dialog).toContainText('46 faces from 2 persons');
    await dialog.getByTestId('merge-confirm').click();

    // THEN one keeper remains, the toast offers undo and the chip persists it
    await expect(cards).toHaveCount(1);
    await expect(cards.first()).toContainText('Anna');
    await expect(cards.first()).toContainText('46 faces');
    await expect(page.locator('.toast-action')).toHaveText('Undo merge');
    await expect(page.getByTestId('people-undo-chip')).toBeVisible();
  });

  test('undoes the last merge from the toolbar chip', async ({ page }) => {
    await mockPeople(page, [person(3, 'Anna', 40), person(8, 'Ben', 6)]);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    const cards = page.locator('[data-testid="person-card"]');

    await page.locator('[data-action="select-mode"]').click();
    await page.locator('[data-action="batch-select-all"]').click();
    await page.locator('[data-action="batch-merge-people"]').click();
    await page.getByTestId('merge-confirm').click();
    await expect(cards).toHaveCount(1);

    // WHEN the user undoes the merge from the chip
    await page.getByTestId('people-undo-chip').click();

    // THEN both persons return and the slot is gone
    await expect(cards).toHaveCount(2);
    await expect(page.getByTestId('people-undo-chip')).toHaveCount(0);
  });

  test('surfaces a stale merge undo without restoring anything', async ({ page }) => {
    // GIVEN one merge whose slot a newer merge replaced elsewhere
    await mockPeople(page, [person(3, 'Anna', 10), person(8, 'Ben', 6)], { staleUndo: true });
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    const cards = page.locator('[data-testid="person-card"]');

    await page.locator('[data-action="select-mode"]').click();
    await page.locator('[data-action="batch-select-all"]').click();
    await page.locator('[data-action="batch-merge-people"]').click();
    await page.getByTestId('merge-confirm').click();
    await expect(cards).toHaveCount(1);
    const undoAction = page.locator('.toast-action');
    await expect(undoAction).toHaveText('Undo merge');

    // WHEN the user clicks the stale undo
    await undoAction.click();

    // THEN the refusal is reported, nothing is restored, and the current slot
    // still backs the chip
    await expect(page.locator('.toast-error')).toContainText('can no longer be undone');
    await expect(cards).toHaveCount(1);
    await expect(page.getByTestId('people-undo-chip')).toBeVisible();
  });

  test('warns before merging visible persons into a hidden keeper', async ({ page }) => {
    await mockPeople(page, [person(3, 'Anna', 10, true), person(8, 'Ben', 6)]);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    const cards = page.locator('[data-testid="person-card"]');

    await page.getByTestId('people-hidden-toggle').click();
    await expect(cards).toHaveCount(2);

    await page.locator('[data-action="select-mode"]').click();
    await cards.nth(0).locator('[data-testid="person-open"]').click();
    await cards.nth(1).locator('[data-testid="person-open"]').click();
    await page.locator('[data-action="batch-merge-people"]').click();

    // THEN the hidden keeper is the default and the consequence is announced
    const dialog = page.getByTestId('people-merge-dialog');
    await expect(dialog.getByTestId('merge-keeper').nth(0)).toBeChecked();
    await expect(dialog.getByTestId('merge-hidden-warning')).toContainText(
      'leaves the default list'
    );
  });

  test('disables merge above the person limit and explains why', async ({ page }) => {
    const many = Array.from({ length: 22 }, (_, index) =>
      person(index + 1, `Person ${index + 1}`, 3)
    );
    await mockPeople(page, many);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');

    await page.locator('[data-action="select-mode"]').click();
    await page.locator('[data-action="batch-select-all"]').click();

    await expect(page.getByTestId('people-merge-limit-hint')).toContainText('at most 21 persons');
    await expect(page.locator('[data-action="batch-merge-people"]')).toBeDisabled();
  });

  test('bulk-hides the selected persons in one step', async ({ page }) => {
    await mockPeople(page, [person(3, 'Anna', 3), person(8, 'Ben', 3)]);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    const cards = page.locator('[data-testid="person-card"]');

    await page.locator('[data-action="select-mode"]').click();
    await page.locator('[data-action="batch-select-all"]').click();
    await page.locator('[data-action="batch-hide-people"]').click();

    await expect(page.locator('.toast')).toContainText('2 persons hidden');
    await expect(cards).toHaveCount(0);
  });

  test('Escape cancels the merge dialog without dropping the selection', async ({ page }) => {
    await mockPeople(page, [person(3, 'Anna', 10), person(8, 'Ben', 6)]);
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    const cards = page.locator('[data-testid="person-card"]');

    await page.locator('[data-action="select-mode"]').click();
    await cards.nth(0).locator('[data-testid="person-open"]').click();
    await cards.nth(1).locator('[data-testid="person-open"]').click();
    await page.locator('[data-action="batch-merge-people"]').click();
    const dialog = page.getByTestId('people-merge-dialog');
    await expect(dialog).toBeVisible();

    // WHEN the user dismisses the dialog with Escape
    await page.keyboard.press('Escape');

    // THEN only the dialog closes; the two-person selection survives and can
    // still be merged
    await expect(dialog).toBeHidden();
    await expect(page.getByTestId('selection-count')).toContainText('2 selected');
    await expect(page.locator('[data-action="batch-merge-people"]')).toBeEnabled();
  });

  test('keeps the selection when bulk hide fails so the user can retry', async ({ page }) => {
    await mockPeople(page, [person(3, 'Anna', 3), person(8, 'Ben', 3)], { failHide: true });
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    const cards = page.locator('[data-testid="person-card"]');

    await page.locator('[data-action="select-mode"]').click();
    await page.locator('[data-action="batch-select-all"]').click();
    await page.locator('[data-action="batch-hide-people"]').click();

    await expect(page.locator('.toast')).toContainText('Batch action failed');
    await expect(page.getByTestId('selection-count')).toContainText('2 selected');
    await expect(cards).toHaveCount(2);
  });

  test('prunes a vanished person from the merge dialog and retries successfully', async ({
    page,
  }) => {
    // GIVEN three clusters, one of which disappears before the merge request
    await mockPeople(page, [person(3, 'Anna', 10), person(8, 'Ben', 6), person(9, 'Cleo', 5)], {
      dropOnMerge: 9,
    });
    await TestHelpers.goto(page);
    await TestHelpers.navigateToView(page, 'people');
    const cards = page.locator('[data-testid="person-card"]');

    await page.locator('[data-action="select-mode"]').click();
    await page.locator('[data-action="batch-select-all"]').click();
    await page.locator('[data-action="batch-merge-people"]').click();
    const dialog = page.getByTestId('people-merge-dialog');
    await dialog.getByTestId('merge-confirm').click();

    // THEN the 404 is reported inline and the dead person is gone from the
    // dialog, so the same button can retry
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('alert')).toContainText('gone');
    await expect(dialog.getByTestId('merge-keeper')).toHaveCount(2);
    await dialog.getByTestId('merge-confirm').click();

    await expect(cards).toHaveCount(1);
    await expect(cards.first()).toContainText('Anna');
  });
});
