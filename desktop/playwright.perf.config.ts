import { defineConfig, devices } from "@playwright/test";

import { staticWebServer } from "./tests/helpers/staticWebServer";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 60_000,
  retries: 0,
  workers: 1,
  reporter: [["list"]],
  use: { baseURL: "http://127.0.0.1:4173" },
  projects: [
    {
      name: "perf",
      testMatch: ["**/*.perf.ts"],
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: staticWebServer({ reuseExistingServer: true }),
});
