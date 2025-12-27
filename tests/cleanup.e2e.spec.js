import { test, expect } from '@playwright/test';

test.describe('Cleanup Functionality', () => {
  test.beforeEach(async ({ page }) => {
      page.on('console', msg => console.log(`PAGE LOG: ${msg.text()}`));
      page.on('pageerror', exception => console.log(`PAGE ERROR: ${exception}`));
      page.on('requestfailed', request => console.log(`REQUEST FAILED: ${request.url()} - ${request.failure().errorText}`));
      page.on('response', response => {
          if (response.status() === 404) {
              console.log(`404 RESPONSE: ${response.url()}`);
          }
      });
  });

  test('should display cleanup candidates and allow keeping them', async ({ page }) => {
    // 1. Mock the API to ensure predictable state for UI testing
    const mockCandidates = {
      candidates: [
        {
          photo_hash: 'mock_hash_1',
          file_path: '/photos/mock_screenshot.png',
          reason: 'screenshot',
          score: 0.95
        },
        {
            photo_hash: 'mock_hash_2',
            file_path: '/photos/mock_meme.jpg',
            reason: 'meme',
            score: 0.88
        }
      ]
    };

    // Intercept the cleanup candidates request
    await page.route('**/api/cleanup/candidates', async route => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockCandidates),
      });
    });

    // Mock the Keep (Delete candidate) API
    // The frontend calls DELETE /api/cleanup/candidates/:hash
    await page.route('**/api/cleanup/candidates/mock_hash_1', async route => {
        await route.fulfill({ status: 200, body: '{}' });
    });
    
    // Mock thumbnail requests to avoid 404s and layout issues
    await page.route('**/api/photos/**/thumbnail*', async route => {
        await route.fulfill({ 
            status: 200, 
            contentType: 'image/png',
            body: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==', 'base64') // 1x1 red pixel
        });
    });

    // Mock full image request to prevent 404s if viewer opens accidentally
    await page.route('**/api/photos/*/file', async route => {
        await route.fulfill({ status: 200, contentType: 'image/png', body: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==', 'base64') });
    });

    // 2. Navigate to the app
    // We assume the app is running on localhost:18473 as per AGENTS.md
    await page.goto('http://localhost:18473');

    // 3. Click the Cleanup navigation button
    // Selector based on data-view="cleanup" from index.html
    await page.click('button[data-view="cleanup"]');

    // 4. Verify candidates are displayed
    // "Found 2 candidates" text
    await expect(page.locator('.cleanup-summary')).toContainText('Found 2 candidates');
    
    // Check for specific candidate elements (photo cards)
    const keepButtons = page.locator('[data-action="keep"]');
    await expect(keepButtons).toHaveCount(2);

    // Verify reason badges
    await expect(page.locator('.cleanup-badge').first()).toContainText('screenshot');

    // 5. Test "Keep" functionality
    // Hover the first card to make buttons visible (if they are hover-only)
    // The button is inside .photo-card
    const firstCard = page.locator('.photo-card').first();
    await firstCard.hover();
    
    // Click "Keep" on the first one
    // Use force: true because sometimes animations/overlays interfere, but ideally valid click.
    await keepButtons.first().click();

    // Verify viewer did NOT open
    await expect(page.locator('#photo-viewer')).toBeHidden();

    // Verify toast message "Kept"
    await expect(page.locator('.toast')).toContainText('Kept');
    
    // Verify it is removed from the DOM (or count decreases)
    await expect(keepButtons).toHaveCount(1);
  });
  
  test('should show empty state when no candidates found', async ({ page }) => {
      await page.route('**/api/cleanup/candidates', async route => {
          await route.fulfill({
              status: 200,
              contentType: 'application/json',
              body: JSON.stringify({ candidates: [] }),
          });
      });

      await page.goto('http://localhost:18473');
      await page.click('button[data-view="cleanup"]');
      
      // Based on cleanup.js L51: <div class="no-photos">No cleanup candidates found. Your library is clean!</div>
      await expect(page.locator('.no-photos')).toContainText('No cleanup candidates found');
  });
});
