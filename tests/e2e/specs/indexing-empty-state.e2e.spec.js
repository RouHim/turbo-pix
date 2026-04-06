import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const PHASE_IDS = [
  'discovering',
  'metadata',
  'semantic_vectors',
  'geo_resolution',
  'collages',
  'housekeeping',
];

const buildPhase = (overrides) => ({
  state: 'pending',
  kind: 'indeterminate',
  processed: 0,
  total: null,
  errors: 0,
  current_item: null,
  ...overrides,
});

const buildStatus = (overrides = {}) => ({
  is_indexing: true,
  is_complete: false,
  started_at: new Date().toISOString(),
  active_phase_id: 'discovering',
  photos_indexed: 0,
  phases: PHASE_IDS.map((id) => buildPhase({ id })),
  ...overrides,
});

const mockIndexingStatus = async (page, body, status = 200) => {
  await page.route('**/api/indexing/status', async (route) => {
    await route.fulfill({
      status,
      contentType: 'application/json',
      body: JSON.stringify(body),
    });
  });
};

const mockIndexingStatusSequence = async (page, responses) => {
  let index = 0;

  await page.route('**/api/indexing/status', async (route) => {
    const nextResponse = responses[Math.min(index, responses.length - 1)];
    index += 1;

    await route.fulfill({
      status: nextResponse.status ?? 200,
      contentType: 'application/json',
      body: JSON.stringify(nextResponse.body),
    });
  });
};

const mockEmptyPhotos = async (page) => {
  const emptyResponse = async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ photos: [], total: 0 }),
    });
  };
  await page.route('**/api/photos?**', emptyResponse);
  await page.route('**/api/photos', emptyResponse);
};

// metadata=done avoids 20s waitForIndexingCompletion() block in app.js
const activeIndexingStatus = () =>
  buildStatus({
    active_phase_id: 'semantic_vectors',
    phases: [
      buildPhase({ id: 'discovering', state: 'done' }),
      buildPhase({ id: 'metadata', state: 'done' }),
      buildPhase({
        id: 'semantic_vectors',
        state: 'running',
        kind: 'determinate',
        processed: 10,
        total: 100,
      }),
      buildPhase({ id: 'geo_resolution' }),
      buildPhase({ id: 'collages' }),
      buildPhase({ id: 'housekeeping' }),
    ],
  });

const idleStatus = () =>
  buildStatus({
    is_indexing: false,
    is_complete: true,
    active_phase_id: null,
    phases: PHASE_IDS.map((id) => buildPhase({ id, state: 'done' })),
  });

