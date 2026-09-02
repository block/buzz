import assert from "node:assert/strict";
import test from "node:test";

import {
  countOffscreenUnreadMessages,
  hasHighPriorityOverflow,
  sidebarOverflowUnreadLabel,
} from "./useSidebarUnreadOverflow.ts";

test("counts unread messages across offscreen channels", () => {
  assert.equal(
    countOffscreenUnreadMessages(
      ["ordinary", "mention", "manual"],
      new Map([
        ["ordinary", 10],
        ["mention", 1],
      ]),
    ),
    12,
  );
});

test("labels the stable total as unread", () => {
  assert.equal(sidebarOverflowUnreadLabel(11), "11 unread");
});

test("promotes only when the offscreen set includes actionable unread", () => {
  const actionable = new Set(["dm", "mention"]);

  assert.equal(hasHighPriorityOverflow(["channel"], actionable), false);
  assert.equal(hasHighPriorityOverflow(["channel", "dm"], actionable), true);
  assert.equal(hasHighPriorityOverflow(["mention"], actionable), true);
});
