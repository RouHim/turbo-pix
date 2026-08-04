import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { TestHelpers } from '../setup/test-helpers.js';

// NOTE: `focus-appearance` is not a rule in axe-core 4.12.x (it was removed
// upstream in an earlier major version) — the focus ring is instead verified
// by the 4px solid box-shadow on `:focus-visible` (SC-004) plus manual
// inspection.
const AXE_RULES = [
  'color-contrast',
  'target-size',
  'aria-valid-attr-value',
  'aria-prohibited-attr',
];

test('timeline should have no axe violations on desktop', async ({ page }) => {
  TestHelpers.setupConsoleMonitoring(page);
  await TestHelpers.goto(page);
  await TestHelpers.waitForPhotosToLoad(page);

  const density = await page.evaluate(() =>
    fetch('/api/photos/timeline')
      .then((response) => response.json())
      .then((data) => data.density || [])
  );
  test.skip(density.length === 0, 'Timeline needs at least one month bucket');

  await expect(page.locator('.timeline-input')).toHaveCount(1);

  const results = await new AxeBuilder({ page })
    .include('.timeline-container')
    .withRules(AXE_RULES)
    .analyze();
  expect(results.violations).toEqual([]);
});

test('timeline should have no axe violations on mobile', async ({ page }) => {
  TestHelpers.setupConsoleMonitoring(page);
  await TestHelpers.setMobileViewport(page);
  await TestHelpers.goto(page);
  await TestHelpers.waitForPhotosToLoad(page);

  const density = await page.evaluate(() =>
    fetch('/api/photos/timeline')
      .then((response) => response.json())
      .then((data) => data.density || [])
  );
  test.skip(density.length === 0, 'Timeline needs at least one month bucket');

  await expect(page.locator('#timeline-year-select')).toHaveCount(1);

  const results = await new AxeBuilder({ page })
    .include('.timeline-container')
    .withRules(AXE_RULES)
    .analyze();
  expect(results.violations).toEqual([]);
});
