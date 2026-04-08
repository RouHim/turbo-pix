import { writeFile } from 'fs/promises';

export class TestHelpers {
  static selectors = {
    navItem: (view) => `button[data-view="${view}"]`,
    photoCard: (hash) => `[data-photo-id="${hash}"]`,
    photoCardAny: '.photo-card',
    action: (action) => `[data-action="${action}"]`,
    viewer: '#photo-viewer',
    viewerImage: '#viewer-image',
    viewerVideo: '#viewer-video',
    searchInput: '#search-input',
    searchBtn: '#search-btn',
    sortSelect: '#sort-select',
    photoGrid: '.photo-grid',
    viewTitle: '#current-view-title',
    menuBtn: '.menu-btn',
    sidebar: '.sidebar',
    favoriteBtn: '.favorite-btn',
    closeViewerBtn: '.close-viewer',
    acceptCollageBtn: '[data-action="accept-collage"]',
  };

  static async navigateToView(page, viewName) {
    const selector = this.selectors.navItem(viewName);
    await page.waitForSelector(selector, { state: 'visible' });
    await page.click(selector);
    await this.verifyActiveView(page, viewName);
  }

  static async goto(page, path = '/') {
    await page.goto(path, { waitUntil: 'domcontentloaded' });
  }

  static async verifyActiveView(page, viewName) {
    const selector = this.selectors.navItem(viewName);
    await page.waitForSelector(`${selector}.active`, { state: 'attached' });
  }

  static async waitForPhotosToLoad(page) {
    await this.disableIndexingBanner(page);
    await page.waitForSelector(this.selectors.photoCardAny, {
      state: 'attached',
      timeout: 30000,
    });
  }

  static async waitForSearchReady(page) {
    await this.disableIndexingBanner(page);
    await page.waitForSelector(this.selectors.searchInput, {
      state: 'visible',
      timeout: 20000,
    });
  }

  static async disableIndexingBanner(page) {
    await page.addStyleTag({
      content: '[data-phase-ring] { pointer-events: none !important; }',
    });
  }

  static async getPhotoCards(page) {
    return await page.locator(this.selectors.photoCardAny).all();
  }

  static async getPhotoCardByHash(page, hash) {
    const selector = this.selectors.photoCard(hash);
    return await page.locator(selector).first();
  }

  static async openViewer(page, photoHash) {
    const card = await this.getPhotoCardByHash(page, photoHash);
    if (!card) {
      throw new Error(`Photo card with hash ${photoHash} not found`);
    }
    await card.click();
    await this.verifyViewerOpen(page);
  }

  static async closeViewer(page) {
    await page.keyboard.press('Escape');
    await page.waitForSelector(this.selectors.viewer, { state: 'hidden' });
  }

  static async verifyViewerOpen(page) {
    await page.waitForSelector(`${this.selectors.viewer}.active`, {
      state: 'attached',
    });
  }

  static async getCurrentPhotoHash(page) {
    const url = new URL(page.url());
    return url.searchParams.get('photo');
  }

  static async setMobileViewport(page) {
    await page.setViewportSize({ width: 375, height: 667 });
  }

  static async setDesktopViewport(page) {
    await page.setViewportSize({ width: 1920, height: 1080 });
  }

  static setupConsoleMonitoring(page) {
    page.on('console', (msg) => {
      const type = msg.type();
      if (type === 'error' || type === 'warning') {
        console.log(`[browser ${type}]`, msg.text());
      }
    });

    page.on('pageerror', (error) => {
      console.error('[browser error]', error.message);
    });

    page.on('requestfailed', (request) => {
      console.error('[request failed]', request.url(), request.failure()?.errorText);
    });

    page.on('response', (response) => {
      if (response.status() >= 400) {
        console.error(`[HTTP ${response.status()}]`, response.url());
      }
    });
  }

  static async waitForApiCall(page, endpoint) {
    return await page.waitForResponse((response) => response.url().includes(endpoint));
  }

  static async takeDebugScreenshot(page, name) {
    const timestamp = Date.now();
    const filename = `test-results/debug-${name}-${timestamp}.png`;
    await writeFile(filename, await page.screenshot({ fullPage: true }));
    console.log(`Screenshot saved: ${filename}`);
    return filename;
  }

  static async waitForNetworkIdle(page) {
    await page.waitForLoadState('networkidle', { timeout: 5000 });
  }

  static async scrollToBottom(page) {
    await page.evaluate(() => {
      window.scrollTo(0, document.body.scrollHeight);
    });
  }

  static async elementExists(page, selector) {
    return (await page.locator(selector).count()) > 0;
  }

  static async performSearch(page, searchTerm) {
    await page.fill(this.selectors.searchInput, searchTerm);
    await page.click(this.selectors.searchBtn);
  }

  static async waitForSearchParam(page, expectedQuery) {
    await page.waitForFunction((query) => {
      const url = new URL(window.location.href);
      return url.searchParams.get('q') === query;
    }, expectedQuery);
  }

  static async clearSearch(page) {
    await page.fill(this.selectors.searchInput, '');
    await page.keyboard.press('Escape');
  }

