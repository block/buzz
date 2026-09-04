import { defineConfig, devices } from "@playwright/test";

const webPort = process.env.BUZZ_RELEASE_SMOKE_WEB_PORT ?? "4173";
const webUrl = `http://127.0.0.1:${webPort}`;

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: [
    "**/release-smoke.spec.ts",
    "**/dm-history-live-regression.spec.ts",
    "**/foreground-responsiveness-regression.spec.ts",
  ],
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
    baseURL: webUrl,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: `python3 -m http.server ${webPort} -d dist`,
    cwd: ".",
    reuseExistingServer: false,
    url: webUrl,
  },
});
