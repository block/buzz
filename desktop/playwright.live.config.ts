import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: "**/agents-everywhere.live.spec.ts",
  timeout: 90_000,
  workers: 1,
  reporter: "list",
});
