import assert from "node:assert/strict";
import test from "node:test";
import {
  buildMissionConstraintEvent,
  buildPlanningProjectEvent,
  buildPlanningTaskEvent,
  parseRelayMissionConstraint,
  parseRelayPlanningProject,
  parseRelayPlanningTask,
  setPlanningEventSignerForTests,
} from "./eventCodec.ts";
import {
  missionConstraint,
  planningProject,
  planningTask,
} from "./testFixtures.ts";

setPlanningEventSignerForTests(async (input) => ({
  id: "signed",
  pubkey: "owner",
  created_at: input.createdAt ?? 1,
  kind: input.kind,
  tags: input.tags,
  content: input.content,
  sig: "sig",
}));

test("planning events use stable d, project, due, and monotonic timestamps", async () => {
  const project = await buildPlanningProjectEvent(planningProject, 100);
  const task = await buildPlanningTaskEvent(planningTask, 200);
  const constraint = await buildMissionConstraintEvent(missionConstraint, 300);
  assert.ok(project.created_at > 100);
  assert.deepEqual(project.tags[0], ["d", planningProject.id]);
  assert.ok(task.created_at > 200);
  assert.ok(task.tags.some((tag) => tag.join(":") === "project:deployment-1"));
  assert.ok(task.tags.some((tag) => tag.join(":") === "due:2026-08-04"));
  assert.ok(constraint.created_at > 300);
  assert.equal(parseRelayPlanningProject(project)?.id, planningProject.id);
  assert.equal(parseRelayPlanningTask(task)?.id, planningTask.id);
  assert.equal(
    parseRelayMissionConstraint(constraint)?.id,
    missionConstraint.id,
  );
});

test("relay decoders reject malformed tag and content combinations", async () => {
  const task = await buildPlanningTaskEvent(planningTask);
  assert.equal(
    parseRelayPlanningTask({
      ...task,
      tags: task.tags.map((tag) =>
        tag[0] === "project" ? ["project", "other"] : tag,
      ),
    }),
    null,
  );
  assert.equal(parseRelayPlanningTask({ ...task, content: "{}" }), null);
});
