import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  outputDir: "test-results-b3-pr-demo",
  timeout: 120_000,
  retries: 0,
  workers: 1,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:4173",
    ...devices["Desktop Chrome"],
    viewport: { width: 1440, height: 900 },
    video: { mode: "on", size: { width: 1440, height: 900 } },
    screenshot: "on",
    trace: "off",
  },
  projects: [
    {
      name: "relay-b3-pr-capture",
      testMatch: "**/workstream-board-b3-pr.capture.spec.ts",
    },
    {
      name: "canvas-bridge-regression",
      testMatch: "**/get-canvas-bridge.spec.ts",
    },
  ],
  webServer: {
    command: "python3 -m http.server 4173 -d dist",
    cwd: ".",
    reuseExistingServer: false,
    url: "http://127.0.0.1:4173",
  },
});
