import assert from "node:assert/strict";
import test from "node:test";

import {
  countDue,
  dueSince,
  groupReminders,
  isDue,
} from "./reminderFilters.ts";

/**
 * Build a Reminder fixture. `notBefore` and `status` are the only fields the
 * filters read besides `createdAt` (for done-group ordering).
 */
function reminder({ id = "r", notBefore, status = "pending", createdAt = 0 }) {
  return {
    id,
    eventId: `${id}-evt`,
    notBefore,
    createdAt,
    content: { status },
  };
}

const NOW = 1_000;

test("isDue_pending_reminder_at_now_is_due", () => {
  assert.equal(isDue(reminder({ notBefore: NOW }), NOW), true);
});

test("isDue_pending_reminder_in_future_is_not_due", () => {
  assert.equal(isDue(reminder({ notBefore: NOW + 1 }), NOW), false);
});

test("isDue_done_reminder_in_past_is_not_due", () => {
  assert.equal(
    isDue(reminder({ notBefore: NOW - 100, status: "done" }), NOW),
    false,
  );
});

test("isDue_reminder_without_notBefore_is_not_due", () => {
  assert.equal(isDue(reminder({ notBefore: undefined }), NOW), false);
});

test("countDue_counts_only_due_pending_reminders", () => {
  const reminders = [
    reminder({ id: "a", notBefore: NOW - 1 }),
    reminder({ id: "b", notBefore: NOW }),
    reminder({ id: "c", notBefore: NOW + 1 }),
    reminder({ id: "d", notBefore: NOW - 1, status: "done" }),
    reminder({ id: "e", notBefore: NOW - 1, status: "cancelled" }),
  ];
  assert.equal(countDue(reminders, NOW), 2);
});

test("countDue_empty_list_returns_zero", () => {
  assert.equal(countDue([], NOW), 0);
});

// dueSince — the watermark fire window. Strict lower bound `>`, inclusive `<=`.
test("dueSince_fires_reminder_crossing_window_since_watermark", () => {
  const due = dueSince([reminder({ notBefore: 50 })], 0, 100);
  assert.equal(due.length, 1);
});

test("dueSince_excludes_reminder_at_exactly_the_watermark", () => {
  // notBefore === watermark fails the strict `>`: the seed-to-now first-launch
  // case must not replay an already-due reminder as a toast.
  const due = dueSince([reminder({ notBefore: 100 })], 100, 200);
  assert.equal(due.length, 0);
});

test("dueSince_includes_reminder_at_exactly_now", () => {
  const due = dueSince([reminder({ notBefore: 100 })], 0, 100);
  assert.equal(due.length, 1);
});

test("dueSince_excludes_future_reminder_past_now", () => {
  const due = dueSince([reminder({ notBefore: 150 })], 0, 100);
  assert.equal(due.length, 0);
});

test("dueSince_excludes_non_pending_in_window", () => {
  const due = dueSince(
    [
      reminder({ id: "done", notBefore: 50, status: "done" }),
      reminder({ id: "cancel", notBefore: 50, status: "cancelled" }),
    ],
    0,
    100,
  );
  assert.equal(due.length, 0);
});

test("dueSince_excludes_reminder_without_notBefore", () => {
  const due = dueSince([reminder({ notBefore: undefined })], 0, 100);
  assert.equal(due.length, 0);
});

test("dueSince_empty_watermark_window_returns_empty", () => {
  // watermark === now: no new reminder can have crossed since last check.
  const due = dueSince([reminder({ notBefore: 50 })], 100, 100);
  assert.equal(due.length, 0);
});

// groupReminders — bucketing. It reads the real wall clock for "now", so
// fixtures are anchored to real time. "Today" = up to local end-of-day.
const realNow = Math.floor(Date.now() / 1_000);
const realEndOfTodaySecs = (() => {
  const d = new Date();
  d.setHours(23, 59, 59, 999);
  return Math.floor(d.getTime() / 1_000);
})();

test("groupReminders_buckets_overdue_today_upcoming", () => {
  // A timestamp strictly between now and end-of-day, robust to running near
  // midnight: the midpoint can never coincide with either boundary.
  const todaySecs = Math.floor((realNow + realEndOfTodaySecs) / 2);
  const groups = groupReminders([
    reminder({ id: "over", notBefore: realNow - 100 }),
    reminder({ id: "today", notBefore: todaySecs }),
    reminder({ id: "soon", notBefore: realEndOfTodaySecs + 86_400 }),
  ]);
  const labels = groups.map((g) => g.label);
  assert.deepEqual(labels, ["Overdue", "Today", "Upcoming"]);
});

test("groupReminders_omits_empty_buckets", () => {
  const groups = groupReminders([reminder({ notBefore: realNow - 100 })]);
  assert.deepEqual(
    groups.map((g) => g.label),
    ["Overdue"],
  );
});

test("groupReminders_excludes_done_when_includeDone_false", () => {
  const groups = groupReminders([
    reminder({ notBefore: realNow - 100, status: "done" }),
  ]);
  assert.equal(groups.length, 0);
});

test("groupReminders_appends_completed_group_when_includeDone_true", () => {
  const groups = groupReminders(
    [
      reminder({ id: "over", notBefore: realNow - 100 }),
      reminder({ id: "d1", status: "done", createdAt: 1 }),
      reminder({ id: "d2", status: "done", createdAt: 2 }),
    ],
    true,
  );
  const completed = groups.find((g) => g.label === "Completed");
  assert.ok(completed);
  // Done reminders are sorted newest-first by createdAt.
  assert.deepEqual(
    completed.reminders.map((r) => r.id),
    ["d2", "d1"],
  );
});

test("groupReminders_never_surfaces_cancelled_reminders", () => {
  const groups = groupReminders(
    [reminder({ notBefore: realNow - 100, status: "cancelled" })],
    true,
  );
  assert.equal(groups.length, 0);
});

test("groupReminders_empty_list_returns_empty", () => {
  assert.deepEqual(groupReminders([]), []);
});

test("groupReminders_buckets_epoch_zero_notBefore_as_overdue", () => {
  // Guards on `notBefore !== undefined`, matching isDue/dueSince: 0 is kept.
  const groups = groupReminders([reminder({ notBefore: 0 })]);
  assert.deepEqual(
    groups.map((g) => g.label),
    ["Overdue"],
  );
});
