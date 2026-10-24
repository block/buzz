import assert from "node:assert/strict";
import test from "node:test";

import {
  hasHighPriorityOverflow,
  sidebarOverflowUnreadLabel,
} from "./useSidebarUnreadOverflow.ts";

test("labels the destination total as unread", () => {
  assert.equal(sidebarOverflowUnreadLabel(3), "3 unread");
});

test("promotes actionable unread and every offscreen DM", () => {
  const actionable = new Set(["mention"]);
  const dms = new Set(["dm"]);

  assert.equal(hasHighPriorityOverflow(["channel"], actionable, dms), false);
  assert.equal(hasHighPriorityOverflow(["mention"], actionable, dms), true);
  assert.equal(hasHighPriorityOverflow(["dm"], actionable, dms), true);
  assert.equal(
    hasHighPriorityOverflow(["channel", "dm"], actionable, dms),
    true,
  );
});
