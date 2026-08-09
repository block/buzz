import assert from "node:assert/strict";
import test from "node:test";
import { parsePlanningSchedule } from "./tauriPlans.ts";

const schedule = {
  tasks: [
    {
      taskId: "A",
      earliestStart: "2026-08-03",
      earliestFinish: "2026-08-04",
      latestStart: "2026-08-03",
      latestFinish: "2026-08-04",
      totalFloatWorkdays: 0,
      critical: true,
      overdue: false,
    },
  ],
  projectStart: "2026-08-03",
  projectFinish: "2026-08-10",
  projectDurationWorkdays: 6,
  missionReadyAtRisk: false,
};

test("strictly parses a finite exact schedule result", () => {
  assert.deepEqual(parsePlanningSchedule(schedule), schedule);
});

test("rejects unknown fields, missing fields, and non-finite float", () => {
  assert.throws(() => parsePlanningSchedule({ ...schedule, unknown: true }));
  assert.throws(() =>
    parsePlanningSchedule({
      ...schedule,
      tasks: [{ ...schedule.tasks[0], totalFloatWorkdays: Number.NaN }],
    }),
  );
  assert.throws(() =>
    parsePlanningSchedule({ ...schedule, projectFinish: "not-a-date" }),
  );
});
