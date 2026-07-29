import assert from "node:assert/strict";
import test from "node:test";
import { schedulePlaybook } from "./playbookSchedule.ts";

const playbook = {
  schemaVersion: 1,
  id: "pre-departure",
  title: "Pre-Departure",
  description: "Prepare to sail",
  status: "active",
  revisionId: "rev-1",
  taskTemplates: [
    {
      id: "rounds",
      title: "Securing for sea rounds",
      instructions: "Complete rounds.",
      timing: "before",
      offsetMinutes: 480,
      durationMinutes: 480,
      dependencyIds: [],
      department: "XO",
      position: "Executive Officer",
      agentId: null,
      outputType: "response",
      reschedulable: true,
      locked: false,
      linkedCapabilityId: null,
      linkedMissionRequirementId: null,
    },
  ],
  createdAt: "2026-07-29T00:00:00Z",
  updatedAt: "2026-07-29T00:00:00Z",
};

test("places an eight-hour alongside predecessor on Friday for Monday sailing", () => {
  const [task] = schedulePlaybook(playbook, {
    anchorDate: "2026-08-10",
    anchorTime: "08:00",
    routine: "alongside",
    timeZone: "Australia/Sydney",
  });
  assert.equal(task.plannedStart, "2026-08-07");
  assert.equal(task.dueDate, "2026-08-07");
  assert.equal(task.plannedStartTime, "08:00");
  assert.equal(task.dueTime, "16:00");
});

test("Sunday Sea work starts no earlier than 1200", () => {
  const [task] = schedulePlaybook(
    {
      ...playbook,
      taskTemplates: [
        {
          ...playbook.taskTemplates[0],
          offsetMinutes: 0,
          durationMinutes: 120,
        },
      ],
    },
    {
      anchorDate: "2026-08-09",
      anchorTime: "14:00",
      routine: "atSea",
      timeZone: "Australia/Sydney",
    },
  );
  assert.equal(task.plannedStart, "2026-08-09");
  assert.equal(task.plannedStartTime, "12:00");
});
