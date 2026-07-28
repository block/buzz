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
  const responses = [[project], [task, orphan], [constraint]];
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
