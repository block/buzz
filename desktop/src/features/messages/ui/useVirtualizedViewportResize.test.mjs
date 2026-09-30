import assert from "node:assert/strict";
import test from "node:test";

import { shouldSettleVirtualizedViewportResize } from "./useVirtualizedViewportResize.ts";

test("viewport resize follows the virtualizer's explicit bottom state", () => {
  assert.equal(
    shouldSettleVirtualizedViewportResize({ virtualizerAtBottom: true }),
    true,
  );
  assert.equal(
    shouldSettleVirtualizedViewportResize({ virtualizerAtBottom: false }),
    false,
  );
});
