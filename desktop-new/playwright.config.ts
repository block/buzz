import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  use: { baseURL: "http://127.0.0.1:1430/?mock=1" },
  webServer: {
    command: "pnpm dev --host 127.0.0.1",
    url: "http://127.0.0.1:1430/?mock=1",
    reuseExistingServer: false,
  },
});
