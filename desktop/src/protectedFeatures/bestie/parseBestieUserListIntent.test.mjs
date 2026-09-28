import assert from "node:assert/strict";
import test from "node:test";

import {
  crystallizeReminderText,
  messageLooksLikeBestieListRequest,
  messageLooksLikeBestieReminderMeridiemReply,
  messageNeedsBestieReminderBareClockConfirm,
  parseBestieDueAtFromText,
  parseBestieReminderMeridiemReply,
  parseBestieUserListIntent,
  reconcileReminderDueAtWithStatedMeridiem,
  resolveBestieBareClockDueAt,
} from "./parseBestieUserListIntent.ts";

const NOW = Date.parse("2026-09-26T15:00:00.000-07:00");

test("crystallizeReminderText capitalizes and drops leading to", () => {
  assert.equal(
    crystallizeReminderText("run the nightly report"),
    "Run the nightly report",
  );
  assert.equal(
    crystallizeReminderText("to run the nightly report"),
    "Run the nightly report",
  );
  assert.equal(crystallizeReminderText("stretch"), "Stretch");
});

test("parses remind me … in N minutes with crystallized text", () => {
  const intent = parseBestieUserListIntent(
    "Remind me to stretch in 20 minutes",
    NOW,
  );
  assert.equal(intent?.op, "add");
  assert.equal(intent?.items[0].kind, "reminder");
  assert.equal(intent?.items[0].text, "Stretch");
  assert.equal(intent?.items[0].dueAt, Math.floor(NOW / 1000) + 20 * 60);
});

test("parses remind me … tomorrow at time with am/pm", () => {
  const intent = parseBestieUserListIntent(
    "remind me to call mom tomorrow at 9am",
    NOW,
  );
  assert.equal(intent?.op, "add");
  assert.equal(intent?.items[0].text, "Call mom");
  assert.ok(intent?.items[0].dueAt);
  const due = new Date(intent.items[0].dueAt * 1000);
  assert.equal(due.getHours(), 9);
  assert.ok(due.getTime() > NOW);
  assert.notEqual(due.toDateString(), new Date(NOW).toDateString());
});

test("bare clock without am/pm does not auto-add — confirm needed", () => {
  const intent = parseBestieUserListIntent(
    "Remind me to run the nightly report at 8:36",
    NOW,
  );
  assert.equal(intent?.op, "reminder-confirm-needed");
  assert.equal(intent?.text, "Run the nightly report");
  assert.deepEqual(intent?.bareClock, {
    dayHint: null,
    hour: 8,
    minute: 36,
  });
  assert.equal(
    messageNeedsBestieReminderBareClockConfirm(
      "Remind me to run the nightly report at 8:36",
      NOW,
    ),
    true,
  );
  assert.equal(
    messageNeedsBestieReminderBareClockConfirm(
      "Remind me to stretch in 5 minutes",
      NOW,
    ),
    false,
  );
});

test("clock with am/pm still auto-adds crystallized text", () => {
  const intent = parseBestieUserListIntent(
    "Remind me to finish the nightly reports at 8:10pm",
    NOW,
  );
  assert.equal(intent?.op, "add");
  assert.equal(intent?.items[0].text, "Finish the nightly reports");
  assert.ok(intent?.items[0].dueAt);
  const due = new Date(intent.items[0].dueAt * 1000);
  assert.equal(due.getHours(), 20);
  assert.equal(due.getMinutes(), 10);
});

test("24h hour without am/pm is unambiguous and auto-adds", () => {
  const intent = parseBestieUserListIntent(
    "Remind me to ship the build at 14:30",
    NOW,
  );
  assert.equal(intent?.op, "add");
  assert.equal(intent?.items[0].text, "Ship the build");
  const due = new Date(intent.items[0].dueAt * 1000);
  assert.equal(due.getHours(), 14);
  assert.equal(due.getMinutes(), 30);
});

