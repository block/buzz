import assert from "node:assert/strict";
import test from "node:test";

import { resolveMentionAckDisplayName } from "./useChannelMentionAckFooters.ts";

test("resolveMentionAckDisplayName prefers displayName over name", () => {
  const profiles = {
    abc: { displayName: "Ada", name: "lovelace" },
  };
  assert.equal(resolveMentionAckDisplayName(profiles, "abc"), "Ada");
});

test("resolveMentionAckDisplayName falls back to the kind-0 name", () => {
  const profiles = {
    abc: { displayName: null, name: "lovelace" },
  };
  assert.equal(resolveMentionAckDisplayName(profiles, "abc"), "lovelace");
});

test("resolveMentionAckDisplayName falls back to a short pubkey", () => {
  const pubkey = "a".repeat(64);
  // Mirror the production fallback: a truncated hex, never the full key.
  const resolved = resolveMentionAckDisplayName(undefined, pubkey);
  assert.notEqual(resolved, pubkey);
  assert.ok(resolved.length < pubkey.length);
  assert.ok(resolved.length > 0);
});
