/**
 * Mirrors the pure helpers in relay-self.ts. Run with:
 *   node --test web/src/shared/lib/relay-self.test.mjs
 *
 * Kept self-contained because buzz-web has no vitest/ts-node runner.
 * If these assertions drift from relay-self.ts, update both.
 */
import assert from "node:assert/strict";
import test from "node:test";

const HEX_PUBKEY = /^[0-9a-f]{64}$/;

function parseRelaySelfPubkey(value) {
  if (typeof value !== "string") return null;
  const normalized = value.toLowerCase();
  return HEX_PUBKEY.test(normalized) ? normalized : null;
}

function trustedRepoRefAuthors(relaySelf, ownerPubkey) {
  return [
    ...new Set(
      [ownerPubkey, relaySelf]
        .map((value) => (typeof value === "string" ? value.toLowerCase() : ""))
        .filter((value) => HEX_PUBKEY.test(value)),
    ),
  ];
}

test("parseRelaySelfPubkey accepts lowercase hex", () => {
  const pk = "a".repeat(64);
  assert.equal(parseRelaySelfPubkey(pk), pk);
});

test("parseRelaySelfPubkey normalizes uppercase", () => {
  const pk = "AB".repeat(32);
  assert.equal(parseRelaySelfPubkey(pk), pk.toLowerCase());
});

test("parseRelaySelfPubkey rejects invalid values", () => {
  assert.equal(parseRelaySelfPubkey(null), null);
  assert.equal(parseRelaySelfPubkey(undefined), null);
  assert.equal(parseRelaySelfPubkey("short"), null);
  assert.equal(parseRelaySelfPubkey("g".repeat(64)), null);
  assert.equal(parseRelaySelfPubkey(123), null);
});

test("trustedRepoRefAuthors requires relay self and dedupes owner", () => {
  const relay = "b".repeat(64);
  const owner = "c".repeat(64);
  assert.deepEqual(trustedRepoRefAuthors(relay), [relay]);
  assert.deepEqual(
    trustedRepoRefAuthors(relay, owner).sort(),
    [owner, relay].sort(),
  );
  assert.deepEqual(trustedRepoRefAuthors(relay, relay), [relay]);
  assert.deepEqual(trustedRepoRefAuthors(relay, "not-a-key"), [relay]);
});
