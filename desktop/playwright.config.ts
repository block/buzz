import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 30_000,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: [
    ["list"],
    ["html", { open: "never", outputFolder: "playwright-report" }],
  ],
  use: {
    baseURL: "http://127.0.0.1:4173",
    screenshot: "only-on-failure",
    trace: "on-first-retry",
    video: "retain-on-failure",
  },
  projects: [
    {
      name: "smoke",
      testMatch: [
        "**/smoke.spec.ts",
        "**/channels.spec.ts",
        "**/channel-browser.spec.ts",
        "**/messaging.spec.ts",
        "**/mentions.spec.ts",
        "**/relay-reconnect.spec.ts",
        "**/workflows.spec.ts",
        "**/identity-archive.spec.ts",
      ],
      use: {
        ...devices["Desktop Chrome"],
      },
    },
    {
      name: "integration",
      testMatch: [
        "**/agents.spec.ts",
        "**/onboarding.spec.ts",
        "**/stream.spec.ts",
        "**/integration.spec.ts",
        "**/profile.spec.ts",
        "**/tokens.spec.ts",
        "**/persona-env-vars.spec.ts",
      ],
      use: {
        ...devices["Desktop Chrome"],
      },
    },
  ],
  webServer: {
    command: "python3 -m http.server 4173 -d dist",
    cwd: ".",
    reuseExistingServer: !process.env.CI,
    url: "http://127.0.0.1:4173",
  },
});
