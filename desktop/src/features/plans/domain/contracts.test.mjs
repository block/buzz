import assert from "node:assert/strict";
import test from "node:test";
import {
  parseMissionConstraint,
  parsePlanningProject,
  parsePlanningTask,
} from "./contracts.ts";
import {
  missionConstraint,
  planningProject,
  planningTask,
} from "./testFixtures.ts";

test("accepts the exact immutable planning contracts", () => {
  assert.deepEqual(parsePlanningProject(planningProject), planningProject);
  assert.deepEqual(parsePlanningTask(planningTask), planningTask);
  assert.deepEqual(
    parseMissionConstraint(missionConstraint),
    missionConstraint,
  );
});

test("rejects unknown fields, progress bounds, and self references", () => {
  assert.throws(() => parsePlanningTask({ ...planningTask, unknown: "field" }));
  assert.throws(() =>
    parsePlanningTask({ ...planningTask, percentComplete: 101 }),
  );
  assert.throws(() =>
    parsePlanningTask({ ...planningTask, dependencyIds: [planningTask.id] }),
  );
  assert.throws(() =>
    parsePlanningTask({ ...planningTask, parentTaskId: planningTask.id }),
  );
});

test("constraint candidates require explicit links and disposition", () => {
  assert.throws(() =>
    parseMissionConstraint({
      ...missionConstraint,
      status: "riskCandidate",
      dispositionNote: null,
    }),
  );
  assert.throws(() =>
    parseMissionConstraint({
      ...missionConstraint,
      linkedMissionRequirementId: null,
      linkedCapabilityId: null,
      linkedTaskId: null,
    }),
  );
});
