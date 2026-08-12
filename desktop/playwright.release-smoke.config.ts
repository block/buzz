import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: ["**/release-smoke.spec.ts"],
  timeout: 10 * 60_000,
  retries: 0,
  workers: 1,
  reporter: [
    ["list"],
    ["json", { outputFile: "test-results/release-smoke/playwright.json" }],
    [
      "html",
      { open: "never", outputFolder: "playwright-release-smoke-report" },
    ],
  ],
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "http://127.0.0.1:4173",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "python3 -m http.server 4173 -d dist",
    cwd: ".",
    reuseExistingServer: false,
    url: "http://127.0.0.1:4173",
  },
});
