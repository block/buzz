import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { getPublicKey } from "nostr-tools";
import { defineConfig } from "vitest/config";
import {
  hexToBytes,
  TEST_PEER_PRINCIPAL,
  TEST_PEER_SECRET_HEX,
  TEST_READER_SECRET_HEX,
  TEST_REPLICATION_SOURCE,
} from "./test/replication/peer-fixture";

// Runs the replication sink suite with one destination-configured peer
// binding, mirroring how a deployment would configure BUZZ_REPLICATION_PEERS.
export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: {
        configPath: "./wrangler.jsonc",
      },
      miniflare: {
        bindings: {
          // Pin the identity posture; deployed wrangler.jsonc var values
          // (BUZZ_REQUIRE_AUTH, owner anchors) must not leak into suites.
          BUZZ_REQUIRE_AUTH: "",
          BUZZ_OWNER_PUBKEY: "",
          BUZZ_NODE_LABEL: "",
          BUZZ_REPLICATION_PEERS: JSON.stringify({
            [TEST_REPLICATION_SOURCE]: {
              principal: TEST_PEER_PRINCIPAL,
              verification_keys: [
                getPublicKey(hexToBytes(TEST_PEER_SECRET_HEX)),
              ],
            },
          }),
          BUZZ_REPLICATION_STREAMS: JSON.stringify({
            [TEST_REPLICATION_SOURCE]: { mirror: true },
            "rendezvous/notes-only": { filter: [{ kinds: [1] }] },
            "rendezvous/from-peer": { from_source: TEST_REPLICATION_SOURCE },
          }),
          BUZZ_REPLICATION_READERS: JSON.stringify({
            [TEST_REPLICATION_SOURCE]: {
              principal: "did:example:reader-node",
              verification_keys: [
                getPublicKey(hexToBytes(TEST_READER_SECRET_HEX)),
              ],
            },
            "rendezvous/notes-only": {
              principal: "did:example:reader-node",
              verification_keys: [
                getPublicKey(hexToBytes(TEST_READER_SECRET_HEX)),
              ],
            },
            "rendezvous/from-peer": {
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
    include: ["test/replication/*.test.ts"],
  },
});
