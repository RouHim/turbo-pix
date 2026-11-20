import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const drawer = (page) => page.getByTestId('sidebar-nav');
const viewer = (page) => page.locator('#photo-viewer');
const sheet = (page) => page.getByTestId('viewer-sidebar');

async function openViewer(page) {
  const cards = await TestHelpers.getPhotoCards(page);
  expect(cards.length).toBeGreaterThan(0);
  await cards[0].scrollIntoViewIfNeeded();
  await cards[0].click();
  await TestHelpers.verifyViewerOpen(page);
}

async function openDetails(page) {
  await page.locator('.metadata-btn').click();
  await expect(sheet(page)).toHaveClass(/show/);
}

async function dispatchTouchPointer(page, type, x, y) {
  await page.evaluate(
    ({ type, x, y }) => {
      const target = type === 'pointerdown' ? document.elementFromPoint(x, y) : window;
      if (!target) throw new Error(`No pointer target at ${x},${y}`);
      target.dispatchEvent(
        new PointerEvent(type, {
          bubbles: true,
          cancelable: true,
          pointerType: 'touch',
          pointerId: 1,
          isPrimary: true,
          clientX: x,
          clientY: y,
          buttons: type === 'pointerup' ? 0 : 1,
        })
      );
    },
    { type, x, y }
  );
}

