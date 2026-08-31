import { defineConfig, devices } from "@playwright/test";

const webPort = process.env.BUZZ_BESTIE_E2E_WEB_PORT ?? "4174";
const webUrl = `http://127.0.0.1:${webPort}`;

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: "**/bestie-sidebar.spec.ts",
  timeout: 30_000,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: [["list"]],
  use: {
    ...devices["Desktop Chrome"],
    baseURL: webUrl,
    screenshot: "only-on-failure",
    trace: "on-first-retry",
  },
  webServer: {
    command: `python3 -m http.server ${webPort} -d dist`,
    cwd: ".",
    reuseExistingServer: false,
    url: webUrl,
  },
});
