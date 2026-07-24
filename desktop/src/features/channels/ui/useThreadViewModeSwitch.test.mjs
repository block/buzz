import assert from "node:assert/strict";
import test from "node:test";

import {
  findTopVisibleThreadMessageId,
  getResolvedThreadTargets,
} from "./useThreadViewModeSwitch.ts";

function row(id, top, bottom) {
  return {
    dataset: { messageId: id },
    getBoundingClientRect: () => ({ bottom, top }),
  };
}

test("finds the first thread message crossing the viewport top", () => {
  const rows = [
    row("above", -80, -5),
    row("crossing", -20, 30),
    row("below", 40, 90),
  ];
  const body = {
    getBoundingClientRect: () => ({ top: 0 }),
    querySelectorAll: () => rows,
  };

  assert.equal(findTopVisibleThreadMessageId(body), "crossing");
});

test("resolves both sources when a layout anchor matches the external target", () => {
  assert.deepEqual(
    getResolvedThreadTargets({
      externalTargetId: "reply-b",
      layoutTargetId: "reply-b",
    }),
    { resolveExternal: true, resolveLayout: true },
  );
  assert.deepEqual(
    getResolvedThreadTargets({
      externalTargetId: "reply-b",
      layoutTargetId: "reply-c",
    }),
    { resolveExternal: false, resolveLayout: true },
  );
});

test("returns null without a mounted thread body or visible message", () => {
  assert.equal(findTopVisibleThreadMessageId(null), null);
  assert.equal(
    findTopVisibleThreadMessageId({
      getBoundingClientRect: () => ({ top: 0 }),
      querySelectorAll: () => [row("above", -80, -1)],
    }),
    null,
  );
});
