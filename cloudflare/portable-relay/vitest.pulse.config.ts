import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { getPublicKey } from "nostr-tools";
import { defineConfig } from "vitest/config";
import {
  hexToBytes,
  TEST_PEER_SECRET_HEX,
  TEST_READER_SECRET_HEX,
  TEST_REPLICATION_SOURCE,
} from "./test/replication/peer-fixture";
import {
  TEST_OWNER_SECRET_HEX,
  TEST_PULSE_NODE_LABEL,
  TEST_WITNESS_SECRET_HEX,
} from "./test/pulse/fixture";

// Runs the Beacon pulse standing suite in the deployed posture: identity
// required, a witness key present, and owner anchors set so kind-30700
// declaration heads govern and appear as agreement pins in the pulse.
export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: {
        configPath: "./wrangler.jsonc",
      },
      miniflare: {
        bindings: {
          BUZZ_REQUIRE_AUTH: "1",
          BUZZ_NODE_SECRET: TEST_WITNESS_SECRET_HEX,
          BUZZ_OWNER_PUBKEY: getPublicKey(hexToBytes(TEST_OWNER_SECRET_HEX)),
          BUZZ_NODE_LABEL: TEST_PULSE_NODE_LABEL,
          BUZZ_REPLICATION_PEERS: JSON.stringify({
            [TEST_REPLICATION_SOURCE]: {
              principal: "did:example:buzz-laptop-test",
              verification_keys: [
                getPublicKey(hexToBytes(TEST_PEER_SECRET_HEX)),
              ],
            },
          }),
          BUZZ_REPLICATION_STREAMS: JSON.stringify({
            [TEST_REPLICATION_SOURCE]: { mirror: true },
          }),
          BUZZ_REPLICATION_READERS: JSON.stringify({
            [TEST_REPLICATION_SOURCE]: {
              principal: "did:example:reader-node",
              verification_keys: [
                getPublicKey(hexToBytes(TEST_READER_SECRET_HEX)),
              ],
            },
          }),
        },
      },
    }),
  ],
  test: {
    include: ["test/pulse/*.test.ts"],
  },
});
