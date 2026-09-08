// Config for generating design-kit assets (screenshots + video) — not a
// test project. Untracked scratch tooling; see design-kit-assets.spec.ts.
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: ["**/design-kit-assets.spec.ts"],
  timeout: 60_000,
  workers: 1,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:4173",
    video: { mode: "on", size: { width: 1440, height: 900 } },
    ...devices["Desktop Chrome"],
  },
  webServer: {
    command: "python3 -m http.server 4173 -d dist",
    cwd: ".",
    reuseExistingServer: true,
    url: "http://127.0.0.1:4173",
  },
});
