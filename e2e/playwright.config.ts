// NOTE: These tests require Playwright to be installed.
// Run: npm install --save-dev @playwright/test
// Then: npx playwright install

import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  retries: 1,
  workers: 3,
  reporter: "html",
  use: {
    baseURL: "http://localhost:1420",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: "npm run dev:web",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
