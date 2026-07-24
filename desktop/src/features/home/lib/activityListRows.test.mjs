import assert from "node:assert/strict";
import test from "node:test";

import { buildActivityListRows } from "./activityListRows.ts";

function inboxItem(
  id,
  latestActivityAt,
  conversationId = `conversation:${id}`,
) {
  return {
    conversationId,
    groupItems: [],
    id,
    item: { id },
    latestActivityAt,
  };
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

function reminder(
  id,
  createdAt,
  status = "pending",
  { eventId, notBefore } = {},
) {
  return {
    id,
    createdAt,
    notBefore,
    content: {
      status,
      target: eventId ? { eventId } : undefined,
    },
  };
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

test("Activity conversation keys stay stable when the representative changes", () => {
  const first = buildActivityListRows({
    drafts: [],
    items: [inboxItem("reply-1", 1, "thread-root")],
    reminders: [],
  });
  const second = buildActivityListRows({
    drafts: [],
    items: [inboxItem("reply-2", 2, "thread-root")],
    reminders: [],
  });

  assert.equal(first[0].key, "inbox:thread-root");
  assert.equal(second[0].key, first[0].key);
});

test("due reminder enriches its existing conversation instead of duplicating it", () => {
  const item = inboxItem("message", 100);
  item.groupItems = [{ id: "reminded-reply" }];
  const rows = buildActivityListRows({
    drafts: [],
    items: [item],
    reminders: [
      reminder("reminder", 50, "pending", {
        eventId: "reminded-reply",
        notBefore: 200,
      }),
    ],
  });

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, "inbox");
  assert.equal(rows[0].dueReminder?.id, "reminder");
  assert.equal(rows[0].sortAt, 200);
});

test("due reminder without a represented conversation sorts at trigger time", () => {
  const rows = buildActivityListRows({
    drafts: [],
    items: [inboxItem("newer-than-creation", 150)],
    reminders: [
      reminder("reminder", 50, "pending", {
        eventId: "not-in-feed",
        notBefore: 200,
      }),
    ],
  });

  assert.deepEqual(
    rows.map((row) => row.kind),
    ["reminder", "inbox"],
  );
});
