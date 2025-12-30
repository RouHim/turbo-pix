import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e/specs',

  // Global setup/teardown
  globalSetup: './tests/e2e/setup/global-setup.js',
  globalTeardown: './tests/e2e/setup/global-teardown.js',

  // Timeout settings
  timeout: 30000, // 30s per test
  expect: {
    timeout: 5000, // 5s for assertions
  },

  // Test execution
  fullyParallel: false, // Sequential execution (shared DB state)
  workers: 1, // Single worker due to shared backend
  retries: process.env.CI ? 2 : 0,

  // Reporter configuration
  reporter: [
    ['html', { outputFolder: 'playwright-report' }],
    ['list'],
    ['junit', { outputFile: 'test-results/junit.xml' }],
  ],

  // Base URL for all tests
  use: {
    baseURL: 'http://localhost:18473',

    // Screenshot on failure
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',

    // Video recording for debugging
    video: process.env.CI ? 'retain-on-failure' : 'off',

    // Navigation timeout
    navigationTimeout: 10000,
  },

  // Browser configurations
  projects: [
    {
      name: 'Desktop Chrome',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1920, height: 1080 },
      },
    },
    // Mobile Safari requires system dependencies (webkit)
    // Run: sudo npx playwright install-deps webkit
    // Uncomment to enable:
    // {
    //   name: 'Mobile Safari',
    //   use: {
    //     ...devices['iPhone 13'],
    //   },
    // },
  ],
});
