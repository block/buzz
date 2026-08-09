import assert from "node:assert/strict";
import test from "node:test";
import {
  buildMissionConstraintEvent,
  buildPlanningPlaybookEvent,
  buildPlanningProjectEvent,
  buildPlanningTaskArtifactEvent,
  buildPlanningTaskDetailsEvent,
  buildPlanningTaskEvent,
  buildPlanningTaskExecutionEvent,
  parseRelayMissionConstraint,
  parseRelayPlanningPlaybook,
  parseRelayPlanningProject,
  parseRelayPlanningTaskArtifact,
  parseRelayPlanningTaskDetails,
  parseRelayPlanningTask,
  parseRelayPlanningTaskExecution,
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

const timestamp = "2026-07-29T00:00:00Z";
const details = {
  schemaVersion: 1,
  id: "details:task-a",
  projectId: "deployment-1",
  taskId: "task-a",
  department: "MEO",
  position: "Marine Engineering Officer",
  individual: null,
  agentId: "operations",
  dueTime: "16:00",
  executionMode: "hybrid",
  outputType: "docx",
  playbookId: null,
  playbookRevisionId: null,
  locked: false,
  createdAt: timestamp,
  updatedAt: timestamp,
};
const playbook = {
  schemaVersion: 1,
  id: "pre-departure",
  title: "Pre-Departure",
  description: "Prepare the ship for sailing.",
  status: "active",
  revisionId: "revision-1",
  taskTemplates: [],
  createdAt: timestamp,
  updatedAt: timestamp,
};
const execution = {
  schemaVersion: 1,
  id: "execution-1",
  projectId: "deployment-1",
  taskId: "task-a",
  status: "queued",
  mode: "hybrid",
  summary: null,
  body: null,
  missingInputs: [],
  assumptions: [],
  provider: null,
  model: null,
  startedAt: timestamp,
  completedAt: null,
  error: null,
  lateStart: false,
};
const artifact = {
  schemaVersion: 1,
  id: "artifact-1",
  projectId: "deployment-1",
  taskId: "task-a",
  executionId: "execution-1",
  fileName: "plan.docx",
  path: "/tmp/plan.docx",
  format: "docx",
  storageState: "local_pending_icloud",
  agentId: "operations",
  provider: "litellm",
  model: "gpt-5.4",
  summary: "Draft plan.",
  missingInputWarning: null,
  sha256: "a".repeat(64),
  sizeBytes: 1024,
  createdAt: timestamp,
};

test("project-execution events use stable companion tags", async () => {
  const detailEvent = await buildPlanningTaskDetailsEvent(details);
  const playbookEvent = await buildPlanningPlaybookEvent(playbook);
  const executionEvent = await buildPlanningTaskExecutionEvent(execution);
  const artifactEvent = await buildPlanningTaskArtifactEvent(artifact);

  assert.deepEqual(detailEvent.tags, [
    ["d", "details:task-a"],
    ["project", "deployment-1"],
    ["task", "task-a"],
  ]);
  assert.deepEqual(playbookEvent.tags, [
    ["d", "pre-departure"],
    ["revision", "revision-1"],
  ]);
  assert.deepEqual(executionEvent.tags, [
    ["d", "execution-1"],
    ["project", "deployment-1"],
    ["task", "task-a"],
  ]);
  assert.deepEqual(artifactEvent.tags, [
    ["d", "artifact-1"],
    ["project", "deployment-1"],
    ["task", "task-a"],
    ["execution", "execution-1"],
  ]);
  assert.equal(parseRelayPlanningTaskDetails(detailEvent)?.taskId, "task-a");
  assert.equal(parseRelayPlanningPlaybook(playbookEvent)?.id, "pre-departure");
  assert.equal(
    parseRelayPlanningTaskExecution(executionEvent)?.id,
    "execution-1",
  );
  assert.equal(parseRelayPlanningTaskArtifact(artifactEvent)?.id, "artifact-1");
});

test("project-execution decoders reject cross-tag substitution", async () => {
  const detailEvent = await buildPlanningTaskDetailsEvent(details);
  assert.equal(
    parseRelayPlanningTaskDetails({
      ...detailEvent,
      tags: detailEvent.tags.map((tag) =>
        tag[0] === "task" ? ["task", "other"] : tag,
      ),
    }),
    null,
  );
});
