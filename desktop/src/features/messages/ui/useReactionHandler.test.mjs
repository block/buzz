import assert from "node:assert/strict";
import test from "node:test";

import { applyOptimisticReaction } from "./useReactionHandler.ts";

// ---------------------------------------------------------------------------
// applyOptimisticReaction — order-preservation invariant
//
// The hook's reactions memo is `optimisticReactions ?? sourceReactions ?? []`
// with no re-sort. These tests verify that applyOptimisticReaction itself
// never reorders pills so the formatter's chronological order is preserved
// through optimistic updates.
// ---------------------------------------------------------------------------

function pill(emoji, count, reactedByCurrentUser = false) {
  return {
    emoji,
    count,
    reactedByCurrentUser,
    users: reactedByCurrentUser
      ? [{ pubkey: "aaa", displayName: "You", avatarUrl: null }]
      : [{ pubkey: "bbb", displayName: "Alice", avatarUrl: null }],
  };
}

test("applyOptimisticReaction: adding new emoji appends to end, preserving prior order", () => {
  // Formatter emits [🎉 (count=3), 👍 (count=1)] — chronological, not count-ranked.
  const source = [pill("🎉", 3), pill("👍", 1)];
  const result = applyOptimisticReaction(source, "❤️", false);
  assert.deepEqual(
    result.map((r) => r.emoji),
    ["🎉", "👍", "❤️"],
    "new reaction must append to end, not reorder by count",
  );
});

test("applyOptimisticReaction: incrementing an existing emoji preserves position", () => {
  // A later emoji (👍) has a lower count than an earlier one (🎉).
  // Incrementing 👍 must NOT move it left of 🎉.
  const source = [pill("🎉", 3), pill("👍", 1)];
  const result = applyOptimisticReaction(source, "👍", false);
  assert.deepEqual(
    result.map((r) => r.emoji),
    ["🎉", "👍"],
    "incrementing count on a later emoji must not change its position",
  );
  assert.equal(result[1].count, 2);
});

test("applyOptimisticReaction: removing last reactor removes pill, preserving remaining order", () => {
  const source = [pill("🎉", 2), pill("👍", 1, true), pill("❤️", 1)];
  const result = applyOptimisticReaction(source, "👍", true);
  assert.deepEqual(
    result.map((r) => r.emoji),
    ["🎉", "❤️"],
    "removing a pill must not reorder the remaining pills",
  );
});

test("applyOptimisticReaction: removing one of several reactors decrements count, preserves position", () => {
  const source = [pill("🎉", 1), pill("👍", 2, true)];
  const result = applyOptimisticReaction(source, "👍", true);
  assert.deepEqual(
    result.map((r) => r.emoji),
    ["🎉", "👍"],
  );
  assert.equal(result[1].count, 1);
});

test("applyOptimisticReaction: no-op when removing an emoji the user has not reacted with", () => {
  const source = [pill("🎉", 1), pill("👍", 1)];
  const result = applyOptimisticReaction(source, "👍", true);
  assert.equal(
    result,
    source,
    "must return same reference when nothing changes",
  );
});

test("applyOptimisticReaction: no-op when adding an emoji the user already reacted with", () => {
  const source = [pill("🎉", 1, true)];
  const result = applyOptimisticReaction(source, "🎉", false);
  assert.equal(
    result,
    source,
    "must return same reference when already reacted",
  );
});
