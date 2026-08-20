import assert from "node:assert/strict";
import test from "node:test";

import { normalizePubkey, truncatePubkey } from "./pubkey.ts";

const PUBKEY =
  "44b8e82baa6e0e254e0208d68f335c283c94e7b78dd1fa10d5a49d3f13dd0435";

test("truncates to the canonical 8+4 form with unicode ellipsis", () => {
  assert.equal(truncatePubkey(PUBKEY), "44b8e82b…0435");
});

test("returns short strings unchanged", () => {
  assert.equal(truncatePubkey("abcd1234"), "abcd1234");
  assert.equal(truncatePubkey(""), "");
});

test("normalizePubkey trims and lowercases", () => {
  assert.equal(normalizePubkey("  ABCDEF  "), "abcdef");
});
