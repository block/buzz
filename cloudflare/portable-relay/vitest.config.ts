import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: {
        configPath: "./wrangler.jsonc",
      },
      miniflare: {
        // Tests pin their own identity posture; deployed wrangler.jsonc var
        // values (e.g. BUZZ_REQUIRE_AUTH="1") must not leak into suites.
        bindings: {
          BUZZ_REQUIRE_AUTH: "",
          BUZZ_OWNER_PUBKEY: "",
          BUZZ_NODE_LABEL: "",
        },
      },
    }),
  ],
  test: {
    include: ["test/*.test.ts"],
  },
});