test.describe('Mobile gestures', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await TestHelpers.setMobileViewport(page);
  });

  test('swipe right opens the app drawer and swipe left closes it', async ({ page }) => {
    const viewport = page.viewportSize();
    const y = Math.floor(viewport.height / 3);

    await TestHelpers.performSwipeOn(page, '.main-content', 40, y, 240, y);

    await expect(drawer(page)).toBeVisible();
    await expect(drawer(page)).toHaveClass(/open/);
    await expect(page.getByTestId('sidebar-overlay')).toBeVisible();

    await TestHelpers.performSwipeOn(page, '#sidebar-nav', 250, y, 40, y);

    await expect(drawer(page)).toBeHidden();
    await expect(drawer(page)).not.toHaveClass(/open/);
  });

  test('map surface keeps its horizontal pan and does not open the drawer', async ({ page }) => {
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.setDesktopViewport(page);
    await TestHelpers.navigateToView(page, 'map');
    await TestHelpers.setMobileViewport(page);
    await expect(page.getByTestId('map-view')).toBeVisible();

    const viewport = page.viewportSize();
    const y = Math.floor(viewport.height / 2);
    await TestHelpers.performSwipeOn(page, '.map-view', 40, y, 240, y);

    await expect(drawer(page)).toBeHidden();
  });

  test('selection mode suppresses the drawer swipe', async ({ page }) => {
    await page.locator('[data-action="select-mode"]').click();
    await expect(page.locator('[data-action="select-mode"]')).toHaveAttribute(
      'aria-pressed',
      'true'
    );

    const viewport = page.viewportSize();
    const y = Math.floor(viewport.height / 3);
    await TestHelpers.performSwipeOn(page, '.main-content', 40, y, 240, y);

    await expect(drawer(page)).toBeHidden();
  });

  test('keeps the incoming neighbour while a slow image loads', async ({ page }) => {
    const cards = await TestHelpers.getPhotoCards(page);
    expect(cards.length).toBeGreaterThan(1);
    const secondHash = await cards[1].getAttribute('data-photo-id');

    let release = () => {};
    const gate = new Promise((resolve) => {
      release = resolve;
    });
    await page.route(`**/api/photos/${secondHash}/file*`, async (route) => {
      await gate;
      await route.continue();
    });

    await cards[0].click();
    await TestHelpers.verifyViewerOpen(page);
    await expect(page.locator('#viewer-image')).toHaveClass(/loaded/);

    await TestHelpers.swipeLeft(page);

    // The slide has ended but the new image is still decoded from the blocked
    // request: the neighbour preview and the loading indicator must still be
    // the visible surfaces instead of a blank viewer.
    await expect(page.locator('.viewer-loading-indicator.show')).toBeVisible();
    await expect(page.locator('.viewer-adjacent-next.visible')).toHaveCount(1);

    release();
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(secondHash);
    await expect(page.locator('#viewer-image')).toHaveClass(/loaded/);
  });

  test('the details sheet is visible while it follows the finger', async ({ page }) => {
    await openViewer(page);
    await page.waitForFunction(
      () => getComputedStyle(document.querySelector('#photo-viewer')).opacity === '1'
    );

    // Five moves without a touchend: the drag is still live, and the closed
    // sheet's base opacity is 0 unless the dragging state overrides it.
    await page.evaluate(() => {
      const target = document.querySelector('.viewer-main');
      const touch = (y) =>
        new Touch({
          identifier: 1,
          target,
          clientX: 180,
          clientY: y,
          pageX: 180,
          pageY: y,
          radiusX: 2,
          radiusY: 2,
          rotationAngle: 0,
          force: 0.5,
        });
      const dispatch = (type, touches, changed, at) => {
        const event = new TouchEvent(type, {
          bubbles: true,
          cancelable: true,
          touches,
          changedTouches: changed,
        });
        Object.defineProperty(event, 'timeStamp', { value: at });
        target.dispatchEvent(event);
      };
      dispatch('touchstart', [touch(520)], [touch(520)], 1000);
      for (let step = 1; step <= 5; step += 1) {
        const y = 520 - step * 40;
        dispatch('touchmove', [touch(y)], [touch(y)], 1000 + step * 16);
      }
    });

    await expect(sheet(page)).toHaveCSS('opacity', '1');
    const sheetTop = await sheet(page).evaluate((el) => el.getBoundingClientRect().top);
    expect(sheetTop).toBeLessThan(page.viewportSize().height);
  });

  test('swipe up opens details; swipe down closes the sheet before the viewer', async ({
    page,
  }) => {
    await openViewer(page);

    await TestHelpers.swipeUp(page);
    await expect(sheet(page)).toHaveClass(/show/);

    const box = await sheet(page).boundingBox();
    expect(box).not.toBeNull();
    await TestHelpers.performSwipeOn(
      page,
      '[data-testid="viewer-sidebar"]',
      180,
      box.y + 20,
      180,
      box.y + 220
    );

    await expect(sheet(page)).not.toHaveClass(/show/);
    await expect(viewer(page)).toHaveClass(/active/);

    await TestHelpers.swipeDown(page);

    await expect(viewer(page)).not.toHaveClass(/active/);
  });

  test('tapping the photo closes the sheet but keeps the viewer open', async ({ page }) => {
    await openViewer(page);
    await openDetails(page);

    await page.locator('.viewer-main').click({ position: { x: 10, y: 10 } });

    await expect(sheet(page)).not.toHaveClass(/show/);
    await expect(viewer(page)).toHaveClass(/active/);
  });

  test('Escape closes the sheet first', async ({ page }) => {
    await openViewer(page);
    await openDetails(page);

    await page.keyboard.press('Escape');
    await expect(sheet(page)).not.toHaveClass(/show/);
    await expect(viewer(page)).toHaveClass(/active/);

    await page.keyboard.press('Escape');
    await expect(viewer(page)).not.toHaveClass(/active/);
  });

  test('horizontal navigation keeps the details sheet open and updates it', async ({ page }) => {
    await openViewer(page);
    await openDetails(page);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);
    const titleBefore = await page.locator('#photo-title').textContent();

    // Start above the 65vh sheet so the swipe really is on the photo surface.
    await TestHelpers.swipeLeft(page, { startY: 120 });

    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(firstHash);
    await expect(sheet(page)).toHaveClass(/show/);
    await expect.poll(() => page.locator('#photo-title').textContent()).not.toBe(titleBefore);
  });

  test('the gestures also work through the browser touch pipeline', async ({ page }) => {
    // Synthetic DOM TouchEvents bypass `touch-action` and native scroll
    // arbitration. CDP dispatches into the real input pipeline, so this is the
    // check that the sheet body's scroll handoff and the drawer's preventDefault
    // survive a browser that is allowed to scroll the page.
    const client = await page.context().newCDPSession(page);
    const point = (x, y) => ({ x, y, radiusX: 5, radiusY: 5, force: 1 });
    const drag = async (fromX, fromY, toX, toY) => {
      const steps = 10;
      const stepMs = 16;
      const startedAt = Date.now();
      await client.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 });
      await client.send('Input.dispatchTouchEvent', {
        type: 'touchStart',
        touchPoints: [point(fromX, fromY)],
        timestamp: startedAt / 1000,
      });
      for (let step = 1; step <= steps; step += 1) {
        await client.send('Input.dispatchTouchEvent', {
          type: 'touchMove',
          touchPoints: [
            point(fromX + ((toX - fromX) * step) / steps, fromY + ((toY - fromY) * step) / steps),
          ],
          timestamp: (startedAt + step * stepMs) / 1000,
        });
      }
      await client.send('Input.dispatchTouchEvent', {
        type: 'touchEnd',
        touchPoints: [],
        timestamp: (startedAt + (steps + 1) * stepMs) / 1000,
      });
      await client.send('Emulation.setTouchEmulationEnabled', { enabled: false });
    };

    // Load at the mobile viewport instead of resizing after load: a resize can
    // leave the compositor's hit regions on the desktop sidebar for the first
    // gesture (at x=40 the target would not be `.main-content` and the drawer
    // would correctly ignore it). The reload makes this the same first-frame
    // mobile layout a phone gets.
    await page.reload({ waitUntil: 'domcontentloaded' });
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.menu-btn')).toBeVisible();

    // Below the timeline filter (which owns its horizontal gestures and is
    // excluded from the drawer swipe), on the photo grid.
    const year = 450;
    await drag(40, year, 240, year);
    await expect(drawer(page)).toHaveClass(/open/);
    await drag(240, year, 40, year);
    await expect(drawer(page)).not.toHaveClass(/open/);
    // The close transition also outlives the drag's click-suppression window;
    // a card click before the drawer is fully gone would be swallowed.
    await expect(drawer(page)).toBeHidden();

    await openViewer(page);
    // The viewer fades in through a View Transition; real pointer input only
    // reaches it once that overlay is done. Synthetic DOM events bypass this,
    // which is exactly why this test exists.
    await expect(viewer(page)).toHaveCSS('opacity', '1');
    await drag(180, 520, 180, 240);
    await expect(sheet(page)).toHaveClass(/show/);
    await expect(sheet(page)).toHaveCSS('transform', 'matrix(1, 0, 0, 1, 0, 0)');

    // Start in the body, below the 56px handle zone, with scrollTop 0: the
    // sheet must take the downward drag instead of scrolling the viewer.
    await drag(180, 480, 180, 640);
    await expect(sheet(page)).not.toHaveClass(/show/);
    await expect(viewer(page)).toHaveClass(/active/);
  });

  test('video double-tap seeks in the tapped third and announces the step', async ({ page }) => {
    test.setTimeout(60000);
    // The app nav is a drawer on mobile; use the desktop nav to reach Videos,
    // then switch to the mobile viewport before opening the viewer.
    await TestHelpers.setDesktopViewport(page);
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await TestHelpers.setMobileViewport(page);
    const cards = await TestHelpers.getPhotoCards(page);
    expect(cards.length).toBeGreaterThan(0);

    await cards[0].click();
    await TestHelpers.verifyViewerOpen(page);

    const video = page.locator('#viewer-video');
    await expect(video).toBeVisible();
    await expect
      .poll(() => video.evaluate((element) => element.readyState), { timeout: 30000 })
      .toBeGreaterThan(0);

    const box = await video.boundingBox();
    expect(box).not.toBeNull();
    const y = box.y + box.height / 2;
    const duration = await video.evaluate((element) => element.duration);
    expect(Number.isFinite(duration) && duration > 0).toBe(true);
    // The fixture set includes sub-second clips; start below half the duration
    // so both directions have room and the assertion proves the direction.
    const start = Math.max(0.01, Math.min(0.2, duration / 2));

    await video.evaluate((element, position) => {
      element.pause();
      element.currentTime = position;
    }, start);
    await TestHelpers.performDoubleTapOn(page, '.viewer-main', box.x + box.width * 0.1, y);
    await expect(page.getByTestId('viewer-seek-feedback')).toHaveText('−10 s');
    await expect.poll(() => video.evaluate((element) => element.currentTime)).toBeLessThan(start);

    await video.evaluate((element, position) => {
      element.pause();
      element.currentTime = position;
    }, start);
    await TestHelpers.performDoubleTapOn(page, '.viewer-main', box.x + box.width * 0.9, y);
    await expect(page.getByTestId('viewer-seek-feedback')).toHaveText('+10 s');
    await expect
      .poll(() => video.evaluate((element) => element.currentTime))
      .toBeGreaterThan(start);
  });

  test('tap on the photo toggles chrome, with the details sheet closing first', async ({
    page,
  }) => {
    await openViewer(page);
    const image = page.locator('#viewer-image');
    await expect(image).toHaveClass(/loaded/);
    const controls = page.locator('.viewer-controls');
    const close = page.locator('.viewer-close');

    await image.click();
    await expect(controls).toHaveClass(/chrome-hidden/);
    await expect(close).toHaveClass(/chrome-hidden/);
    await expect(controls).toHaveAttribute('inert', '');

    await image.click();
    await expect(controls).not.toHaveClass(/chrome-hidden/);
    await expect(close).not.toHaveClass(/chrome-hidden/);

    await openDetails(page);
    await image.click();
    await expect(sheet(page)).not.toHaveClass(/show/);
    await expect(controls).not.toHaveClass(/chrome-hidden/);

    await image.click();
    await expect(controls).toHaveClass(/chrome-hidden/);
    await page.locator('.viewer-main').click({ position: { x: 5, y: 5 } });
    await expect(controls).not.toHaveClass(/chrome-hidden/);
    await expect(viewer(page)).toHaveClass(/active/);
  });

  test('long-press then drag selects a range and shrinks on the way back', async ({ page }) => {
    const cards = page.locator('.photo-card');
    await cards.nth(0).scrollIntoViewIfNeeded();
    const first = await cards.nth(0).boundingBox();
    const second = await cards.nth(1).boundingBox();
    const third = await cards.nth(2).boundingBox();
    expect(first).not.toBeNull();
    expect(second).not.toBeNull();
    expect(third).not.toBeNull();

    const center = (box) => ({ x: box.x + box.width / 2, y: box.y + box.height / 2 });
    const anchor = center(first);
    const middle = center(second);
    const target = center(third);

    await dispatchTouchPointer(page, 'pointerdown', anchor.x, anchor.y);
    await expect(page.locator('#selection-bar')).toBeVisible();

    await dispatchTouchPointer(page, 'pointermove', target.x, target.y);
    await expect(page.locator('#selection-bar')).toContainText('3 selected');
    await expect(drawer(page)).toBeHidden();

    await dispatchTouchPointer(page, 'pointermove', middle.x, middle.y);
    await expect(page.locator('#selection-bar')).toContainText('2 selected');

    await dispatchTouchPointer(page, 'pointerup', middle.x, middle.y);
    await expect(page.locator('#selection-bar')).toContainText('2 selected');
  });

  test('horizontal swipe on the details header navigates and keeps the sheet open', async ({
    page,
  }) => {
    await openViewer(page);
    await openDetails(page);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);
    const firstTitle = await page.locator('#photo-title').textContent();
    const header = page.locator('.photo-info-header');
    let box = await header.boundingBox();
    expect(box).not.toBeNull();

    await TestHelpers.performSwipeOn(
      page,
      '.photo-info-header',
      box.x + box.width - 20,
      box.y + box.height / 2,
      box.x + 20,
      box.y + box.height / 2
    );

    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(firstHash);
    await expect(sheet(page)).toHaveClass(/show/);
    await expect.poll(() => page.locator('#photo-title').textContent()).not.toBe(firstTitle);

    box = await header.boundingBox();
    expect(box).not.toBeNull();
    await TestHelpers.performSwipeOn(
      page,
      '.photo-info-header',
      box.x + 20,
      box.y + box.height / 2,
      box.x + box.width - 20,
      box.y + box.height / 2
    );

    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(firstHash);
    await expect(sheet(page)).toHaveClass(/show/);
    await expect.poll(() => page.locator('#photo-title').textContent()).toBe(firstTitle);
  });
});
