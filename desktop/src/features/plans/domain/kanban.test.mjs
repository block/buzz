import assert from "node:assert/strict";
import test from "node:test";
import { kanbanColumnForTask, taskStatusForKanbanColumn } from "./kanban.ts";
import { planningTask } from "./testFixtures.ts";

const tasks = [
  { ...planningTask, id: "done", status: "complete", percentComplete: 100 },
  {
    ...planningTask,
    id: "dependent",
    status: "notStarted",
    percentComplete: 0,
    dependencyIds: ["done"],
  },
];

test("not-started work becomes ready only after every dependency completes", () => {
  assert.equal(kanbanColumnForTask(tasks[1], tasks), "ready");
  assert.equal(
    kanbanColumnForTask({ ...tasks[1], dependencyIds: ["not-done"] }, [
      ...tasks,
      {
        ...planningTask,
        id: "not-done",
        status: "inProgress",
        percentComplete: 50,
      },
    ]),
    "planned",
  );
});

test("active statuses map to visible workflow columns", () => {
  assert.equal(
    kanbanColumnForTask({ ...planningTask, status: "inProgress" }, tasks),
    "inProgress",
  );
  assert.equal(
    kanbanColumnForTask({ ...planningTask, status: "blocked" }, tasks),
    "waiting",
  );
  assert.equal(
    kanbanColumnForTask({ ...planningTask, status: "forReview" }, tasks),
    "forReview",
  );
  assert.equal(
    kanbanColumnForTask({ ...planningTask, status: "complete" }, tasks),
    "complete",
  );
});

test("column moves persist as the canonical task status", () => {
  assert.equal(taskStatusForKanbanColumn("planned"), "notStarted");
  assert.equal(taskStatusForKanbanColumn("ready"), "notStarted");
  assert.equal(taskStatusForKanbanColumn("inProgress"), "inProgress");
  assert.equal(taskStatusForKanbanColumn("waiting"), "blocked");
  assert.equal(taskStatusForKanbanColumn("forReview"), "forReview");
  assert.equal(taskStatusForKanbanColumn("complete"), "complete");
});
