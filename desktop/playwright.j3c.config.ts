import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e/j3c",
  testMatch: "current-binding-status-native-trace.spec.ts",
  timeout: 30_000,
  retries: 0,
  workers: 1,
  outputDir: "test-results/j3c-current-binding-status",
  reporter: [["list"]],
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "http://127.0.0.1:4175",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    video: "retain-on-failure",
  },
  projects: [
    {
      name: "current-binding-status-native-trace",
    },
  ],
  webServer: {
    command: "python3 -m http.server 4175 -d dist",
    cwd: ".",
    reuseExistingServer: false,
    url: "http://127.0.0.1:4175",
  },
});
