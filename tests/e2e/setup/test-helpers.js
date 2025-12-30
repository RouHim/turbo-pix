import { expect } from '@playwright/test';
import path from 'path';

/**
 * Shared test utilities for E2E tests
 */
export class TestHelpers {
  // Selectors - centralized for easy maintenance
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
    toast: '.toast',
    photoGrid: '.photo-grid',
    loadingIndicator: '#loading-indicator',
    noPhotos: '.error-state, .empty-state',
    viewTitle: '#current-view-title',
    menuBtn: '.menu-btn',
    sidebar: '.sidebar',
  };

  /**
   * Navigate to a specific view by clicking the navigation button
   */
  static async navigateToView(page, viewName) {
    const navButton = page.locator(this.selectors.navItem(viewName));
    await navButton.click();
    // Wait for the view to be active
    await this.verifyActiveView(page, viewName);
  }

  /**
   * Verify that a specific view is currently active
   */
  static async verifyActiveView(page, viewName) {
    const navButton = page.locator(this.selectors.navItem(viewName));
    await expect(navButton).toHaveClass(/active/);
  }

  /**
   * Get all photo cards currently visible in the grid
   */
  static async getPhotoCards(page) {
    return page.locator(this.selectors.photoCardAny).all();
  }

  /**
   * Get a specific photo card by hash
   */
  static getPhotoCardByHash(page, hash) {
    return page.locator(this.selectors.photoCard(hash));
  }

  /**
   * Wait for photos to load (skeleton disappears, photos appear)
   */
  static async waitForPhotosToLoad(page) {
    // Wait for loading skeleton to disappear
    await page
      .waitForSelector('.loading-skeleton', {
        state: 'hidden',
        timeout: 10000,
      })
      .catch(() => {
        // Skeleton might not be present if photos load quickly
      });

    // Wait for either photo grid with photos or empty state
    await Promise.race([
      page.waitForSelector(this.selectors.photoCardAny, { timeout: 10000 }),
      page.waitForSelector(this.selectors.noPhotos, { timeout: 10000 }),
    ]);
  }

  /**
   * Open the photo viewer for a specific photo hash
   */
  static async openViewer(page, photoHash) {
    const photoCard = this.getPhotoCardByHash(page, photoHash);
    await photoCard.click();
    await this.verifyViewerOpen(page);
  }

  /**
   * Close the photo viewer
   */
  static async closeViewer(page) {
    await page.keyboard.press('Escape');
    await expect(page.locator(this.selectors.viewer)).not.toHaveClass(/active/);
  }

  /**
   * Verify that the photo viewer is open and active
   */
  static async verifyViewerOpen(page) {
    const viewer = page.locator(this.selectors.viewer);
    await expect(viewer).toHaveClass(/active/);
  }

  /**
   * Wait for a toast message with specific text
   */
  static async waitForToast(page, expectedText) {
    const toast = page.locator(this.selectors.toast);
    await expect(toast).toBeVisible({ timeout: 5000 });
    await expect(toast).toContainText(expectedText);
  }

  /**
   * Set viewport to mobile size
   */
  static async setMobileViewport(page) {
    await page.setViewportSize({ width: 375, height: 667 }); // iPhone SE
  }

  /**
   * Set viewport to desktop size
   */
  static async setDesktopViewport(page) {
    await page.setViewportSize({ width: 1920, height: 1080 });
  }

  /**
   * Take a debug screenshot
   */
  static async takeDebugScreenshot(page, name) {
    const screenshotDir = path.join(process.cwd(), 'tests', 'e2e', 'screenshots');
    const timestamp = new Date().toISOString().replace(/:/g, '-');
    const filename = `${name}-${timestamp}.png`;
    await page.screenshot({
      path: path.join(screenshotDir, filename),
      fullPage: true,
    });
  }

  /**
   * Setup console monitoring for debugging
   */
  static setupConsoleMonitoring(page) {
    page.on('console', (msg) => {
      const type = msg.type();
      if (type === 'error' || type === 'warning') {
        console.log(`[PAGE ${type.toUpperCase()}] ${msg.text()}`);
      }
    });

    page.on('pageerror', (exception) => {
      console.error(`[PAGE ERROR] ${exception}`);
    });

    page.on('requestfailed', (request) => {
      console.error(
        `[REQUEST FAILED] ${request.url()} - ${request.failure()?.errorText || 'unknown error'}`
      );
    });

    page.on('response', (response) => {
      if (response.status() === 404) {
        console.warn(`[404 RESPONSE] ${response.url()}`);
      } else if (response.status() >= 500) {
        console.error(`[${response.status()} RESPONSE] ${response.url()}`);
      }
    });
  }

  /**
   * Wait for a specific API endpoint to be called
   */
  static async waitForApiCall(page, endpoint) {
    return page.waitForResponse((response) => response.url().includes(endpoint), {
      timeout: 10000,
    });
  }

  /**
   * Wait for server to be ready
   */
  static async waitForServer(baseURL = 'http://localhost:18473', maxRetries = 10) {
    for (let i = 0; i < maxRetries; i++) {
      try {
        const response = await fetch(`${baseURL}/health`);
        if (response.ok) {
          return true;
        }
      } catch (e) {
        // Not ready yet
      }
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    throw new Error('Server not ready within timeout');
  }

  /**
   * Get the current photo hash from viewer URL
   */
  static async getCurrentPhotoHash(page) {
    const url = new URL(page.url());
    return url.searchParams.get('photo');
  }

  /**
   * Check if element exists (without throwing)
   */
  static async elementExists(page, selector) {
    try {
      const element = await page.locator(selector).first();
      return (await element.count()) > 0;
    } catch {
      return false;
    }
  }

  /**
   * Scroll to bottom of page (for infinite scroll)
   */
  static async scrollToBottom(page) {
    await page.evaluate(() => {
      window.scrollTo(0, document.body.scrollHeight);
    });
  }

  /**
   * Wait for network idle
   */
  static async waitForNetworkIdle(page) {
    await page.waitForLoadState('networkidle', { timeout: 10000 });
  }
}
