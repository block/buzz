import assert from "node:assert/strict";
import test from "node:test";

import {
  shouldVirtualizeThreadReplies,
  THREAD_REPLY_VIRTUALIZATION_THRESHOLD,
} from "./threadReplyVirtualization.ts";

test("small thread reply lists stay fully mounted", () => {
  assert.equal(shouldVirtualizeThreadReplies(0), false);
  assert.equal(
    shouldVirtualizeThreadReplies(THREAD_REPLY_VIRTUALIZATION_THRESHOLD),
    false,
  );
});

test("large thread reply lists are virtualized", () => {
  assert.equal(
    shouldVirtualizeThreadReplies(THREAD_REPLY_VIRTUALIZATION_THRESHOLD + 1),
    true,
  );
  assert.equal(shouldVirtualizeThreadReplies(2_418), true);
});
