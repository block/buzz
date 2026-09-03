import assert from "node:assert/strict";
import test from "node:test";

import {
  hasHighPriorityOverflow,
  sidebarOverflowUnreadLabel,
} from "./useSidebarUnreadOverflow.ts";

test("labels the destination total as unread", () => {
  assert.equal(sidebarOverflowUnreadLabel(3), "3 unread");
});

test("promotes only when the offscreen set includes actionable unread", () => {
  const actionable = new Set(["dm", "mention"]);

  assert.equal(hasHighPriorityOverflow(["channel"], actionable), false);
  assert.equal(hasHighPriorityOverflow(["channel", "dm"], actionable), true);
  assert.equal(hasHighPriorityOverflow(["mention"], actionable), true);
});
