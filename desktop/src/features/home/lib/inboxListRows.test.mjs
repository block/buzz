import assert from "node:assert/strict";
import test from "node:test";

import { buildInboxListRows } from "./inboxListRows.ts";

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

test("Inbox All combines rows in latest-first order", () => {
  const rows = buildInboxListRows({
    items: [inboxItem("message", 1_753_099_300)],
    reminders: [reminder("reminder", 1_753_099_100)],
  });

  assert.deepEqual(
    rows.map((row) => row.kind),
    ["inbox", "reminder"],
  );
});

test("Inbox All excludes completed reminders", () => {
  const rows = buildInboxListRows({
    items: [],
    reminders: [reminder("done", 1_753_099_100, "done")],
  });

  assert.deepEqual(rows, []);
});

test("Inbox conversation keys stay stable when the representative changes", () => {
  const first = buildInboxListRows({
    items: [inboxItem("reply-1", 1, "thread-root")],
    reminders: [],
  });
  const second = buildInboxListRows({
    items: [inboxItem("reply-2", 2, "thread-root")],
    reminders: [],
  });

  assert.equal(first[0].key, "inbox:thread-root");
  assert.equal(second[0].key, first[0].key);
});

test("due reminder enriches its existing conversation instead of duplicating it", () => {
  const item = inboxItem("message", 100);
  item.groupItems = [{ id: "reminded-reply" }];
  const rows = buildInboxListRows({
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
  const rows = buildInboxListRows({
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
