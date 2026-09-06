import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  workers: 2,
  retries: 0,
  timeout: 30_000,
  outputDir: '../artifacts/website/test-results',
  reporter: [['list'], ['html', { outputFolder: '../artifacts/website/report', open: 'never' }]],
  use: {
    baseURL: 'http://127.0.0.1:4178',
    viewport: { width: 1440, height: 920 },
    colorScheme: 'light',
    reducedMotion: 'reduce',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
    { name: 'firefox', use: { ...devices['Desktop Firefox'], viewport: { width: 1440, height: 920 } } },
    { name: 'webkit', use: { ...devices['Desktop Safari'], viewport: { width: 1440, height: 920 } } },
  ],
  webServer: { command: 'npm run preview', url: 'http://127.0.0.1:4178', reuseExistingServer: false },
});
