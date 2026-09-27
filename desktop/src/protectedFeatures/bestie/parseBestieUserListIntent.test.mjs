import assert from "node:assert/strict";
import test from "node:test";

import {
  messageLooksLikeBestieListRequest,
  parseBestieDueAtFromText,
  parseBestieUserListIntent,
} from "./parseBestieUserListIntent.ts";

const NOW = Date.parse("2026-09-26T15:00:00.000-07:00");

test("parses remind me … in N minutes", () => {
  const intent = parseBestieUserListIntent(
    "Remind me to stretch in 20 minutes",
    NOW,
  );
  assert.equal(intent?.op, "add");
  assert.equal(intent?.items[0].kind, "reminder");
  assert.equal(intent?.items[0].text, "stretch");
  assert.equal(intent?.items[0].dueAt, Math.floor(NOW / 1000) + 20 * 60);
});

test("parses remind me … tomorrow at time", () => {
  const intent = parseBestieUserListIntent(
    "remind me to call mom tomorrow at 9am",
    NOW,
  );
  assert.equal(intent?.op, "add");
  assert.equal(intent?.items[0].text, "call mom");
  assert.ok(intent?.items[0].dueAt);
  const due = new Date(intent.items[0].dueAt * 1000);
  assert.equal(due.getHours(), 9);
  assert.ok(due.getTime() > NOW);
  assert.notEqual(due.toDateString(), new Date(NOW).toDateString());
});

test("parses add todo", () => {
  const intent = parseBestieUserListIntent("Add a todo: ship phase 3", NOW);
  assert.deepEqual(intent, {
    items: [{ kind: "todo", text: "ship phase 3" }],
    op: "add",
  });
});

test("parses complete / remove by text", () => {
  assert.deepEqual(parseBestieUserListIntent("mark todo ship phase 3 done", NOW), {
    kind: "todo",
    op: "complete-match",
    text: "ship phase 3",
  });
  assert.deepEqual(parseBestieUserListIntent("remove reminder call mom", NOW), {
    kind: "reminder",
    op: "remove-match",
    text: "call mom",
  });
});

test("ignores free-form chat", () => {
  assert.equal(parseBestieUserListIntent("how are my reminders looking?", NOW), null);
  assert.equal(messageLooksLikeBestieListRequest("just chatting"), false);
});

test("parseBestieDueAtFromText handles at-time roll to tomorrow", () => {
  // 3pm already passed relative to NOW (3pm PT) → next day.
  const { dueAt, textWithoutDue } = parseBestieDueAtFromText("pay rent at 2pm", NOW);
  assert.equal(textWithoutDue, "pay rent");
  assert.ok(dueAt);
  const due = new Date(dueAt * 1000);
  assert.equal(due.getHours(), 14);
  assert.ok(due.getTime() > NOW);
});
