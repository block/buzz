import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";
import { TEST_WITNESS_SECRET_HEX } from "./test/pulse/fixture";

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
          BUZZ_NODE_SECRET: TEST_WITNESS_SECRET_HEX,
        },
      },
    }),
  ],
  test: {
    include: ["test/*.test.ts"],
  },
});
