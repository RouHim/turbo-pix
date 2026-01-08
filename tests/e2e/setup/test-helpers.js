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
      timeout: 10000,
    });
  }

  static async disableIndexingBanner(page) {
    await page.addStyleTag({
      content: '#indexing-banner { pointer-events: none !important; }',
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

  static async waitForServer(baseURL, maxRetries = 30) {
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
}