test("parses add todo", () => {
  const intent = parseBestieUserListIntent("Add a todo: ship phase 3", NOW);
  assert.deepEqual(intent, {
    items: [{ kind: "todo", text: "ship phase 3" }],
    op: "add",
  });
});

test("parses complete / remove by text", () => {
  assert.deepEqual(
    parseBestieUserListIntent("mark todo ship phase 3 done", NOW),
    {
      kind: "todo",
      op: "complete-match",
      text: "ship phase 3",
    },
  );
  assert.deepEqual(parseBestieUserListIntent("remove reminder call mom", NOW), {
    kind: "reminder",
    op: "remove-match",
    text: "call mom",
  });
});

test("ignores free-form chat", () => {
  assert.equal(
    parseBestieUserListIntent("how are my reminders looking?", NOW),
    null,
  );
  assert.equal(messageLooksLikeBestieListRequest("just chatting"), false);
});

test("parseBestieDueAtFromText handles at-time roll to tomorrow", () => {
  // 2pm already passed relative to NOW (3pm PT) → next day.
  const { ambiguousBareClock, dueAt, textWithoutDue } = parseBestieDueAtFromText(
    "pay rent at 2pm",
    NOW,
  );
  assert.equal(ambiguousBareClock, null);
  assert.equal(textWithoutDue, "pay rent");
  assert.ok(dueAt);
  const due = new Date(dueAt * 1000);
  assert.equal(due.getHours(), 14);
  assert.ok(due.getTime() > NOW);
});

test("parseBestieDueAtFromText flags bare clock ambiguity", () => {
  const { ambiguousBareClock, dueAt, textWithoutDue } = parseBestieDueAtFromText(
    "run the nightly report at 8:36",
    NOW,
  );
  assert.equal(dueAt, null);
  assert.equal(textWithoutDue, "run the nightly report");
  assert.deepEqual(ambiguousBareClock, {
    dayHint: null,
    hour: 8,
    minute: 36,
  });
});

test("parseBestieReminderMeridiemReply accepts PM/AM forms", () => {
  assert.deepEqual(parseBestieReminderMeridiemReply("PM"), { meridiem: "pm" });
  assert.deepEqual(parseBestieReminderMeridiemReply("am"), { meridiem: "am" });
  assert.deepEqual(parseBestieReminderMeridiemReply("8:45 PM"), {
    hour: 8,
    meridiem: "pm",
    minute: 45,
  });
  assert.equal(parseBestieReminderMeridiemReply("how are you?"), null);
  assert.equal(messageLooksLikeBestieReminderMeridiemReply("PM"), true);
});

test("resolveBestieBareClockDueAt maps evening PM correctly", () => {
  // Sunday 8:40 PM PT — confirm 8:45 PM → tonight (not next-morning AM).
  const evening = Date.parse("2026-09-27T20:40:00.000-07:00");
  const dueAt = resolveBestieBareClockDueAt(
    { dayHint: null, hour: 8, minute: 45 },
    "pm",
    evening,
  );
  const due = new Date(dueAt * 1000);
  assert.equal(due.getHours(), 20);
  assert.equal(due.getMinutes(), 45);
  assert.equal(due.toDateString(), new Date(evening).toDateString());
  assert.ok(due.getTime() > evening);
});

test("reconcileReminderDueAtWithStatedMeridiem fixes PM prose + AM fence", () => {
  const evening = Date.parse("2026-09-27T20:40:00.000-07:00");
  const wrongAm = Math.floor(
    Date.parse("2026-09-28T08:45:00.000-07:00") / 1000,
  );
  const fixed = reconcileReminderDueAtWithStatedMeridiem(
    wrongAm,
    "Got it — reminder set for 8:45 PM.",
    { dayHint: null, hour: 8, minute: 45 },
    evening,
  );
  assert.ok(fixed);
  const due = new Date(fixed * 1000);
  assert.equal(due.getHours(), 20);
  assert.equal(due.getMinutes(), 45);
  assert.notEqual(fixed, wrongAm);
});
