import assert from "node:assert/strict";
import test from "node:test";

import {
  canonicalNpub,
  normalizePubkey,
  truncateNpub,
  truncatePubkey,
} from "./pubkey.ts";

const PUBKEY =
  "44b8e82baa6e0e254e0208d68f335c283c94e7b78dd1fa10d5a49d3f13dd0435";
const PUBKEY_NPUB =
  "npub1gjuws2a2dc8z2nszprtg7v6u9q7ffeah3hgl5yx45jwn7y7aqs6s5e9xj6";
const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const HEX_NPUB =
  "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";

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

test("truncateNpub compacts the hex pubkey's npub, not its hex form", () => {
  assert.equal(truncateNpub(PUBKEY), "npub1gju…9xj6");
  assert.equal(truncateNpub(HEX), "npub1a2d…yp60");
  assert.equal(truncateNpub(HEX.toUpperCase()), "npub1a2d…yp60");
});

test("truncateNpub accepts already-npub strings", () => {
  assert.equal(truncateNpub(PUBKEY_NPUB), "npub1gju…9xj6");
  assert.equal(truncateNpub(`  ${HEX_NPUB} `), "npub1a2d…yp60");
  // All-uppercase Bech32 is a valid identity per the parser; render the
  // canonical form, never the neutral label.
  assert.equal(truncateNpub(HEX_NPUB.toUpperCase()), "npub1a2d…yp60");
});

test("truncateNpub renders the neutral label for invalid identities", () => {
  // Never the raw hex/input fallback: a wrong-length or non-hex string is not
  // a displayable identity.
  assert.equal(truncateNpub(""), "Unavailable");
  assert.equal(truncateNpub("not a pubkey"), "Unavailable");
  assert.equal(truncateNpub(`${HEX.slice(0, 63)}`), "Unavailable");
  assert.equal(truncateNpub(`z${HEX.slice(1)}`), "Unavailable");
  // Corrupted npub checksum is not a valid identity either.
  assert.equal(truncateNpub(`${HEX_NPUB.slice(0, -1)}q`), "Unavailable");
  // Other bech32 entities are not pubkeys.
  assert.equal(
    truncateNpub(
      "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5",
    ),
    "Unavailable",
  );
});

test("canonicalNpub returns the full npub for valid identities only", () => {
  assert.equal(canonicalNpub(HEX), HEX_NPUB);
  assert.equal(canonicalNpub(HEX.toUpperCase()), HEX_NPUB);
  assert.equal(canonicalNpub(HEX_NPUB), HEX_NPUB);
  // All-uppercase Bech32 is valid and returns the canonical lowercase npub
  // (parser agreement); a mixed-case npub is invalid Bech32.
  assert.equal(canonicalNpub(HEX_NPUB.toUpperCase()), HEX_NPUB);
  assert.equal(
    canonicalNpub(
      `${HEX_NPUB.slice(0, 10)}${HEX_NPUB.slice(10).toUpperCase()}`,
    ),
    null,
  );
  // Strict identity keys only — short/degenerate payloads never encode.
  assert.equal(canonicalNpub(""), null);
  assert.equal(canonicalNpub("deadbeef"), null);
  assert.equal(canonicalNpub(`${HEX.slice(0, 63)}`), null);
  // Checksum-valid short npubs are degenerate payloads too (8-char and
  // empty) — `npubEncode` would happily re-encode them, so never bind them.
  assert.equal(canonicalNpub("npub1m6kmamcvty5gd"), null);
  assert.equal(canonicalNpub("npub106246s"), null);
  // Corrupted checksum never binds as the identity it resembles.
  assert.equal(canonicalNpub(`${HEX_NPUB.slice(0, -2)}qq`), null);
});
