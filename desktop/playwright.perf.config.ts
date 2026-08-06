import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  // Wave-1 typing burst + busy-channel warmup exceeds 60s on throttled CPU.
  timeout: 180_000,
  retries: 0,
  workers: 1,
  reporter: [["list"]],
  // Perf runs are repeated dozens of times — never retain failure artifacts
  // (screenshots/traces fill the disk and previously caused ENOSPC mid-suite).
  use: {
    baseURL: "http://127.0.0.1:4173",
    screenshot: "off",
    video: "off",
    trace: "off",
  },
  outputDir: "/tmp/buzz-playwright-perf-results",
  projects: [
    {
      name: "perf",
      testMatch: ["**/*.perf.ts"],
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: "python3 -m http.server 4173 -d dist",
    cwd: ".",
    // Keep a warm static server across the collect loop — restarting it every
    // attempt was a major source of mock-bridge boot flakes.
    reuseExistingServer: true,
    url: "http://127.0.0.1:4173",
  },
});
