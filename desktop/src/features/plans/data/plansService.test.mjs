import assert from "node:assert/strict";
import test from "node:test";
import {
  assertProjectCanDelete,
  fetchPlans,
  publishPlanningTask,
} from "./plansService.ts";
import {
  buildMissionConstraintEvent,
  buildPlanningProjectEvent,
  buildPlanningTaskDetailsEvent,
  buildPlanningTaskEvent,
  setPlanningEventSignerForTests,
} from "../domain/eventCodec.ts";
import {
  missionConstraint,
  planningProject,
  planningTask,
} from "../domain/testFixtures.ts";

setPlanningEventSignerForTests(async (input) => ({
  id: "signed",
  pubkey: "owner",
  created_at: input.createdAt ?? 1,
  kind: input.kind,
  tags: input.tags,
  content: input.content,
  sig: "sig",
}));

test("fetch uses explicit author and kind queries and excludes orphans", async () => {
  const calls = [];
  const project = await buildPlanningProjectEvent(planningProject);
  const task = await buildPlanningTaskEvent(planningTask);
  const orphan = await buildPlanningTaskEvent({
    ...planningTask,
    id: "orphan",
    projectId: "missing",
  });
  const constraint = await buildMissionConstraintEvent(missionConstraint);
  const details = await buildPlanningTaskDetailsEvent({
    schemaVersion: 1,
    id: "details:task-a",
    projectId: "deployment-1",
    taskId: "task-a",
    department: "SO",
    position: "Supply Officer",
    individual: null,
    agentId: null,
    dueTime: "16:00",
    executionMode: "hybrid",
    outputType: "response",
    playbookId: null,
    playbookRevisionId: null,
    locked: false,
    createdAt: "2026-07-29T00:00:00Z",
    updatedAt: "2026-07-29T00:00:00Z",
  });
  const responses = [
    [project],
    [task, orphan],
    [constraint],
    [details],
    [],
    [],
    [],
  ];
  const result = await fetchPlans("owner", {
    fetchEvents: async (filter) => {
      calls.push(filter);
      return responses.shift() ?? [];
    },
  });
  assert.deepEqual(
    calls.map((call) => [call.kinds, call.authors]),
    [
      [[30632], ["owner"]],
      [[30633], ["owner"]],
      [[30634], ["owner"]],
      [[30635], ["owner"]],
      [[30636], ["owner"]],
      [[30637], ["owner"]],
      [[30638], ["owner"]],
    ],
  );
  assert.deepEqual(
    result.projects.map((item) => item.id),
    ["deployment-1"],
  );
  assert.deepEqual(
    result.tasks.map((item) => item.id),
    ["task-a"],
  );
  assert.deepEqual(
    result.constraints.map((item) => item.id),
    ["constraint-1"],
  );
  assert.equal(result.details[0].department, "SO");
  assert.deepEqual(result.playbooks, []);
  assert.deepEqual(result.executions, []);
  assert.deepEqual(result.artifacts, []);
});

test("existing tasks receive durable compatible assignment defaults", async () => {
  const project = await buildPlanningProjectEvent(planningProject);
  const task = await buildPlanningTaskEvent(planningTask);
  const responses = [[project], [task], [], [], [], [], []];
  const result = await fetchPlans("owner", {
    fetchEvents: async () => responses.shift() ?? [],
  });
  assert.equal(result.details.length, 1);
  assert.equal(result.details[0].taskId, planningTask.id);
  assert.equal(result.details[0].department, planningTask.owner);
  assert.equal(result.details[0].executionMode, "manual");
});

test("publication advances the relay head and deletion cannot orphan records", async () => {
  const old = {
    ...(await buildPlanningTaskEvent(planningTask)),
    created_at: 44,
  };
  let published;
  await publishPlanningTask("owner", planningTask, {
    fetchEvents: async () => [old],
    publishEvent: async (event) => {
      published = event;
      return event;
    },
  });
  assert.ok(published.created_at > 44);
  assert.throws(() =>
    assertProjectCanDelete(planningProject.id, [planningTask], []),
  );
  assert.doesNotThrow(() => assertProjectCanDelete(planningProject.id, [], []));
});
