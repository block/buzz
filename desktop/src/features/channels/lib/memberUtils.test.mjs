import { strict as assert } from "node:assert";
import test from "node:test";

import { canonicalNpub, truncateNpub } from "@/shared/lib/pubkey";
import { compareMembersByRole, formatMemberName } from "./memberUtils.ts";

// Sequential-value pubkeys, the same shape as the members-sidebar e2e
// roster fixture: every full npub shares the `npub1qqq…` head, so the
// compact label is decided by the checksum tail while the full npub
// diverges mid-key. These two keys disagree between the two orders.
const V5_HEX =
  "0000000000000000000000000000000000000000000000000000000000000005";
const V5_NPUB =
  "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqzsfj2hcx";
const V24_HEX =
  "0000000000000000000000000000000000000000000000000000000000000018";
const V24_NPUB =
  "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqvq532w5c";

function member(pubkey, overrides = {}) {
  return {
    pubkey,
    role: "member",
    isAgent: false,
    joinedAt: "2026-09-08T00:00:00Z",
    displayName: null,
    ...overrides,
  };
}

function rosterOrder(members, currentPubkey) {
  return [...members]
    .sort((left, right) => compareMembersByRole(left, right, currentPubkey))
    .map((item) => item.pubkey);
}

test("unnamed members order by full canonical npub, not the compact label", () => {
  assert.equal(canonicalNpub(V5_HEX), V5_NPUB);
  assert.equal(canonicalNpub(V24_HEX), V24_NPUB);
  // Compact labels order V5 first (`…2hcx` < `…2w5c`); the full npubs
  // disagree (`…vq53…` < `…zsfj…`). Ordering follows the full key.
  assert.ok(truncateNpub(V5_HEX).localeCompare(truncateNpub(V24_HEX)) < 0);
  assert.ok(V24_NPUB.localeCompare(V5_NPUB) < 0);

  const v5 = member(V5_HEX);
  const v24 = member(V24_HEX);

  assert.deepEqual(rosterOrder([v5, v24]), [V24_HEX, V5_HEX]);
  assert.deepEqual(rosterOrder([v24, v5]), [V24_HEX, V5_HEX]);

  // The compact 8+4 label stays the display form.
  assert.equal(formatMemberName(v5), "npub1qqq…2hcx");
  assert.equal(formatMemberName(v24), "npub1qqq…2w5c");

  // Authored names still order against npub surfaces as before.
  const bob = member("0".repeat(64), { displayName: "Bob" });
  assert.deepEqual(rosterOrder([v5, bob, v24]), [
    "0".repeat(64),
    V24_HEX,
    V5_HEX,
  ]);
});

test("duplicate authored names tie-break by the full identity key", () => {
  const lowHexName = member(V5_HEX, { displayName: "Ada" });
  const highHexName = member(V24_HEX, { displayName: "Ada" });

  // Same name, distinct keys: the tie breaks by canonical npub (which
  // reverses the raw hex order of these two keys), in both input orders.
  assert.deepEqual(rosterOrder([lowHexName, highHexName]), [V24_HEX, V5_HEX]);
  assert.deepEqual(rosterOrder([highHexName, lowHexName]), [V24_HEX, V5_HEX]);
});

test("role precedence still outranks the name stage", () => {
  const owner = member("1".repeat(64), { role: "owner", displayName: "Zed" });
  const admin = member("2".repeat(64), { role: "admin", displayName: "Yan" });
  const plain = member("3".repeat(64), { role: "member", displayName: "Xan" });
  const guest = member("4".repeat(64), { role: "guest", displayName: "Wes" });
  const bot = member("5".repeat(64), { role: "bot", displayName: "Ann" });

  assert.deepEqual(rosterOrder([bot, guest, plain, admin, owner]), [
    "1".repeat(64),
    "2".repeat(64),
    "3".repeat(64),
    "4".repeat(64),
    "5".repeat(64),
  ]);
});

test("the current member still sorts first in compareMembersByRole", () => {
  const current = member(V24_HEX);
  const owner = member("1".repeat(64), { role: "owner", displayName: "Ada" });

  assert.deepEqual(rosterOrder([owner, current], V24_HEX), [
    V24_HEX,
    "1".repeat(64),
  ]);
  assert.ok(compareMembersByRole(current, owner, V24_HEX) < 0);
  assert.ok(compareMembersByRole(owner, current, V24_HEX) > 0);

  // Without a current pubkey, roles lead again.
  assert.ok(compareMembersByRole(current, owner) > 0);
});

test("invalid keys keep the neutral surface and break ties deterministically", () => {
  const first = member("not-a-key");
  const second = member("zzz-definitely-not-a-key");

  // Both render the neutral label — never raw input — and the label
  // collision breaks by the normalized key, not incoming order.
  assert.equal(formatMemberName(first), "Unavailable");
  assert.equal(formatMemberName(second), "Unavailable");
  assert.deepEqual(rosterOrder([second, first]), [
    "not-a-key",
    "zzz-definitely-not-a-key",
  ]);
  assert.deepEqual(rosterOrder([first, second]), [
    "not-a-key",
    "zzz-definitely-not-a-key",
  ]);
});
