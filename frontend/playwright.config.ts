import { defineConfig, devices } from '@playwright/test';

/**
 * End-to-end tests for the web app.
 *
 * They drive the real SPA against a running parcel-server in **dev auth mode**
 * (`AUTH_MODE=dev`: every request is the dev user, so no OAuth is involved).
 * The backend's app origin (`PARCEL_APP_ORIGIN`) must be the origin the tests
 * load, or the content host's `frame-ancestors` CSP blocks the viewer iframe.
 *
 * - Default: `bun run e2e` starts (or reuses) Vite on :5178, which proxies
 *   `/api` to the backend on :8106.
 * - Against another stack: `PARCEL_E2E_BASE_URL=https://parcel.local.stridelabs.ai bun run e2e`
 *   (no dev server is started).
 */
const BASE_URL = process.env.PARCEL_E2E_BASE_URL ?? 'http://localhost:5178';
const START_DEV_SERVER = !process.env.PARCEL_E2E_BASE_URL;

export default defineConfig({
  testDir: './e2e',
  // Serial: the suite shares one backend, database and dev user.
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  timeout: 30_000,
  expect: { timeout: 10_000 },
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: BASE_URL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    // Local *.local.stridelabs.ai certificates may not be in Playwright's store.
    ignoreHTTPSErrors: true,
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: START_DEV_SERVER
    ? {
        command: 'bun run dev',
        url: BASE_URL,
        reuseExistingServer: !process.env.CI,
        timeout: 60_000,
        stdout: 'ignore',
        stderr: 'pipe',
      }
    : undefined,
});
