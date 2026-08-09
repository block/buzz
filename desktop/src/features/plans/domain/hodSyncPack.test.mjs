import assert from "node:assert/strict";
import test from "node:test";
import { buildHodSyncPack } from "./hodSyncPack.ts";
import { planningProject, planningTask } from "./testFixtures.ts";
import { defaultTaskDetails } from "./extendedContracts.ts";

test("groups HOD work and orders overdue then critical then ordinary", () => {
  const tasks = [
    { ...planningTask, id: "ordinary", dueDate: "2026-08-10" },
    { ...planningTask, id: "critical", dueDate: "2026-08-09" },
    { ...planningTask, id: "overdue", dueDate: "2026-07-28" },
    { ...planningTask, id: "other", dueDate: "2026-08-08" },
  ];
  const details = tasks.map((task) => ({
    ...defaultTaskDetails(task),
    department: task.id === "other" ? "Navigation" : "MEO",
    position: task.id === "other" ? "Navigator" : "Marine Engineering Officer",
  }));
  const schedule = tasks.map((task) => ({
    taskId: task.id,
    earlyStart: task.plannedStart,
    earlyFinish: task.dueDate,
    lateStart: task.plannedStart,
    lateFinish: task.dueDate,
    totalFloatWorkdays: task.id === "critical" ? 0 : 2,
    critical: task.id === "critical",
  }));

  const pack = buildHodSyncPack(
    planningProject,
    tasks,
    details,
    schedule,
    "2026-07-29T09:00:00+10:00",
  );

  assert.deepEqual(
    pack.groups.MEO.map((item) => item.task.id),
    ["overdue", "critical", "ordinary"],
  );
  assert.equal(pack.groups.other[0].task.id, "other");
  assert.deepEqual(
    pack.combined.map((item) => item.task.id),
    ["overdue", "critical", "ordinary", "other"],
  );
});
