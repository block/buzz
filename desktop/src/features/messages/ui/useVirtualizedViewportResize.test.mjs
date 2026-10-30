import assert from "node:assert/strict";
import test from "node:test";

import { shouldSettleVirtualizedViewportResize } from "./useVirtualizedViewportResize.ts";

test("viewport resize follows the virtualizer's explicit bottom state", () => {
  for (const [virtualizerAtBottom, targetHeld, expected] of [
    [true, false, true],
    [false, false, false],
    // A held navigation target keeps the viewport even at the floor.
    [true, true, false],
  ]) {
    assert.equal(
      shouldSettleVirtualizedViewportResize({
        targetHeld,
        virtualizerAtBottom,
      }),
      expected,
      JSON.stringify({ targetHeld, virtualizerAtBottom }),
    );
  }
});
