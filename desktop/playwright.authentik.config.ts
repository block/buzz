import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/real-authentik",
  timeout: 60_000,
  fullyParallel: false,
  workers: 1,
  reporter: [["list"]],
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "https://workbench.bizfin.test",
    ignoreHTTPSErrors: false,
    launchOptions: {
      args: ["--host-resolver-rules=MAP *.bizfin.test 127.0.0.1"],
    },
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
});
