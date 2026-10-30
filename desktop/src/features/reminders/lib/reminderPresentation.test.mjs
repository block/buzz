import assert from "node:assert/strict";
import test from "node:test";

import {
  activeReminderEventIds,
  describeDueReminders,
  reminderPreviewText,
} from "./reminderPresentation.ts";

const TARGET = {
  eventId: "evt-1",
  channelId: "chan-1",
  preview: "ship it",
  authorPubkey: "pk",
};

function reminder(content) {
  return { id: "r", eventId: "e", createdAt: 0, notBefore: 1, content };
}

const noteOnly = reminder({ status: "pending", note: "stretch" });
const messageReminder = reminder({
  status: "pending",
  target: TARGET,
  note: "follow up",
});
const channels = [{ id: "chan-1", name: "general" }];

test("a note-only reminder previews as its note", () => {
  assert.equal(reminderPreviewText(noteOnly), "stretch");
  assert.equal(reminderPreviewText(messageReminder), "ship it");
});

test("only message reminders tint a message", () => {
  assert.deepEqual(
    [...activeReminderEventIds([noteOnly, messageReminder])],
    ["evt-1"],
  );
  assert.deepEqual(
    [...activeReminderEventIds([reminder({ status: "done", target: TARGET })])],
    [],
  );
});

test("a due note-only reminder notifies with its note and no channel", () => {
  assert.deepEqual(describeDueReminders([noteOnly], channels), {
    title: "Reminder due",
    body: "stretch",
  });
  assert.deepEqual(describeDueReminders([messageReminder], channels), {
    title: "Reminder due in #general",
    body: "ship it",
  });
  assert.deepEqual(
    describeDueReminders([noteOnly, messageReminder], channels),
    {
      title: "Reminder due",
      body: "2 reminders are due",
    },
  );
});
