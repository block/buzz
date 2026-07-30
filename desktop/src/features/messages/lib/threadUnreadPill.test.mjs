import assert from "node:assert/strict";
import test from "node:test";

import {
  computeThreadUnreadPillTarget,
  threadUnreadPillLabel,
} from "./threadUnreadPill.ts";

function msg(id) {
  return { id };
}

test("computeThreadUnreadPillTarget_noCounts_returnsEmpty", () => {
  const target = computeThreadUnreadPillTarget([msg("a"), msg("b")], undefined);
  assert.equal(target.totalUnreadReplies, 0);
  assert.equal(target.oldestParentId, null);
});

test("computeThreadUnreadPillTarget_emptyMap_returnsEmpty", () => {
  const target = computeThreadUnreadPillTarget([msg("a")], new Map());
  assert.equal(target.totalUnreadReplies, 0);
  assert.equal(target.oldestParentId, null);
});

test("computeThreadUnreadPillTarget_sumsAndPicksOldestLoadedRoot", () => {
  const target = computeThreadUnreadPillTarget(
    [msg("a"), msg("b"), msg("c")],
    new Map([
      ["b", 2],
      ["c", 3],
    ]),
  );
  assert.equal(target.totalUnreadReplies, 5);
  // Messages are chronological, so the first hit is the oldest root.
  assert.equal(target.oldestParentId, "b");
});

test("computeThreadUnreadPillTarget_ignoresZeroAndUnloadedRoots", () => {
  const target = computeThreadUnreadPillTarget(
    [msg("a"), msg("b")],
    new Map([
      ["a", 0],
      ["b", 1],
      ["not-loaded", 7],
    ]),
  );
  // Roots outside the loaded window can't be jump targets and don't count.
  assert.equal(target.totalUnreadReplies, 1);
  assert.equal(target.oldestParentId, "b");
});

test("threadUnreadPillLabel_pluralizesReplies", () => {
  assert.equal(threadUnreadPillLabel(1), "1 new reply in threads");
  assert.equal(threadUnreadPillLabel(4), "4 new replies in threads");
});
