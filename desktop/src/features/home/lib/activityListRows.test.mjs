import assert from "node:assert/strict";
import test from "node:test";

import { buildActivityListRows } from "./activityListRows.ts";

function inboxItem(id, latestActivityAt) {
  return { id, latestActivityAt };
}

function draftItem(key, updatedAt, rootStatus = "available") {
  return {
    entry: {
      key,
      draft: { createdAt: updatedAt, updatedAt },
    },
    rootStatus,
  };
}

function reminder(id, createdAt, status = "pending") {
  return { id, createdAt, content: { status } };
}

test("Activity All combines rows in latest-first order", () => {
  const rows = buildActivityListRows({
    drafts: [draftItem("draft", "2026-07-21T12:00:00.000Z")],
    items: [inboxItem("message", 1_753_099_300)],
    reminders: [reminder("reminder", 1_753_099_100)],
  });

  assert.deepEqual(
    rows.map((row) => row.kind),
    ["draft", "inbox", "reminder"],
  );
});

test("Activity All excludes completed reminders and deleted-root drafts", () => {
  const rows = buildActivityListRows({
    drafts: [draftItem("deleted", "2026-07-21T12:00:00.000Z", "deleted")],
    items: [],
    reminders: [reminder("done", 1_753_099_100, "done")],
  });

  assert.deepEqual(rows, []);
});
