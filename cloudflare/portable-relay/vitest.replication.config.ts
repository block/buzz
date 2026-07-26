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
          BUZZ_REPLICATION_PEERS: JSON.stringify({
            [TEST_REPLICATION_SOURCE]: {
              principal: TEST_PEER_PRINCIPAL,
              verification_keys: [
                getPublicKey(hexToBytes(TEST_PEER_SECRET_HEX)),
              ],
            },
          }),
          BUZZ_REPLICATION_STREAMS: JSON.stringify({
            [TEST_REPLICATION_SOURCE]: { filter: null },
            "rendezvous/notes-only": { filter: [{ kinds: [1] }] },
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
          }),
        },
      },
    }),
  ],
  test: {
    include: ["test/replication/*.test.ts"],
  },
});
