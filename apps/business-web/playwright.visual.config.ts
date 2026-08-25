import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/visual",
  timeout: 30_000,
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  snapshotPathTemplate: "{testDir}/__screenshots__/{arg}{ext}",
  expect: {
    timeout: 8_000,
    toHaveScreenshot: {
      animations: "disabled",
      caret: "hide",
      maxDiffPixelRatio: 0.01,
      threshold: 0.2,
    },
  },
  use: {
    ...devices["Desktop Chrome"],
    viewport: { width: 1366, height: 768 },
    baseURL: "http://127.0.0.1:4175",
    colorScheme: "light",
    reducedMotion: "reduce",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "vite preview --host 127.0.0.1 --port 4175 --strictPort",
    cwd: ".",
    reuseExistingServer: false,
    url: "http://127.0.0.1:4175",
  },
});
