import assert from "node:assert/strict";
import test from "node:test";

import {
  addBestieListItem,
  emptyBestieListState,
} from "./bestieListStorage.ts";
import {
  evaluateBestieWake,
  nextOpenReminderDueAt,
} from "./bestieWakeScheduler.ts";

test("evaluateBestieWake nudges on due reminders and open todos", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Due soon", dueAt: 100 },
    50,
  );
  state = addBestieListItem(state, { kind: "todo", text: "Open item" }, 50);
  const first = evaluateBestieWake(state, 100);
  assert.equal(first.shouldWakeAgent, true);
  assert.ok(first.nudge);
  assert.equal(first.nudge.reason, "due-reminder");
  assert.equal(first.nudge.title, "Reminder");
  assert.equal(first.nudge.body, "Due soon");
  assert.doesNotMatch(first.nudge.body, /^Reminder:/i);
  assert.doesNotMatch(first.nudge.body, /to-do|Open item/i);

  const same = evaluateBestieWake(state, 100, {
    previousNudgeId: first.nudge.id,
  });
  assert.equal(same.nudge, null);
  assert.equal(same.shouldWakeAgent, false);
});

test("evaluateBestieWake due-reminder body never includes open todo counts", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Run my reports", dueAt: 100 },
    50,
  );
  state = addBestieListItem(state, { kind: "todo", text: "Buy milk" }, 50);
  state = addBestieListItem(state, { kind: "todo", text: "Call bank" }, 50);
  state = addBestieListItem(state, { kind: "todo", text: "Ship fix" }, 50);
  const result = evaluateBestieWake(state, 100);
  assert.equal(result.nudge?.reason, "due-reminder");
  assert.equal(result.nudge?.title, "Reminder");
  assert.equal(result.nudge?.body, "Run my reports");
  assert.doesNotMatch(result.nudge?.body ?? "", /to-do|Buy milk|3 open/i);
});

test("evaluateBestieWake starred todos-only is check-in (no auto-open reason)", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "todo", starred: true, text: "Only todo" },
    50,
  );
  const result = evaluateBestieWake(state, 100);
  assert.equal(result.nudge?.reason, "check-in");
  assert.equal(result.nudge?.title, "Assistant check-in");
  assert.equal(result.nudge?.body, "Starred to-do: Only todo");
});

test("evaluateBestieWake ignores unstarred todos for proactive check-in", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(state, { kind: "todo", text: "Backlog item" }, 50);
  const result = evaluateBestieWake(state, 100);
  assert.equal(result.nudge, null);
  assert.equal(result.shouldWakeAgent, false);
});

test("evaluateBestieWake is quiet when lists are empty", () => {
  const result = evaluateBestieWake(emptyBestieListState(), 100);
  assert.equal(result.nudge, null);
  assert.equal(result.shouldWakeAgent, false);
});

test("nextOpenReminderDueAt picks the soonest future due", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Later", dueAt: 300 },
    50,
  );
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Sooner", dueAt: 200 },
    50,
  );
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Past", dueAt: 50 },
    50,
  );
  assert.equal(nextOpenReminderDueAt(state, 100), 200);
  assert.equal(nextOpenReminderDueAt(emptyBestieListState(), 100), null);
});
