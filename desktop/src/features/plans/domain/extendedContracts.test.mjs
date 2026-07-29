import assert from "node:assert/strict";
import test from "node:test";

import {
  defaultTaskDetails,
  parsePlanningPlaybook,
  parsePlanningTaskArtifact,
  parsePlanningTaskDetails,
  parsePlanningTaskExecution,
} from "./extendedContracts.ts";
import { planningTask } from "./testFixtures.ts";

const timestamp = "2026-07-29T00:00:00Z";

export const taskDetails = {
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

export const playbook = {
  schemaVersion: 1,
  id: "pre-departure",
  title: "Pre-Departure",
  description: "Prepare the ship for sailing.",
  status: "active",
  revisionId: "revision-1",
  taskTemplates: [
    {
      id: "navigation-plan",
      title: "Navigation plan briefed",
      instructions: "Brief the approved navigation plan.",
      timing: "before",
      offsetMinutes: 1440,
      durationMinutes: 60,
      dependencyIds: [],
      department: "Navigation",
      position: "Navigation Officer",
      agentId: "navigation",
      outputType: "response",
      reschedulable: true,
      locked: false,
      linkedCapabilityId: null,
      linkedMissionRequirementId: null,
    },
    {
      id: "departure-brief",
      title: "Departure pilotage briefed",
      instructions: "Brief the departure pilotage.",
      timing: "before",
      offsetMinutes: 720,
      durationMinutes: 60,
      dependencyIds: ["navigation-plan"],
      department: "Navigation",
      position: "Navigation Officer",
      agentId: "navigation",
      outputType: "pptx",
      reschedulable: true,
      locked: false,
      linkedCapabilityId: null,
      linkedMissionRequirementId: null,
    },
  ],
  createdAt: timestamp,
  updatedAt: timestamp,
};

export const taskExecution = {
  schemaVersion: 1,
  id: "execution-1",
  projectId: "deployment-1",
  taskId: "task-a",
  status: "forReview",
  mode: "hybrid",
  summary: "Draft logistics plan prepared.",
  body: "The plan uses the evidence available at execution time.",
  missingInputs: ["MEO defect update"],
  assumptions: ["Port services remain available"],
  provider: "litellm",
  model: "gpt-5.4",
  startedAt: timestamp,
  completedAt: "2026-07-29T00:05:00Z",
  error: null,
  lateStart: false,
};

export const taskArtifact = {
  schemaVersion: 1,
  id: "artifact-1",
  projectId: "deployment-1",
  taskId: "task-a",
  executionId: "execution-1",
  fileName: "logistics-support-plan.docx",
  path: "/Users/test/Command Adviser/logistics-support-plan.docx",
  format: "docx",
  storageState: "icloud",
  agentId: "operations",
  provider: "litellm",
  model: "gpt-5.4",
  summary: "Draft logistics plan.",
  missingInputWarning: "MEO defect update",
  sha256: "a".repeat(64),
  sizeBytes: 2048,
  createdAt: timestamp,
};

test("accepts exact immutable project-execution contracts", () => {
  assert.deepEqual(parsePlanningTaskDetails(taskDetails), taskDetails);
  assert.deepEqual(parsePlanningPlaybook(playbook), playbook);
  assert.deepEqual(parsePlanningTaskExecution(taskExecution), taskExecution);
  assert.deepEqual(parsePlanningTaskArtifact(taskArtifact), taskArtifact);
});

test("rejects unknown fields, invalid time, and unsafe artifact paths", () => {
  assert.throws(() =>
    parsePlanningTaskDetails({ ...taskDetails, unexpected: true }),
  );
  assert.throws(() =>
    parsePlanningTaskDetails({ ...taskDetails, dueTime: "4pm" }),
  );
  assert.throws(() =>
    parsePlanningTaskArtifact({ ...taskArtifact, path: "relative/file.docx" }),
  );
  assert.throws(() =>
    parsePlanningTaskArtifact({
      ...taskArtifact,
      sha256: "not-a-sha256",
    }),
  );
});

test("rejects playbook self dependencies and oversized missing-input lists", () => {
  assert.throws(() =>
    parsePlanningPlaybook({
      ...playbook,
      taskTemplates: [
        {
          ...playbook.taskTemplates[0],
          dependencyIds: ["navigation-plan"],
        },
      ],
    }),
  );
  assert.throws(() =>
    parsePlanningTaskExecution({
      ...taskExecution,
      missingInputs: Array.from(
        { length: 129 },
        (_, index) => `input-${index}`,
      ),
    }),
  );
});

test("maps existing tasks to compatible details defaults", () => {
  assert.deepEqual(defaultTaskDetails(planningTask), {
    schemaVersion: 1,
    id: "details:task-a",
    projectId: "deployment-1",
    taskId: "task-a",
    department: "Logistics Officer",
    position: "Logistics Officer",
    individual: null,
    agentId: null,
    dueTime: null,
    executionMode: "manual",
    outputType: "response",
    playbookId: null,
    playbookRevisionId: null,
    locked: false,
    createdAt: "2026-07-29T00:00:00Z",
    updatedAt: "2026-07-29T00:00:00Z",
  });
});
