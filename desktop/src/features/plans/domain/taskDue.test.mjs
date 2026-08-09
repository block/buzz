import assert from "node:assert/strict";
import test from "node:test";
import { automaticStartAt, dueAutomaticTasks } from "./taskDue.ts";
import { planningTask } from "./testFixtures.ts";
import { defaultTaskDetails } from "./extendedContracts.ts";

test("starts hybrid work one hour before due and defaults date-only work to 1600", () => {
  const details = {
    ...defaultTaskDetails(planningTask),
    executionMode: "hybrid",
    agentId: "builtin:command-operations",
    dueTime: "16:00",
  };
  const timing = automaticStartAt(planningTask, details, "Australia/Sydney");
  assert.match(timing.dueAt, /T16:00:00\+10:00$/);
  assert.match(timing.startAt, /T05:00:00\.000Z$/);
});

test("returns overdue unclaimed work and excludes terminal or manual work", () => {
  const details = {
    ...defaultTaskDetails(planningTask),
    executionMode: "hybrid",
    agentId: "builtin:command-operations",
    dueTime: null,
  };
  const due = dueAutomaticTasks({
    tasks: [planningTask],
    details: [details],
    executions: [],
    now: "2026-08-04T06:01:00Z",
    timeZoneFor: () => "Australia/Sydney",
  });
  assert.equal(due.length, 1);
  assert.equal(due[0].lateStart, true);
  assert.equal(
    dueAutomaticTasks({
      tasks: [{ ...planningTask, status: "complete" }],
      details: [details],
      executions: [],
      now: "2026-08-04T06:01:00Z",
      timeZoneFor: () => "Australia/Sydney",
    }).length,
    0,
  );
  assert.equal(
    dueAutomaticTasks({
      tasks: [planningTask],
      details: [{ ...details, executionMode: "manual" }],
      executions: [],
      now: "2026-08-04T06:01:00Z",
      timeZoneFor: () => "Australia/Sydney",
    }).length,
    0,
  );
});
