import assert from "node:assert/strict";
import test from "node:test";
import { projectTaskMilestone } from "./calendarProjection.ts";
import { planningProject, planningTask } from "./testFixtures.ts";

test("projects an active plan task as one stable linked all-day milestone", () => {
  const milestone = projectTaskMilestone(planningTask, planningProject);

  assert.deepEqual(milestone, {
    kind: "planTask",
    id: "plan-task:task-a",
    title: "1.1 Prepare logistics support plan",
    date: "2026-08-04",
    allDay: true,
    visualStatus: "inProgress",
    owner: "Logistics Officer",
    projectId: "deployment-1",
    taskId: "task-a",
    href: "/plans/deployment-1?task=task-a",
  });
});

test("does not project draft plans, summary, cancelled, or undated tasks", () => {
  assert.equal(
    projectTaskMilestone(planningTask, {
      ...planningProject,
      status: "draft",
    }),
    null,
  );
  assert.equal(
    projectTaskMilestone({ ...planningTask, isSummary: true }, planningProject),
    null,
  );
  assert.equal(
    projectTaskMilestone(
      { ...planningTask, status: "cancelled" },
      planningProject,
    ),
    null,
  );
  assert.equal(
    projectTaskMilestone({ ...planningTask, dueDate: null }, planningProject),
    null,
  );
});

test("completion and date movement update the milestone without changing identity", () => {
  const completed = projectTaskMilestone(
    { ...planningTask, status: "complete", percentComplete: 100 },
    planningProject,
  );
  const moved = projectTaskMilestone(
    { ...planningTask, dueDate: "2026-08-07" },
    planningProject,
  );

  assert.equal(completed?.id, "plan-task:task-a");
  assert.equal(completed?.visualStatus, "complete");
  assert.equal(moved?.id, "plan-task:task-a");
  assert.equal(moved?.date, "2026-08-07");
});
