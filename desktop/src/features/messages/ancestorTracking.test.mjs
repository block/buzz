import assert from "node:assert/strict";
import test from "node:test";

import { shouldResetAncestorTracking } from "@/features/messages/useLoadMissingAncestors";

// useLoadMissingAncestors records fetched ancestor ids so it never re-fetches
// the same id. That tracking must reset when the IDENTITY scope of the cache
// bucket changes, not only when the channel changes — otherwise a cold-start
// ancestor fetched while selfPubkey is undefined (a no-op decrypt that lands
// raw under the [...,null] bucket) is recorded as "done", and after identity
// resolves the effect SKIPS it: the ancestor is never re-fetched/re-decrypted
// into the rendered [...,pubkey] bucket and silently goes missing.

test("ancestor tracking resets when selfPubkey resolves from undefined to a pubkey", () => {
  assert.equal(
    shouldResetAncestorTracking(
      { channelId: "c1", selfPubkey: undefined },
      { channelId: "c1", selfPubkey: "a".repeat(64) },
    ),
    true,
    "cold-start identity resolution must reset so the ancestor is re-fetched",
  );
});

test("ancestor tracking resets when the active channel changes", () => {
  assert.equal(
    shouldResetAncestorTracking(
      { channelId: "c1", selfPubkey: "a".repeat(64) },
      { channelId: "c2", selfPubkey: "a".repeat(64) },
    ),
    true,
  );
});

test("ancestor tracking does NOT reset when neither channel nor identity changed", () => {
  assert.equal(
    shouldResetAncestorTracking(
      { channelId: "c1", selfPubkey: "a".repeat(64) },
      { channelId: "c1", selfPubkey: "a".repeat(64) },
    ),
    false,
    "a stable scope must keep the dedup so the same ancestor is not refetched every render",
  );
});
