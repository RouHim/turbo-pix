import { execSync } from 'child_process';
import { readdir, rm } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

/** The E2E server's SQLite file, relative to the runner's cwd (repo root). */
const TEST_DB_PATH = 'test-e2e-data/database/turbo-pix.db';

// Per-run test data directory; must match `tests/e2e/setup/global-setup.js`.
const TEST_DATA_DIR = 'test-e2e-data';

// Conversion-cache namespaces; must match the server's TRANSCODE_NAMESPACES.
const CACHE_NAMESPACES = ['transcoded', 'copied', 'remux'];

// A cache entry is either a finished artifact (`{hash}_{size}_{mtime}.mp4`) or
// a conversion still in flight (`.mp4.tmp` of a whole-file job, `.{pid}.{seq}.tmp`
// of a remux fill). Existence of the finished name is what every cache-hit
// check looks at, so only those may be removed.
const FINISHED_ARTIFACT = /^\d+_\d+\.mp4$/;

// How long the cache must look quiet — no `InProgress` status, no temp, no
// artifact — before the wipe may call it cold.
const CACHE_QUIET_MS = 1500;
const CACHE_POLL_MS = 250;
// Whole-run budget for the settle. Exhausting it fails the test: a conversion
// that never settles must not be papered over with a warm cache.
const CACHE_SETTLE_TIMEOUT_MS = 30_000;

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
    // `PRAGMA busy_timeout=5000` like every sibling seeding site: the sqlite3
    // CLI waits 0 ms by default, so a concurrent server write fails the UPDATE
    // and execSync throws.
    const sql =
      `PRAGMA busy_timeout=5000; ` +
      `UPDATE photos SET metadata = json_set(metadata, '$.location.latitude', ${latitude}, '$.location.longitude', ${longitude}) WHERE filename = '${fileName}'`;
    execSync(`sqlite3 "${TEST_DB_PATH}" "${sql}"`, { stdio: 'pipe' });
    return metadata;
  }

  /**
   * Reverts setPhotoLocationInDb: the row keeps a JSON null location, which the
   * map's coordinate validation (lib/map.js) and every query that reads
   * `metadata.location` treat as "no location".
   */
  static clearPhotoLocationInDb(fileName) {
    const sql =
      `PRAGMA busy_timeout=5000; ` +
      `UPDATE photos SET metadata = json_set(metadata, '$.location.latitude', null, '$.location.longitude', null) WHERE filename = '${fileName}'`;
    execSync(`sqlite3 "${TEST_DB_PATH}" "${sql}"`, { stdio: 'pipe' });
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

  /**
   * Clear the finished conversion artifacts of `hash`, and do not return until
   * the cache is COLD: nothing left for the server to serve, and no conversion
   * still running that could publish one moments later.
   *
   * The transcode cache lasts for the whole run (global-setup wipes it once)
   * and a full playthrough fills it, so a fixture an earlier test played
   * through is legitimately served as `direct`/`cached` from then on. Tests
   * asserting the *cold* first-play behaviour (a `stream`/`remux` decision, a
   * visible conversion notice) clear it first — and that assertion only means
   * something if nothing survives the call that the server can serve from, so
   * this waits out conversions that are already running instead of racing
   * them.
   *
   * Every cache namespace matters: the universal H.264 re-encodes live in
   * `transcoded/`, video copies (source video codec, converted audio) in
   * `copied/` and the lossless faststart sidecars of remux playthroughs in
   * `remux/`. Only a *missing* subdirectory is benign (nothing was cached
   * yet): a namespace that cannot be listed throws, because a cache that did
   * not answer cannot be read as empty (see `conversionCacheEntries`).
   *
   * Only the versioned artifact itself (`{hash}_{size}_{mtime}.mp4`) is
   * removed — never a `*.tmp`. A conversion writes its temp first and renames
   * it into place on success, and the temps carry the same `{hash}_` prefix:
   * the deterministic `{hash}_{size}_{mtime}.mp4.tmp` of a whole-file job and
   * the unique `{hash}_{size}_{mtime}.{pid}.{seq}.tmp` a remux fill writes.
   * Deleting one out from under its writer makes the job's final rename fail,
   * which poisons the retry: the conversion settles as failed, the next
   * whole-file request answers `PreviouslyFailedOrTimedOut` (the original with
   * status 200 instead of the expected 202) and the `video/status` poll can
   * never reach `Completed`. Leaving temps behind is safe: every cache-hit
   * check looks at the finished path, and the writer cleans up after itself.
   *
   * Outlasting a conversion that is already running is what keeps the callers'
   * cold premise true across Playwright retries, which re-run this wipe while
   * the previous attempt's conversion — or the background fill its stream
   * started — is very possibly still alive. Three things can overtake a wipe
   * that just deletes and returns, and each is checked before the cache is
   * called cold:
   * - a whole-file job or fill that already claimed the hash renames its
   *   artifact moments later. `claim_transcode` registers `InProgress` before
   *   the job runs and the artifact is renamed into place before the status
   *   settles, so waiting for the status to leave `InProgress` guarantees the
   *   delete below sees that artifact;
   * - a remux fill registers no status at all, but it holds a `{hash}_…tmp` in
   *   `remux/` for the whole copy: a temp is a conversion in flight, so the
   *   wipe keeps waiting for it;
   * - a publish that lands while this helper is running. Deleting an artifact
   *   is activity, so after it the cache must stay quiet — no status, no temp,
   *   no artifact — for one settle window (`CACHE_QUIET_MS`, a second or two)
   *   before the caller is told it is cold.
   *
   * The wait is bounded by `CACHE_SETTLE_TIMEOUT_MS`, and exhausting it throws
   * instead of returning: a caller left to assert a cache that is not cold
   * fails later with a warm-cache symptom (a `direct`/`cached` decision, a
   * missing conversion notice) that points nowhere near the conversion still
   * running.
   *
   * For the same reason a `/video/status` probe that does not answer throws
   * (`conversionState`) instead of counting as "no conversion in flight": the
   * probe is the only thing that can rule out a job that already claimed the
   * hash, so a probe that never produced a response leaves the premise
   * unestablished rather than true.
   *
   * The listing of the cache namespaces is the wipe's other observation, and
   * it is held to the same rule: a namespace that could not be listed (EACCES,
   * EIO, ENOTDIR when a namespace path is not a directory) throws instead of
   * counting as empty. Reading a failed listing as "nothing was cached"
   * swallows a `{hash}_…tmp` that is still in flight — the exact warm cache
   * this helper exists to rule out.
   */
  static async clearCachedConversions(page, hash) {
    const deadline = Date.now() + CACHE_SETTLE_TIMEOUT_MS;
    let quietSince = Date.now();
    for (;;) {
      // The per-hash conversion status. A whole-file job (user-facing or the
      // background fill of a finished stream run) claims the hash before it
      // starts and settles the status only after its artifact is renamed into
      // place; an absent entry (404) and every settled state mean no such job
      // can publish behind this wipe's back. A probe that failed says neither,
      // and throws.
      const state = await this.conversionState(page, hash);

      const entries = await this.conversionCacheEntries(hash);
      const finished = entries.filter((entry) => entry.finished);
      const inFlight = entries.filter((entry) => !entry.finished);
      await Promise.all(
        finished.map((entry) => rm(path.join(entry.dir, entry.name), { force: true }))
      );

      const busy = state === 'InProgress' || inFlight.length > 0 || finished.length > 0;
      quietSince = busy ? Date.now() : quietSince;
      if (!busy && Date.now() - quietSince >= CACHE_QUIET_MS) return;
      if (Date.now() >= deadline) {
        throw new Error(
          `clearCachedConversions(${hash}): the conversion cache did not go cold within ` +
            `${CACHE_SETTLE_TIMEOUT_MS} ms — status ${state ?? 'missing'}, ` +
            `${inFlight.length} in-flight temp file(s). Refusing to let the caller assert ` +
            `a cold cache that is not cold.`
        );
      }
      await delay(CACHE_POLL_MS);
    }
  }

  /**
   * The conversion state the server reports for `hash`, or `null` when it has
   * no entry for it — the endpoint's 404, its only non-ok answer (a status
   * entry exists for every claimed job until it settles).
   *
   * A probe that fails is NOT `null`. A request that never produced a response
   * (connection refused, reset, timeout) or a response the server could not
   * answer the probe from (a 5xx) leaves "no conversion is publishing" unknown,
   * and reading it as idle would let a job that already claimed the hash
   * publish behind the caller's wipe. Throws instead, naming the probe, so the
   * failure is not mistaken for a cold cache.
   */
  static async conversionState(page, hash) {
    const url = `/api/photos/${hash}/video/status`;
    const response = await page.request.get(url).catch((cause) => {
      throw new Error(
        `conversionState(${hash}): the ${url} probe never produced a response ` +
          `(${cause.message}) — refusing to call the conversion cache cold on a probe ` +
          `that did not answer.`,
        { cause }
      );
    });
    if (response.ok()) return (await response.json()).state;
    const status = response.status();
    if (status === 404) return null;
    throw new Error(
      `conversionState(${hash}): the ${url} probe answered ${status} — refusing to call ` +
        `the conversion cache cold on a probe that did not answer.`
    );
  }

  /**
   * Every `{hash}_…` entry the conversion cache holds for `hash`, each with the
   * directory it lives in and whether it is a finished artifact (`false` means
   * a conversion still in flight).
   *
   * Only a missing namespace (`ENOENT` — nothing was cached yet) is read as
   * empty. Any other listing failure throws: the wipe uses these entries as its
   * evidence that no artifact and no in-flight temp survive it, so a namespace
   * that did not list cannot be reported as empty without letting an artifact
   * or a running conversion slip past the caller's cold-cache premise.
   */
  static async conversionCacheEntries(hash) {
    const perNamespace = await Promise.all(
      CACHE_NAMESPACES.map(async (namespace) => {
        const dir = path.join(TEST_DATA_DIR, 'transcode-cache', namespace);
        const entries = await readdir(dir).catch((cause) => {
          if (cause.code === 'ENOENT') return [];
          throw new Error(
            `conversionCacheEntries(${hash}): the ${dir} cache namespace could not be ` +
              `listed (${cause.code ?? cause.message}) — refusing to read a namespace ` +
              `that did not answer as empty.`,
            { cause }
          );
        });
        return entries
          .filter((name) => name.startsWith(`${hash}_`))
          .map((name) => ({
            dir,
            name,
            finished: FINISHED_ARTIFACT.test(name.slice(hash.length + 1)),
          }));
      })
    );
    return perNamespace.flat();
  }
}