  static async waitForServer(baseURL, maxRetries = 60) {
    for (let i = 0; i < maxRetries; i++) {
      try {
        const response = await fetch(`${baseURL}/health`);
        if (response.ok) {
          return true;
        }
      } catch {
        // Continue waiting
      }
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    throw new Error('Server not ready after max retries');
  }

  static async addToFavorites(page, photoHash) {
    const card = await this.getPhotoCardByHash(page, photoHash);
    const favoriteBtn = await card.locator(this.selectors.favoriteBtn).first();
    await favoriteBtn.click();
  }

  static async removeFromFavorites(page, photoHash) {
    const card = await this.getPhotoCardByHash(page, photoHash);
    const favoriteBtn = await card.locator(this.selectors.favoriteBtn).first();
    await favoriteBtn.click();
  }

  static getUrlState(page) {
    const url = new URL(page.url());
    const pathname = url.pathname.replace(/^\//, '') || 'all';
    const yearRaw = url.searchParams.get('year');
    const monthRaw = url.searchParams.get('month');
    return {
      view: pathname,
      query: url.searchParams.get('q') || null,
      sort: url.searchParams.get('sort') || null,
      year: yearRaw !== null ? parseInt(yearRaw, 10) : null,
      month: monthRaw !== null ? parseInt(monthRaw, 10) : null,
      photo: url.searchParams.get('photo') || null,
    };
  }

  static async waitForUrlParam(page, param, value) {
    await page.waitForFunction(
      ({ param: p, value: v }) => {
        const url = new URL(window.location.href);
        const current = url.searchParams.get(p);
        return v === null ? current === null : current === v;
      },
      { param, value }
    );
  }

  static async assertUrlState(page, expected) {
    const { expect } = await import('@playwright/test');
    const state = this.getUrlState(page);
    for (const [key, value] of Object.entries(expected)) {
      expect(state[key], `URL state mismatch for "${key}"`).toBe(value);
    }
  }

  static async swipeLeft(page, options = {}) {
    const viewport = page.viewportSize();
    const startX = options.startX ?? Math.floor(viewport.width / 2);
    const startY = options.startY ?? Math.floor(viewport.height / 2);
    const distance = options.distance ?? Math.floor(viewport.width * 0.4);
    await this.performSwipe(page, startX, startY, startX - distance, startY, options.stepDelay);
  }

  static async swipeRight(page, options = {}) {
    const viewport = page.viewportSize();
    const startX = options.startX ?? Math.floor(viewport.width / 2);
    const startY = options.startY ?? Math.floor(viewport.height / 2);
    const distance = options.distance ?? Math.floor(viewport.width * 0.4);
    await this.performSwipe(page, startX, startY, startX + distance, startY, options.stepDelay);
  }

  static async swipeDown(page, options = {}) {
    const viewport = page.viewportSize();
    const startX = options.startX ?? Math.floor(viewport.width / 2);
    const startY = options.startY ?? Math.floor(viewport.height / 2);
    const distance = options.distance ?? 200;
    await this.performSwipe(page, startX, startY, startX, startY + distance, options.stepDelay);
  }

  static async doubleTap(page, x, y) {
    await page.evaluate(
      ({ tx, ty }) => {
        const target = document.querySelector('.viewer-main') || document.body;

        const createTouch = (id, cx, cy) =>
          new Touch({
            identifier: id,
            target,
            clientX: cx,
            clientY: cy,
            pageX: cx,
            pageY: cy,
            radiusX: 2,
            radiusY: 2,
            rotationAngle: 0,
            force: 0.5,
          });

        const tap = (id) => {
          const touch = createTouch(id, tx, ty);
          target.dispatchEvent(
            new TouchEvent('touchstart', {
              bubbles: true,
              cancelable: true,
              touches: [touch],
              changedTouches: [touch],
            })
          );
          target.dispatchEvent(
            new TouchEvent('touchend', {
              bubbles: true,
              cancelable: true,
              touches: [],
              changedTouches: [touch],
            })
          );
        };

        tap(1);
        setTimeout(() => tap(2), 50);
      },
      { tx: x, ty: y }
    );
    // Wait for the setTimeout + gesture processing
    await page.waitForTimeout(200);
  }

  // Target .viewer-main so events bubble: .viewer-main (SwipeableViewer enablePan) → #photo-viewer (GestureManager)
  // Uses setTimeout spacing between touchmove events so GestureManager
  // computes realistic velocity via Date.now() deltas.
  static async performSwipe(page, startX, startY, endX, endY, stepDelay = 16) {
    await page.evaluate(
      ({ sx, sy, ex, ey, delay }) =>
        new Promise((resolve) => {
          const target = document.querySelector('.viewer-main') || document.body;

          const createTouch = (id, x, y) =>
            new Touch({
              identifier: id,
              target,
              clientX: x,
              clientY: y,
              pageX: x,
              pageY: y,
              radiusX: 2,
              radiusY: 2,
              rotationAngle: 0,
              force: 0.5,
            });

          const dispatch = (type, touches, changed) =>
            target.dispatchEvent(
              new TouchEvent(type, {
                bubbles: true,
                cancelable: true,
                touches,
                changedTouches: changed,
              })
            );

          const startTouch = createTouch(1, sx, sy);
          dispatch('touchstart', [startTouch], [startTouch]);

          const steps = 10;
          let step = 1;

          const nextStep = () => {
            if (step <= steps) {
              const x = sx + ((ex - sx) * step) / steps;
              const y = sy + ((ey - sy) * step) / steps;
              const moveTouch = createTouch(1, x, y);
              dispatch('touchmove', [moveTouch], [moveTouch]);
              step++;
              setTimeout(nextStep, delay);
            } else {
              const endTouch = createTouch(1, ex, ey);
              dispatch('touchend', [], [endTouch]);
              resolve();
            }
          };

          setTimeout(nextStep, delay);
        }),
      { sx: startX, sy: startY, ex: endX, ey: endY, delay: stepDelay }
    );
  }
}
