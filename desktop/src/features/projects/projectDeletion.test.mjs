import assert from "node:assert/strict";
import test from "node:test";

import {
  buildProjectDeletionTemplate,
  canDeleteProject,
} from "./projectDeletion.ts";

const AGENT = "a".repeat(64);
const OWNER = "b".repeat(64);
const OTHER = "c".repeat(64);
const PROJECT_ADDRESS = `30621:${AGENT}:platform`;
const project = {
  name: "Platform",
  owner: AGENT,
  projectAddress: PROJECT_ADDRESS,
};

test("project deletion capability includes direct and agent owners", () => {
  assert.equal(canDeleteProject(project, AGENT, undefined), true);
  assert.equal(
    canDeleteProject(project, OWNER, { [AGENT]: { ownerPubkey: OWNER } }),
    true,
  );
});

test("project deletion capability rejects unrelated viewers", () => {
  assert.equal(canDeleteProject(project, OWNER, undefined), false);
  assert.equal(
    canDeleteProject(project, OTHER, { [AGENT]: { ownerPubkey: OWNER } }),
    false,
  );
});

test("project deletion tombstone targets only the container and dominates its head", () => {
  assert.deepEqual(
    buildProjectDeletionTemplate(project, { created_at: 101 }, 100),
    {
      kind: 5,
      content: "Delete project Platform",
      createdAt: 102,
      tags: [["a", PROJECT_ADDRESS]],
    },
  );
});