test.describe('indexing empty state', () => {
  test.beforeEach(({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
  });

  test('shows indexing-in-progress state when indexing is active and no photos', async ({
    page,
  }) => {
    // GIVEN indexing is active (metadata done, semantic_vectors running)
    // AND /api/photos returns empty
    await mockIndexingStatus(page, activeIndexingStatus());
    await mockEmptyPhotos(page);

    // WHEN page loads
    await TestHelpers.goto(page);

    // THEN the indexing-in-progress empty state is visible
    const indexingState = page.locator('.indexing-in-progress');
    await expect(indexingState).toBeVisible({ timeout: 10000 });

    // AND text "Indexing Your Photos" (or i18n equivalent) is visible
    await expect(indexingState.locator('.error-state-title')).toBeVisible();

    // AND no "No Photos Found" text is visible
    await expect(page.getByText('No Photos Found')).not.toBeVisible();

    // AND no refresh button is visible
    await expect(page.locator('.error-state-button')).not.toBeVisible();
  });

  test('shows "No Photos Found" when indexing is NOT active and no photos', async ({ page }) => {
    // GIVEN indexing is NOT active
    // AND /api/photos returns empty
    await mockIndexingStatus(page, idleStatus());
    await mockEmptyPhotos(page);

    // WHEN page loads
    await TestHelpers.goto(page);

    // THEN "No Photos Found" is visible
    await expect(page.getByText('No Photos Found')).toBeVisible({ timeout: 10000 });

    // AND the refresh button IS visible
    await expect(page.locator('.error-state-button')).toBeVisible();

    // AND no indexing-in-progress element exists
    await expect(page.locator('.indexing-in-progress')).toHaveCount(0);
  });

  test('transitions from indexing to idle and auto-reloads photos', async ({ page }) => {
    // GIVEN indexing starts active with empty photos, then transitions to idle
    const activeBody = activeIndexingStatus();
    const idleBody = idleStatus();

    await mockIndexingStatusSequence(page, [
      { body: activeBody },
      { body: activeBody },
      { body: activeBody },
      { body: idleBody },
    ]);

    let photosCallCount = 0;
    const photosRoute = async (route) => {
      photosCallCount += 1;
      if (photosCallCount <= 1) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ photos: [], total: 0 }),
        });
      } else {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            photos: [
              {
                hash_sha256: 'abc123test000000000000000000000000000000000000000000000000000000',
                filename: 'test.jpg',
                mime_type: 'image/jpeg',
                width: 800,
                height: 600,
                file_size: 12345,
                taken_at: '2024-01-01T12:00:00Z',
                is_favorite: false,
              },
            ],
            total: 1,
          }),
        });
      }
    };
    await page.route('**/api/photos?**', photosRoute);
    await page.route('**/api/photos', photosRoute);

    // WHEN page loads
    await TestHelpers.goto(page);

    // THEN initially the indexing-in-progress state is shown
    await expect(page.locator('.indexing-in-progress')).toBeVisible({ timeout: 10000 });

    // WHEN indexingOrbit detects idle (after poll transitions)
    // THEN photoGrid.loadPhotos() is called and a photo card appears
    await expect(page.locator('[data-photo-id]')).toBeVisible({ timeout: 15000 });
  });

  test('large ring mode shown for first-time user during active indexing with 0 photos', async ({
    page,
  }) => {
    // GIVEN first-time user (no localStorage key)
    await page.addInitScript(() => {
      window.localStorage.removeItem('turbopix_has_indexed');
    });

    // AND indexing active, 0 photos
    await mockIndexingStatus(
      page,
      buildStatus({
        active_phase_id: 'semantic_vectors',
        photos_indexed: 0,
        phases: [
          buildPhase({ id: 'discovering', state: 'done' }),
          buildPhase({ id: 'metadata', state: 'done' }),
          buildPhase({
            id: 'semantic_vectors',
            state: 'running',
            kind: 'determinate',
            processed: 5,
            total: 50,
          }),
          buildPhase({ id: 'geo_resolution' }),
          buildPhase({ id: 'collages' }),
          buildPhase({ id: 'housekeeping' }),
        ],
      })
    );
    await mockEmptyPhotos(page);

    // WHEN page loads
    await TestHelpers.goto(page);

    // THEN the orbit ring renders in "large" mode
    const ring = page.locator('[data-phase-ring]');
    await expect(ring).toHaveAttribute('data-ring-mode', 'large', { timeout: 10000 });

    // AND the indexing-in-progress empty state is visible
    await expect(page.locator('.indexing-in-progress')).toBeVisible();
  });

  test('hides loading spinner during first-run indexing with zero photos', async ({ page }) => {
    // GIVEN indexing is active (metadata done, semantic_vectors running)
    // AND /api/photos returns empty
    await mockIndexingStatus(page, activeIndexingStatus());
    await mockEmptyPhotos(page);

    // WHEN page loads
    await TestHelpers.goto(page);

    // THEN the indexing-in-progress empty state is visible
    await expect(page.locator('.indexing-in-progress')).toBeVisible({ timeout: 10000 });

    // AND the loading spinner does NOT have the .show class (not visible as overlay)
    await expect(page.locator('#loading-indicator')).not.toHaveClass(/show/);
  });

  test('hides load-more container during first-run indexing with zero photos', async ({ page }) => {
    // GIVEN indexing is active (metadata done, semantic_vectors running)
    // AND /api/photos returns empty
    await mockIndexingStatus(page, activeIndexingStatus());
    await mockEmptyPhotos(page);

    // WHEN page loads
    await TestHelpers.goto(page);

    // THEN the indexing-in-progress empty state is visible
    await expect(page.locator('.indexing-in-progress')).toBeVisible({ timeout: 10000 });

    // AND the load-more container is NOT visible
    await expect(page.locator('#load-more-container')).not.toBeVisible();
  });
});
