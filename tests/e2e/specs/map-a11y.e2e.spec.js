import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { TestHelpers } from '../setup/test-helpers.js';

const AXE_RULES = [
  'color-contrast',
  'target-size',
  'aria-valid-attr-value',
  'aria-prohibited-attr',
];

/** Port 9 (discard) has no listener, so every tile request fails to connect. */
const TILE_FAILURE_URL = 'http://127.0.0.1:9/{z}/{x}/{y}.png';

const MAP_PHOTOS_ENDPOINT = '**/api/photos/map**';
const MAP_FEATURES = '[data-map-cluster], [data-map-location]';

/** Repoints the tile template at an endpoint the test controls (FR-002). */
async function mockTileEndpoint(page, tileUrl) {
  await page.route('**/api/config', async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    await route.fulfill({ response, json: { ...body, tile_url: tileUrl } });
  });
}

/** Waits until the map has drawn at least one feature. */
async function waitForMapFeatures(page) {
  await expect
    .poll(async () => page.locator(MAP_FEATURES).count(), { timeout: 15000 })
    .toBeGreaterThan(0);
}

/**
 * Waits until marker replacement has stopped. Loading the result set fits the
 * map once, and that `moveend` re-render replaces every marker element — Tab
 * pressed between the two would hand focus to an element that is gone a moment
 * later. Two identical samples (the first a full animation length apart) mean
 * the DOM has settled, so keyboard focus sticks.
 */
async function waitForSettledFeatures(page) {
  let previous = null;
  await expect
    .poll(
      async () => {
        const signature = await page.evaluate(
          (selector) =>
            [...document.querySelectorAll(selector)]
              .map(
                (element) =>
                  element.getAttribute('data-map-cluster') ??
                  element.getAttribute('data-map-location')
              )
              .join('|'),
          MAP_FEATURES
        );
        const settled = signature !== '' && signature === previous;
        previous = signature;
        return settled;
      },
      { timeout: 15000, intervals: [500, 500, 1000] }
    )
    .toBe(true);
}

/** Mouse-only setup: expands clusters until individual location markers render. */
async function revealLocationMarker(page) {
  for (let attempt = 0; attempt < 6; attempt += 1) {
    if ((await page.locator('[data-map-location]').count()) > 0) break;
    const cluster = page.locator('[data-map-cluster]').first();
    if ((await cluster.count()) === 0) break;
    await cluster.click();
    await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
  }

  const marker = page.locator('[data-map-location]').first();
  await expect(marker).toBeVisible();
  return marker;
}

/** The zoom levels of the tiles in the DOM — Leaflet's observable zoom. */
function renderedTileZoomLevels(page) {
  return page.evaluate(() =>
    [...document.querySelectorAll('.leaflet-tile-pane img.leaflet-tile')]
      .map((image) => new URL(image.src).pathname.split('/')[1])
      .filter(Boolean)
  );
}

