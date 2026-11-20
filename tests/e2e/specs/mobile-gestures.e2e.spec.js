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
  // The details button is the only visible entry point on a 390 px phone;
  // it must sit outside the control strip's horizontal overflow.
  await expect(page.locator('.metadata-btn')).toBeInViewport();
  await page.locator('.metadata-btn').click();
  await expect(sheet(page)).toHaveClass(/show/);
  // CDP hit-testing uses the animated position; wait for the 0.3 s sheet
  // transition before measuring a header box.
  await expect(sheet(page)).toHaveCSS('transform', 'matrix(1, 0, 0, 1, 0, 0)');
  await expect(sheet(page)).toHaveCSS('opacity', '1');
}

async function createTouchDriver(page) {
  const client = await page.context().newCDPSession(page);
  await client.send('Emulation.setTouchEmulationEnabled', {
    enabled: true,
    maxTouchPoints: 1,
  });

  let clock = Date.now();
  const point = (x, y) => ({ x, y, radiusX: 5, radiusY: 5, force: 1 });
  const dispatch = async (type, touches, stepMs) => {
    clock += stepMs;
    await client.send('Input.dispatchTouchEvent', {
      type,
      touchPoints: touches,
      timestamp: clock / 1000,
    });
  };

  return {
    start: (x, y) => dispatch('touchStart', [point(x, y)], 0),
    move: (x, y, stepMs = 16) => dispatch('touchMove', [point(x, y)], stepMs),
    end: (stepMs = 16) => dispatch('touchEnd', [], stepMs),
    async tap(x, y) {
      await this.start(x, y);
      await this.end(40);
    },
    async doubleTap(x, y) {
      await this.tap(x, y);
      clock += 100;
      await this.tap(x, y);
    },
    async dispose() {
      await client.send('Emulation.setTouchEmulationEnabled', { enabled: false });
      await client.detach();
    },
  };
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
    const image = page.locator('#viewer-image');
    await expect(image).toHaveClass(/loaded/, { timeout: 20000 });
    const imageBox = await image.boundingBox();
    const sheetBox = await sheet(page).boundingBox();
    expect(imageBox).not.toBeNull();
    expect(sheetBox).not.toBeNull();
    // The sheet may not hide the media it describes: otherwise the header
    // swipe would move a photo that is entirely behind the panel.
    expect(imageBox.y + imageBox.height).toBeLessThanOrEqual(sheetBox.y + 1);

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
    // Use the known 2 s fixture. `test_video.mp4` can be a zero-duration
    // placeholder after the streaming specs, which makes seeking untestable.
    await page.getByRole('button', { name: 'test_video_moov_end.mp4', exact: true }).click();
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
    expect(Number.isFinite(duration) && duration > 15).toBe(true);
    // One native + one app seek would land at 0 s / 20 s. The bounds below
    // fail on that double application while allowing decoder seek jitter.
    const forwardStart = 5;
    const backStart = 15;

    const setVideoTime = async (position) => {
      await video.evaluate((element, value) => {
        element.pause();
        element.currentTime = value;
      }, position);
      await expect
        .poll(() => video.evaluate((element) => element.currentTime))
        .toBeGreaterThan(position - 0.5);
      await expect
        .poll(() => video.evaluate((element) => element.currentTime))
        .toBeLessThan(position + 0.5);
      // The property updates before the decoder has settled the seek; a tap
      // dispatched in that window can still hit the UA's own double-tap state.
      await page.waitForTimeout(200);
      await page.evaluate(() => {
        window.__seekFeedback = [];
      });
    };

    // The feedback is deliberately transient; record it as it enters the DOM
    // instead of racing the removal timer with a locator.
    await page.evaluate(() => {
      window.__seekFeedback = [];
      const main = document.querySelector('.viewer-main');
      const observer = new MutationObserver(() => {
        const text = document
          .querySelector('[data-testid="viewer-seek-feedback"]')
          ?.textContent?.trim();
        if (text && !window.__seekFeedback.includes(text)) window.__seekFeedback.push(text);
      });
      observer.observe(main, { childList: true, subtree: true });
    });

    const driver = await createTouchDriver(page);
    try {
      await setVideoTime(backStart);
      await driver.doubleTap(box.x + box.width * 0.1, y);
      await expect.poll(() => page.evaluate(() => window.__seekFeedback)).toContain('−10 s');
      await expect
        .poll(() => video.evaluate((element) => element.currentTime))
        .toBeGreaterThan(3.5);
      await expect.poll(() => video.evaluate((element) => element.currentTime)).toBeLessThan(6.5);

      await setVideoTime(forwardStart);
      await driver.doubleTap(box.x + box.width * 0.9, y);
      await expect.poll(() => page.evaluate(() => window.__seekFeedback)).toContain('+10 s');
      await expect
        .poll(() => video.evaluate((element) => element.currentTime))
        .toBeGreaterThan(13.5);
      await expect.poll(() => video.evaluate((element) => element.currentTime)).toBeLessThan(16.5);

      // The middle third must be inert for the app AND the UA: a native
      // double-tap seek there would move the clip without app feedback.
      await setVideoTime(forwardStart);
      await driver.doubleTap(box.x + box.width * 0.5, y);
      await page.waitForTimeout(450);
      await expect
        .poll(() => video.evaluate((element) => element.currentTime))
        .toBeGreaterThan(4.5);
      await expect.poll(() => video.evaluate((element) => element.currentTime)).toBeLessThan(5.5);
      expect(await page.evaluate(() => window.__seekFeedback)).toEqual([]);
    } finally {
      await driver.dispose();
    }
  });

  test('tap on the photo toggles chrome, with the details sheet closing first', async ({
    page,
  }) => {
    await openViewer(page);
    const image = page.locator('#viewer-image');
    await expect(image).toHaveClass(/loaded/, { timeout: 20000 });
    await expect(viewer(page)).toHaveCSS('opacity', '1');
    const controls = page.locator('.viewer-controls');
    const close = page.locator('.viewer-close');
    const imageBox = await image.boundingBox();
    const mainBox = await page.locator('.viewer-main').boundingBox();
    expect(imageBox).not.toBeNull();
    expect(mainBox).not.toBeNull();

    const driver = await createTouchDriver(page);
    const tapImage = () =>
      driver.tap(imageBox.x + imageBox.width / 2, imageBox.y + imageBox.height / 2);

    try {
      await tapImage();
      await expect(controls).toHaveClass(/chrome-hidden/);
      await expect(close).toHaveClass(/chrome-hidden/);
      await expect(controls).toHaveAttribute('inert', '');

      await tapImage();
      await expect(controls).not.toHaveClass(/chrome-hidden/);
      await expect(close).not.toHaveClass(/chrome-hidden/);

      await openDetails(page);
      const sheetBox = await sheet(page).boundingBox();
      expect(sheetBox).not.toBeNull();
      const visibleImageY = imageBox.y + (sheetBox.y - imageBox.y) / 2;
      await driver.tap(imageBox.x + imageBox.width / 2, visibleImageY);
      await expect(sheet(page)).not.toHaveClass(/show/);
      await expect(controls).not.toHaveClass(/chrome-hidden/);

      await tapImage();
      await expect(controls).toHaveClass(/chrome-hidden/);
      await driver.tap(mainBox.x + 5, mainBox.y + 5);
      await expect(controls).not.toHaveClass(/chrome-hidden/);
      await expect(viewer(page)).toHaveClass(/active/);
    } finally {
      await driver.dispose();
    }
  });

  test('long-press then drag selects a range and shrinks on the way back', async ({ page }) => {
    const cards = page.locator('.photo-card');
    await page.locator('.main-content').evaluate((element) => {
      element.scrollTop = 0;
    });
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

    const main = await page.locator('.main-content').boundingBox();
    expect(main).not.toBeNull();
    const driver = await createTouchDriver(page);

    try {
      await driver.start(anchor.x, anchor.y);
      await expect(page.locator('#selection-bar')).toBeVisible();

      await driver.move(target.x, target.y);
      await expect(page.locator('#selection-bar')).toContainText('3 selected');
      await expect(drawer(page)).toBeHidden();

      await driver.move(middle.x, middle.y);
      await expect(page.locator('#selection-bar')).toContainText('2 selected');

      // Hold at the bottom edge: the range selector itself must keep scrolling
      // while the finger is still, not merely hand the move to native scroll.
      const scrollTop = () =>
        page.locator('.main-content').evaluate((element) => element.scrollTop);
      await driver.move(middle.x, main.y + main.height - 10);
      await page.waitForTimeout(120);
      const firstScroll = await scrollTop();
      await page.waitForTimeout(300);
      const secondScroll = await scrollTop();
      expect(secondScroll).toBeGreaterThan(firstScroll);

      await driver.end();
      await expect(page.locator('#selection-bar')).toBeVisible();
    } finally {
      await driver.dispose();
    }
  });

  test('long-press while selection is active leaves touch scrolling alone', async ({ page }) => {
    await page.locator('[data-action="select-mode"]').click();
    await expect(page.locator('#selection-bar')).toContainText('0 selected');
    await page.locator('.main-content').evaluate((element) => {
      element.scrollTop = 0;
    });

    const first = await page.locator('.photo-card').nth(0).boundingBox();
    expect(first).not.toBeNull();
    const driver = await createTouchDriver(page);

    try {
      await driver.start(first.x + first.width / 2, first.y + first.height / 2);
      await page.waitForTimeout(650);

      // Register AFTER the long-press fired. A buggy range drag adds its
      // non-passive touchmove listener first, so this probe sees its
      // preventDefault; a no-op press leaves the event untouched.
      await page.evaluate(() => {
        window.__rangeTouchMoves = [];
        window.addEventListener(
          'touchmove',
          (event) => window.__rangeTouchMoves.push(event.defaultPrevented),
          { passive: true }
        );
      });

      await driver.move(first.x + first.width / 2, first.y + first.height + 120);
      await page.waitForTimeout(100);
      const prevented = await page.evaluate(() => window.__rangeTouchMoves);
      expect(prevented.length).toBeGreaterThan(0);
      expect(prevented).not.toContain(true);

      await driver.end();
      await expect(page.locator('#selection-bar')).toContainText('0 selected');
    } finally {
      await driver.dispose();
    }
  });

  test('horizontal swipe on the details header navigates and keeps the sheet open', async ({
    page,
  }) => {
    await openViewer(page);
    await openDetails(page);

    const firstHash = await TestHelpers.getCurrentPhotoHash(page);
    const firstTitle = await page.locator('#photo-title').textContent();
    const header = page.locator('.photo-info-header');
    const driver = await createTouchDriver(page);

    const swipeHeader = async (fromX, toX) => {
      const box = await header.boundingBox();
      expect(box).not.toBeNull();
      const y = box.y + box.height / 2;
      await driver.start(fromX(box), y);
      for (let step = 1; step <= 10; step += 1) {
        await driver.move(fromX(box) + ((toX(box) - fromX(box)) * step) / 10, y);
      }
      await driver.end();
    };

    try {
      await swipeHeader(
        (box) => box.x + box.width * 0.5,
        (box) => box.x + 20
      );

      await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(firstHash);
      await expect(sheet(page)).toHaveClass(/show/);
      await expect.poll(() => page.locator('#photo-title').textContent()).not.toBe(firstTitle);

      await swipeHeader(
        (box) => box.x + 20,
        (box) => box.x + box.width * 0.5
      );

      await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).toBe(firstHash);
      await expect(sheet(page)).toHaveClass(/show/);
      await expect.poll(() => page.locator('#photo-title').textContent()).toBe(firstTitle);
    } finally {
      await driver.dispose();
    }
  });
});
