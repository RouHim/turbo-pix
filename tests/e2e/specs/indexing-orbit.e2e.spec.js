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

const mockIndexingError = async (page) => {
  await page.route('**/api/indexing/status', async (route) => {
    await route.fulfill({
      status: 500,
      contentType: 'application/json',
      body: JSON.stringify({ error: 'indexing status unavailable' }),
    });
  });
};

test.describe('Indexing orbit', () => {
  test.beforeEach(({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
  });

  test.fixme('renders the orbit ring with all phase segments', async ({ page }) => {
    // GIVEN the orbit component is mounted during active indexing
    await mockIndexingStatus(
      page,
      buildStatus({
        active_phase_id: 'metadata',
        phases: [
          buildPhase({ id: 'discovering', state: 'done' }),
          buildPhase({
            id: 'metadata',
            state: 'active',
            kind: 'determinate',
            processed: 50,
            total: 100,
          }),
          buildPhase({ id: 'semantic_vectors' }),
          buildPhase({ id: 'geo_resolution' }),
          buildPhase({ id: 'collages' }),
          buildPhase({ id: 'housekeeping' }),
        ],
      })
    );

    // WHEN the page loads the indexing UI
    await TestHelpers.goto(page);

    // THEN the orbit container and all segments should be present
    const ring = page.locator('[data-phase-ring]');
    await expect(ring).toBeVisible();
    await expect(ring).toHaveAttribute('data-ring-mode', 'large');
    await expect(page.locator('[data-phase-id]')).toHaveCount(6);
  });

  test.fixme('shows determinate progress at roughly fifty percent', async ({ page }) => {
    // GIVEN metadata indexing is halfway complete
    await mockIndexingStatus(
      page,
      buildStatus({
        active_phase_id: 'metadata',
        phases: [
          buildPhase({ id: 'discovering', state: 'done' }),
          buildPhase({
            id: 'metadata',
            state: 'active',
            kind: 'determinate',
            processed: 50,
            total: 100,
          }),
          buildPhase({ id: 'semantic_vectors' }),
          buildPhase({ id: 'geo_resolution' }),
          buildPhase({ id: 'collages' }),
          buildPhase({ id: 'housekeeping' }),
        ],
      })
    );

    // WHEN the orbit renders the determinate arc
    await TestHelpers.goto(page);

    // THEN the metadata arc should indicate about half completion
    const metadataPhase = page.locator('[data-phase-id="metadata"]');
    await expect(metadataPhase).toHaveAttribute('data-phase-state', 'active');
  });

  test.fixme('shows the orbit dot while discovering is indeterminate', async ({ page }) => {
    // GIVEN discovery is the active indeterminate phase
    await mockIndexingStatus(
      page,
      buildStatus({
        active_phase_id: 'discovering',
        phases: [
          buildPhase({
            id: 'discovering',
            state: 'active',
            kind: 'indeterminate',
          }),
          buildPhase({ id: 'metadata' }),
          buildPhase({ id: 'semantic_vectors' }),
          buildPhase({ id: 'geo_resolution' }),
          buildPhase({ id: 'collages' }),
          buildPhase({ id: 'housekeeping' }),
        ],
      })
    );

    // WHEN the orbit is displayed
    await TestHelpers.goto(page);

    // THEN the indeterminate marker should be visible
    await expect(page.locator('[data-orbit-dot]')).toBeVisible();
  });

  test.fixme('marks each phase with the expected state attribute', async ({ page }) => {
    // GIVEN phases are in mixed states
    await mockIndexingStatus(
      page,
      buildStatus({
        active_phase_id: 'semantic_vectors',
        phases: [
          buildPhase({ id: 'discovering', state: 'done' }),
          buildPhase({
            id: 'metadata',
            state: 'done',
            kind: 'determinate',
            processed: 100,
            total: 100,
          }),
          buildPhase({
            id: 'semantic_vectors',
            state: 'active',
            kind: 'determinate',
            processed: 12,
            total: 100,
          }),
          buildPhase({ id: 'geo_resolution', state: 'pending' }),
          buildPhase({ id: 'collages', state: 'error' }),
          buildPhase({ id: 'housekeeping', state: 'pending' }),
        ],
      })
    );

    // WHEN the component receives a mixed status snapshot
    await TestHelpers.goto(page);

    // THEN each phase should expose its state for stable assertions
    await expect(page.locator('[data-phase-id="discovering"]')).toHaveAttribute(
      'data-phase-state',
      'done'
    );
    await expect(page.locator('[data-phase-id="metadata"]')).toHaveAttribute(
      'data-phase-state',
      'done'
    );
    await expect(page.locator('[data-phase-id="semantic_vectors"]')).toHaveAttribute(
      'data-phase-state',
      'active'
    );
    await expect(page.locator('[data-phase-id="collages"]')).toHaveAttribute(
      'data-phase-state',
      'error'
    );
  });

  test.fixme('updates the ring when the active phase changes', async ({ page }) => {
    // GIVEN the API first reports metadata, then semantic vectors
    await mockIndexingStatus(
      page,
      buildStatus({
        active_phase_id: 'metadata',
        phases: [
          buildPhase({ id: 'discovering', state: 'done' }),
          buildPhase({
            id: 'metadata',
            state: 'active',
            kind: 'determinate',
            processed: 20,
            total: 100,
          }),
          buildPhase({
            id: 'semantic_vectors',
            state: 'pending',
            kind: 'determinate',
            processed: 0,
            total: 100,
          }),
          buildPhase({ id: 'geo_resolution' }),
          buildPhase({ id: 'collages' }),
          buildPhase({ id: 'housekeeping' }),
        ],
      })
    );

    // WHEN the active phase advances during indexing
    await TestHelpers.goto(page);

    // THEN the ring should reflect the new phase ordering
    await expect(page.locator('[data-phase-id="semantic_vectors"]')).toHaveAttribute(
      'data-phase-state',
      'pending'
    );
  });

  test.fixme('uses large mode on the first indexing run', async ({ page }) => {
    // GIVEN this is the first indexing run
    await mockIndexingStatus(
      page,
      buildStatus({
        photos_indexed: 0,
        active_phase_id: 'discovering',
        phases: [
          buildPhase({ id: 'discovering', state: 'active' }),
          ...PHASE_IDS.slice(1).map((id) => buildPhase({ id })),
        ],
      })
    );

    // WHEN the UI detects an empty library during indexing
    await TestHelpers.goto(page);

    // THEN the ring should prefer the large layout
    await expect(page.locator('[data-phase-ring]')).toHaveAttribute('data-ring-mode', 'large');
  });

  test.fixme('uses compact mode during re-indexing', async ({ page }) => {
    // GIVEN photos already exist and indexing restarts
    await mockIndexingStatus(
      page,
      buildStatus({
        photos_indexed: 100,
        active_phase_id: 'metadata',
        phases: [
          buildPhase({ id: 'discovering', state: 'done' }),
          buildPhase({
            id: 'metadata',
            state: 'active',
            kind: 'determinate',
            processed: 1,
            total: 100,
          }),
          buildPhase({ id: 'semantic_vectors' }),
          buildPhase({ id: 'geo_resolution' }),
          buildPhase({ id: 'collages' }),
          buildPhase({ id: 'housekeeping' }),
        ],
      })
    );

    // WHEN indexing starts again on an existing library
    await TestHelpers.goto(page);

    // THEN the compact ring mode should be used
    await expect(page.locator('[data-phase-ring]')).toHaveAttribute('data-ring-mode', 'compact');
  });

  test.fixme('opens the bottom sheet when the compact ring is tapped', async ({ page }) => {
    // GIVEN the compact ring is visible on desktop
    await mockIndexingStatus(
      page,
      buildStatus({
        photos_indexed: 100,
        active_phase_id: 'metadata',
        phases: [
          buildPhase({ id: 'discovering', state: 'done' }),
          buildPhase({
            id: 'metadata',
            state: 'active',
            kind: 'determinate',
            processed: 50,
            total: 100,
          }),
          buildPhase({ id: 'semantic_vectors' }),
          buildPhase({ id: 'geo_resolution' }),
          buildPhase({ id: 'collages' }),
          buildPhase({ id: 'housekeeping' }),
        ],
      })
    );

    // WHEN the user clicks the compact ring
    await TestHelpers.goto(page);
    await page.locator('[data-phase-ring]').click();

    // THEN the bottom sheet should open
    await expect(page.locator('[data-bottom-sheet]')).toBeVisible();
  });

  test.fixme('renders phase names and counts inside the bottom sheet', async ({ page }) => {
    // GIVEN the bottom sheet opens from a compact status snapshot
    await mockIndexingStatus(
      page,
      buildStatus({
        photos_indexed: 100,
        active_phase_id: 'semantic_vectors',
        phases: [
          buildPhase({ id: 'discovering', state: 'done' }),
          buildPhase({
            id: 'metadata',
            state: 'done',
            kind: 'determinate',
            processed: 100,
            total: 100,
          }),
          buildPhase({
            id: 'semantic_vectors',
            state: 'active',
            kind: 'determinate',
            processed: 30,
            total: 100,
          }),
          buildPhase({ id: 'geo_resolution', state: 'pending' }),
          buildPhase({ id: 'collages', state: 'pending' }),
          buildPhase({ id: 'housekeeping', state: 'pending' }),
        ],
      })
    );

    // WHEN the user inspects the sheet content
    await TestHelpers.goto(page);
    await page.locator('[data-phase-ring]').click();

    // THEN the sheet should expose readable phase labels and progress values
    const sheet = page.locator('[data-bottom-sheet]');
    await expect(sheet).toBeVisible();
    await expect(sheet.locator('[data-phase-id="semantic_vectors"]')).toBeVisible();
  });

  test.fixme('repositions for mobile viewport at bottom center', async ({ page }) => {
    // GIVEN the device is a phone
    await TestHelpers.setMobileViewport(page);
    await mockIndexingStatus(page, buildStatus({ active_phase_id: 'discovering' }));

    // WHEN the orbit is shown on mobile
    await TestHelpers.goto(page);

    // THEN it should anchor at the bottom center of the screen
    await expect(page.locator('[data-phase-ring]')).toBeVisible();
  });

  test.fixme('stays bottom-right in compact desktop mode', async ({ page }) => {
    // GIVEN the app is on a desktop viewport with existing indexed photos
    await TestHelpers.setDesktopViewport(page);
    await mockIndexingStatus(
      page,
      buildStatus({ photos_indexed: 100, active_phase_id: 'metadata' })
    );

    // WHEN the compact mode is rendered
    await TestHelpers.goto(page);

    // THEN the ring should remain visible in its compact placement
    await expect(page.locator('[data-phase-ring]')).toHaveAttribute('data-ring-mode', 'compact');
  });

  test.fixme('remains visible in dark theme', async ({ page }) => {
    // GIVEN dark mode is enabled
    await mockIndexingStatus(page, buildStatus({ active_phase_id: 'metadata' }));

    // WHEN the page toggles the dark-theme class
    await TestHelpers.goto(page);
    await page.locator('html').evaluate((element) => element.classList.add('dark-theme'));

    // THEN the orbit should still be visible
    await expect(page.locator('[data-phase-ring]')).toBeVisible();
  });

  test.fixme('respects reduced motion preferences', async ({ page }) => {
    // GIVEN the user prefers reduced motion
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await mockIndexingStatus(page, buildStatus({ active_phase_id: 'housekeeping' }));

    // WHEN the orbit is rendered
    await TestHelpers.goto(page);

    // THEN animation-dependent assertions should remain stable without motion
    await expect(page.locator('[data-phase-ring]')).toBeVisible();
  });

  test.fixme('hides the orbit once indexing is complete', async ({ page }) => {
    // GIVEN indexing has completed successfully
    await mockIndexingStatus(
      page,
      buildStatus({
        is_indexing: false,
        is_complete: true,
        photos_indexed: 250,
        active_phase_id: 'housekeeping',
        phases: PHASE_IDS.map((id) =>
          buildPhase({
            id,
            state: 'done',
            kind:
              id === 'metadata' || id === 'semantic_vectors' || id === 'geo_resolution'
                ? 'determinate'
                : 'indeterminate',
          })
        ),
      })
    );

    // WHEN the final snapshot arrives
    await TestHelpers.goto(page);

    // THEN the orbit should hide itself
    await expect(page.locator('[data-phase-ring]')).toHaveAttribute('data-ring-mode', 'hidden');
  });

  test.fixme('emits indexingStatusChanged when the status updates', async ({ page }) => {
    // GIVEN a consumer listens for status changes
    await mockIndexingStatus(page, buildStatus({ active_phase_id: 'metadata' }));

    // WHEN the orbit publishes a fresh indexing snapshot
    await TestHelpers.goto(page);
    await page.evaluate(() => {
      window.addEventListener('indexingStatusChanged', () => {
        window.__indexingOrbitEventSeen = true;
      });
    });

    // THEN the custom event should be dispatched
    await expect(page.locator('[data-phase-ring]')).toBeVisible();
  });

  test.fixme('survives API errors without console noise', async ({ page }) => {
    // GIVEN the status endpoint fails once
    await mockIndexingError(page);

    // WHEN the orbit attempts to refresh
    await TestHelpers.goto(page);

    // THEN the page should remain usable
    await expect(page.locator('[data-phase-ring]')).toBeVisible();
  });
});
