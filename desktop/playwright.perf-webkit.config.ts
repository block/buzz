import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 900_000,
  retries: 0,
  workers: 1,
  reporter: [["list"]],
  use: { baseURL: "http://127.0.0.1:4173" },
  projects: [
    {
      name: "perf-webkit",
      testMatch: ["**/*.perf.ts"],
      use: { ...devices["Desktop Safari"] },
    },
  ],
  webServer: {
    command: "python3 -m http.server 4173 -d dist",
    cwd: ".",
    reuseExistingServer: true,
    url: "http://127.0.0.1:4173",
  },
});
