import assert from "node:assert/strict";
import test from "node:test";

import {
  bestieJobFireSlotId,
  computeBestieJobNextDueAt,
  formatBestieJobRunPrompt,
} from "./bestieJobSchedule.ts";

test("once schedule returns dueAt before fire and null after", () => {
  assert.equal(
    computeBestieJobNextDueAt({ kind: "once", dueAt: 1000 }, 900),
    1000,
  );
  assert.equal(
    computeBestieJobNextDueAt({ kind: "once", dueAt: 1000 }, 1000, {
      afterRun: true,
    }),
    null,
  );
});

test("interval reschedules after run", () => {
  const next = computeBestieJobNextDueAt(
    { everySeconds: 300, kind: "interval" },
    1000,
    { afterRun: true },
  );
  assert.equal(next, 1300);
});

test("daily rolls to next day when after run", () => {
  // 2026-09-27 10:00 PT = use fixed UTC ms
  const from = Math.floor(Date.parse("2026-09-27T17:00:00.000Z") / 1000);
  const next = computeBestieJobNextDueAt(
    { hour: 9, kind: "daily", minute: 0 },
    from,
    { afterRun: true },
  );
  assert.ok(next != null && next > from);
});

test("fire slot id and run prompt format", () => {
  assert.equal(bestieJobFireSlotId("abc", 42), "abc@42");
  assert.match(
    formatBestieJobRunPrompt("Brief", "Do the thing"),
    /^\[Bestie job: Brief\]\n\nDo the thing$/,
  );
});
