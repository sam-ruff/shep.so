import { defineConfig, devices } from "@playwright/test";
export default defineConfig({
  testDir: "e2e",
  timeout: 30000,
  fullyParallel: true,
  outputDir: "../artifacts/web/test-results",
  reporter: [
    ["list"],
    ["html", { outputFolder: "../artifacts/web/report", open: "never" }],
  ],
  use: {
    baseURL: "http://127.0.0.1:5180",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "npm run dev",
    url: "http://127.0.0.1:5180",
    reuseExistingServer: !process.env.CI,
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
