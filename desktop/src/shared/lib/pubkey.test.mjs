import assert from "node:assert/strict";
import test from "node:test";

import { normalizePubkey, truncateHexId, truncatePubkey } from "./pubkey.ts";

const PUBKEY =
  "44b8e82baa6e0e254e0208d68f335c283c94e7b78dd1fa10d5a49d3f13dd0435";
const NPUB = "npub1gjuws2a2dc8z2nszprtg7v6u9q7ffeah3hgl5yx45jwn7y7aqs6s5e9xj6";

test("truncates public keys in canonical npub form", () => {
  assert.equal(truncatePubkey(PUBKEY), "npub1gju…9xj6");
});

test("does not expose invalid public-key input", () => {
  assert.equal(truncatePubkey("abcd1234"), "Invalid public key");
  assert.equal(truncatePubkey(""), "Invalid public key");
});

test("keeps protocol-level event and blob ids in compact hex form", () => {
  assert.equal(truncateHexId(PUBKEY), "44b8e82b…0435");
  assert.equal(truncateHexId("abcd1234"), "abcd1234");
});

test("normalizePubkey converges npub and hex while preserving partial values", () => {
  assert.equal(normalizePubkey(NPUB), PUBKEY);
  assert.equal(normalizePubkey(PUBKEY.toUpperCase()), PUBKEY);
  assert.equal(normalizePubkey("  ABCDEF  "), "abcdef");
});
