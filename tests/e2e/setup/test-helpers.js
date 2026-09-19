import { execSync } from 'child_process';

/** The E2E server's SQLite file, relative to the runner's cwd (repo root). */
const TEST_DB_PATH = 'test-e2e-data/database/turbo-pix.db';

export class TestHelpers {
  /**
   * 1×1 PNG — a valid image response for stubbed tile requests. The committed
   * bytes decode to a single opaque pixel, RGBA (19, 87, 138, 255): the map
   * specs need a decodable image, never a transparent one.
   */
  static TINY_PNG = Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mMQDu/6DwADaQH0rwEuVwAAAABJRU5ErkJggg==',
    'base64'
  );

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

  /**
   * Answers every slippy-map tile request with TINY_PNG so map specs run
   * without network access. The pathname shape (`/{z}/{x}/{y}.png`) matches the
   * default OSM endpoint and any custom TURBO_PIX_TILE_URL template.
   */
  static async stubMapTiles(page) {
    await page.route(
      (url) => /\/\d+\/\d+\/\d+\.png$/.test(url.pathname),
      (route) =>
        route.fulfill({ status: 200, contentType: 'image/png', body: TestHelpers.TINY_PNG })
    );
  }

  /**
   * Writes GPS coordinates through the metadata endpoint, so a spec can place a
   * photo at a known point (the seeded EXIF coordinates are not per-test).
   */
  static async setPhotoCoordinates(page, hash, latitude, longitude) {
    const response = await page.request.patch(`/api/photos/${hash}/metadata`, {
      data: { latitude, longitude },
    });
    if (!response.ok()) {
      throw new Error(`PATCH metadata for ${hash} failed: ${response.status()}`);
    }
  }

  /**
   * Writes coordinates straight into the indexed row — videos cannot take EXIF
   * writes, and a direct UPDATE needs no re-index. Every spec shares one
   * server and one database, so a spec that seeds a location must restore it
   * with clearPhotoLocationInDb before it finishes.
   */
  static setPhotoLocationInDb(fileName, latitude, longitude) {
    const metadata = JSON.stringify({ location: { latitude, longitude } });
    execSync(
      `sqlite3 "${TEST_DB_PATH}" "UPDATE photos SET metadata = json_set(metadata, '$.location.latitude', ${latitude}, '$.location.longitude', ${longitude}) WHERE filename = '${fileName}'"`,
      { stdio: 'pipe' }
    );
    return metadata;
  }

  /**
   * Reverts setPhotoLocationInDb: the row keeps a JSON null location, which the
   * map's coordinate validation (lib/map.js) and every query that reads
   * `metadata.location` treat as "no location".
   */
  static clearPhotoLocationInDb(fileName) {
    execSync(
      `sqlite3 "${TEST_DB_PATH}" "UPDATE photos SET metadata = json_set(metadata, '$.location.latitude', null, '$.location.longitude', null) WHERE filename = '${fileName}'"`,
      { stdio: 'pipe' }
    );
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
    const toYearRaw = url.searchParams.get('to_year');
    const toMonthRaw = url.searchParams.get('to_month');
    return {
      view: pathname,
      query: url.searchParams.get('q') || null,
      sort: url.searchParams.get('sort') || null,
      year: yearRaw !== null ? parseInt(yearRaw, 10) : null,
      month: monthRaw !== null ? parseInt(monthRaw, 10) : null,
      toYear: toYearRaw !== null ? parseInt(toYearRaw, 10) : null,
      toMonth: toMonthRaw !== null ? parseInt(toMonthRaw, 10) : null,
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
