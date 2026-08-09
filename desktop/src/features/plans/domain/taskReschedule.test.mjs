import assert from "node:assert/strict";
import test from "node:test";
import { planningTask } from "./testFixtures.ts";
import { requestTaskMove } from "./taskReschedule.ts";

test("moves a task while preserving its planned date span", () => {
  const moved = requestTaskMove(
    {
      ...planningTask,
      plannedStart: "2026-08-03",
      dueDate: "2026-08-05",
      fixedStart: null,
    },
    "2026-08-07",
    false,
    "2026-07-29T10:00:00.000Z",
  );
  assert.equal(moved.plannedStart, "2026-08-07");
  assert.equal(moved.dueDate, "2026-08-09");
  assert.equal(moved.fixedStart, "2026-08-07");
});

test("rejects drag rescheduling for locked tasks", () => {
  assert.throws(
    () =>
      requestTaskMove(
        { ...planningTask, plannedStart: "2026-08-03", dueDate: "2026-08-05" },
        "2026-08-07",
        true,
        "2026-07-29T10:00:00.000Z",
      ),
    /locked/,
  );
});