test.describe('Map degradation and accessibility', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
  });

  test('an unreachable tile endpoint still renders markers and attribution', async ({ page }) => {
    // FR-015/SC-005: tiles are decoration — losing them must not lose the data.
    await mockTileEndpoint(page, TILE_FAILURE_URL);
    await TestHelpers.goto(page, '/map');

    await expect(page.locator('[data-testid="map-tiles-notice"]')).toBeVisible();
    const attribution = page.locator('.leaflet-control-attribution');
    await expect(attribution).toBeVisible();
    await expect(attribution).toContainText('OpenStreetMap');
    await waitForMapFeatures(page);
  });

  test('shows the loading overlay, reports a failed load and recovers on Retry', async ({
    page,
  }) => {
    await TestHelpers.stubMapTiles(page);

    let phase = 'gated';
    let mapRequests = 0;
    let releaseFirstLoad;
    const firstLoadGate = new Promise((resolve) => {
      releaseFirstLoad = resolve;
    });

    await page.route(MAP_PHOTOS_ENDPOINT, async (route) => {
      mapRequests += 1;
      if (phase === 'gated') {
        await firstLoadGate;
        await route.continue();
        return;
      }
      if (phase === 'failing') {
        await route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'map listing unavailable' }),
        });
        return;
      }
      await route.continue();
    });

    await TestHelpers.goto(page, '/map');
    // The listing request is intercepted and held open, so the overlay the user
    // sees while the map has no data is observable instead of a race.
    await expect.poll(() => mapRequests).toBeGreaterThan(0);
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await expect(page.locator('[data-testid="map-loading"]')).toBeVisible();

    phase = 'serving';
    releaseFirstLoad();
    await expect(page.locator('[data-testid="map-loading"]')).toBeHidden();
    await waitForMapFeatures(page);

    // A load that fails replaces the overlay with the error state and a retry.
    phase = 'failing';
    await page.reload({ waitUntil: 'domcontentloaded' });
    const errorOverlay = page.locator('[data-testid="map-error"]');
    await expect(errorOverlay).toBeVisible();
    const retry = errorOverlay.locator('button');
    await expect(retry).toBeVisible();

    phase = 'serving';
    await page.unroute(MAP_PHOTOS_ENDPOINT);
    await retry.click();

    await expect(errorOverlay).toBeHidden();
    await expect(page.locator('[data-testid="map-loading"]')).toBeHidden();
    await waitForMapFeatures(page);
  });

  test('attribution stays inside a 375px viewport', async ({ page }) => {
    // SC-006: the legally required attribution may never be pushed off-screen.
    await TestHelpers.setMobileViewport(page);
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');

    const attribution = page.locator('.leaflet-control-attribution');
    await expect(attribution).toBeVisible();
    const box = await attribution.boundingBox();
    expect(box).not.toBeNull();
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(375);
  });

  test('Tab reaches a map feature and a popup thumbnail hands off to the viewer', async ({
    page,
  }) => {
    // FR-018. The popup cycle itself (Enter, Escape, reopen) is covered by
    // map.e2e.spec.js; this covers the two ends of the keyboard path: Tab from
    // the map container into a feature, and the thumbnail → viewer handoff.
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await waitForMapFeatures(page);
    await waitForSettledFeatures(page);

    await page.locator('[data-testid="map-canvas"]').focus();
    await page.keyboard.press('Tab');
    await expect(page.locator('[data-map-cluster]:focus, [data-map-location]:focus')).toHaveCount(
      1
    );

    const marker = await revealLocationMarker(page);
    await marker.focus();
    await page.keyboard.press('Enter');

    const popup = page.locator('.leaflet-popup');
    await expect(popup).toBeVisible();
    const thumbnail = popup.locator('[data-map-popup-photo]').first();
    await expect(thumbnail).toBeFocused();

    const hash = await thumbnail.getAttribute('data-map-popup-photo');
    await page.keyboard.press('Enter');

    await expect(page.locator('#photo-viewer')).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`photo=${hash}`));
  });

  test('reduced motion keeps the keyboard path into and out of the popup', async ({ page }) => {
    // FR-018/SC-007. The reduce-motion path opens the popup without the fade or
    // the pan, so the focus handoff has to survive without those frames: a user
    // with the OS preference set still has to land on a thumbnail, and Escape
    // still has to dismiss the popup from there.
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-view"]')).toHaveAttribute(
      'data-reduced-motion',
      'true'
    );
    await waitForMapFeatures(page);
    await waitForSettledFeatures(page);

    const marker = await revealLocationMarker(page);
    await marker.focus();
    await page.keyboard.press('Enter');

    const popup = page.locator('.leaflet-popup');
    await expect(popup).toBeVisible();
    // Landing on the first thumbnail is what keeps the popup's photos one Tab
    // away; without it a keyboard user walks the remaining markers first.
    await expect(popup.locator('[data-map-popup-photo]').first()).toBeFocused();

    // Escape only reaches the map's own handler while focus sits inside the
    // popup pane, and Leaflet's Escape hook is unhooked once the map container
    // has lost focus — so a thumbnail that is not focused leaves the popup
    // undismissable. The marker gets focus back on the way out.
    await page.keyboard.press('Escape');
    await expect(popup).toHaveCount(0);
    await expect(page.locator('[data-map-location]:focus')).toHaveCount(1);
  });

  test('reduced motion turns off animated zooming', async ({ page }) => {
    // FR-018/SC-007.
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-view"]')).toHaveAttribute(
      'data-reduced-motion',
      'true'
    );
    await waitForMapFeatures(page);

    // Leaflet marks an animated zoom by putting `.leaflet-zoom-anim` on the map
    // pane for the length of the transition. Watching for it while the zoom runs
    // is the only way to catch a pane that would come and go inside one tick.
    await page.evaluate(() => {
      const canvas = document.querySelector('[data-testid="map-canvas"]');
      window.reducedMotionProbe = { animated: false };
      window.reducedMotionProbeObserver = new MutationObserver(() => {
        if (canvas.querySelector('.leaflet-zoom-anim')) window.reducedMotionProbe.animated = true;
      });
      window.reducedMotionProbeObserver.observe(canvas, {
        subtree: true,
        childList: true,
        attributes: true,
        attributeFilter: ['class'],
      });
    });

    const zoomLevelsBefore = await renderedTileZoomLevels(page);
    await page.locator('.leaflet-control-zoom-in').click();

    // Prove the zoom really happened before claiming it was not animated.
    await expect
      .poll(
        async () => {
          const levels = await renderedTileZoomLevels(page);
          return levels.some((level) => !zoomLevelsBefore.includes(level));
        },
        { timeout: 10000 }
      )
      .toBe(true);
    await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    expect(await page.evaluate(() => window.reducedMotionProbe.animated)).toBe(false);

    // Control: with no OS preference the same marker reports `false`, so a
    // hard-coded attribute could not pass this test.
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await page.reload({ waitUntil: 'domcontentloaded' });
    await expect(page.locator('[data-testid="map-view"]')).toHaveAttribute(
      'data-reduced-motion',
      'false'
    );
  });

  test('map view has no axe violations', async ({ page }) => {
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await waitForMapFeatures(page);

    const results = await new AxeBuilder({ page })
      .include('[data-testid="map-view"]')
      .withRules(AXE_RULES)
      .analyze();
    expect(results.violations).toEqual([]);
    // Guard against a pass over nothing: the scoped subtree has to have been
    // analyzed for the empty violation list to mean anything.
    expect(results.passes.flatMap((rule) => rule.nodes).length).toBeGreaterThan(0);
  });
});
