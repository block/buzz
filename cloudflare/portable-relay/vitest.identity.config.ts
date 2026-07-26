import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

// Runs the identity conformance suite with the secured mode enabled, the
// same worker configuration the core suite uses otherwise.
export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: {
        configPath: "./wrangler.jsonc",
      },
      miniflare: {
        bindings: {
          BUZZ_REQUIRE_AUTH: "1",
        },
      },
    }),
  ],
  test: {
    include: ["test/identity/*.test.ts"],
  },
});
