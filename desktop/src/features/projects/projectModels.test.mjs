import assert from "node:assert/strict";
import test from "node:test";

import {
  buildProjectReadModels,
  eventToRepository,
  selectProjectRepository,
} from "./projectModels.ts";

const PROJECT_OWNER = "a".repeat(64);
const FRONTEND_OWNER = "b".repeat(64);
const BACKEND_OWNER = "c".repeat(64);
const RELAY_ORIGIN = "https://relay.example";

function repositoryEvent(owner, id, createdAt = 100) {
  return {
    id: `${id}-${createdAt}`,
    kind: 30617,
    pubkey: owner,
    created_at: createdAt,
    content: "",
    tags: [
      ["d", id],
      ["name", id],
    ],
  };
}

function projectEvent(repositoryTags, overrides = {}) {
  return {
    id: "project-event",
    kind: 30621,
    pubkey: PROJECT_OWNER,
    created_at: 200,
    content: "A multi-repository project",
    tags: [
      ["d", "sprout"],
      ["name", "Sprout"],
      ["h", "11111111-1111-4111-8111-111111111111"],
      ...repositoryTags,
    ],
    ...overrides,
  };
}

test("eventToRepository preserves repository-scoped identity and clone data", () => {
  const repository = eventToRepository(
    repositoryEvent(FRONTEND_OWNER, "frontend"),
    RELAY_ORIGIN,
  );

  assert.equal(repository.id, `${FRONTEND_OWNER}:frontend`);
  assert.equal(repository.repoAddress, `30617:${FRONTEND_OWNER}:frontend`);
  assert.deepEqual(repository.cloneUrls, [
    `${RELAY_ORIGIN}/git/${FRONTEND_OWNER}/frontend`,
  ]);
});

test("buildProjectReadModels resolves ordered repositories and primary", () => {
  const frontendAddress = `30617:${FRONTEND_OWNER}:frontend`;
  const backendAddress = `30617:${BACKEND_OWNER}:backend`;
  const projects = buildProjectReadModels({
    projectEvents: [
      projectEvent([
        ["a", frontendAddress, "", "primary"],
        ["a", backendAddress],
      ]),
    ],
    repositoryEvents: [
      repositoryEvent(FRONTEND_OWNER, "frontend"),
      repositoryEvent(BACKEND_OWNER, "backend"),
    ],
    relayOrigin: RELAY_ORIGIN,
  });

  assert.equal(projects.length, 1);
  assert.equal(projects[0].id, `${PROJECT_OWNER}:sprout`);
  assert.equal(projects[0].projectAddress, `30621:${PROJECT_OWNER}:sprout`);
  assert.equal(projects[0].primaryRepositoryAddress, frontendAddress);
  assert.deepEqual(
    projects[0].repositories.map((repository) => repository.repoAddress),
    [frontendAddress, backendAddress],
  );
});

test("buildProjectReadModels keeps ungrouped repositories as legacy projects", () => {
  const frontendAddress = `30617:${FRONTEND_OWNER}:frontend`;
  const projects = buildProjectReadModels({
    projectEvents: [projectEvent([["a", frontendAddress, "", "primary"]])],
    repositoryEvents: [
      repositoryEvent(FRONTEND_OWNER, "frontend"),
      repositoryEvent(BACKEND_OWNER, "backend"),
    ],
    relayOrigin: RELAY_ORIGIN,
  });

  assert.equal(projects.length, 2);
  assert.equal(projects[0].legacy, false);
  assert.equal(projects[1].legacy, true);
  assert.equal(
    projects[1].primaryRepositoryAddress,
    projects[1].projectAddress,
  );
  assert.equal(projects[1].repositories[0].dtag, "backend");
});

test("buildProjectReadModels ignores malformed primary membership", () => {
  const frontendAddress = `30617:${FRONTEND_OWNER}:frontend`;
  const projects = buildProjectReadModels({
    projectEvents: [
      projectEvent([
        ["a", frontendAddress],
        [`a`, `30617:${BACKEND_OWNER}:backend`],
      ]),
    ],
    repositoryEvents: [repositoryEvent(FRONTEND_OWNER, "frontend")],
    relayOrigin: RELAY_ORIGIN,
  });

  assert.equal(projects.length, 1);
  assert.equal(projects[0].legacy, true);
  assert.equal(projects[0].repositories[0].dtag, "frontend");
});

test("selectProjectRepository honors a request and falls back to primary", () => {
  const frontendAddress = `30617:${FRONTEND_OWNER}:frontend`;
  const projects = buildProjectReadModels({
    projectEvents: [
      projectEvent([
        ["a", frontendAddress, "", "primary"],
        ["a", `30617:${BACKEND_OWNER}:backend`],
      ]),
    ],
    repositoryEvents: [
      repositoryEvent(FRONTEND_OWNER, "frontend"),
      repositoryEvent(BACKEND_OWNER, "backend"),
    ],
    relayOrigin: RELAY_ORIGIN,
  });

  assert.equal(
    selectProjectRepository(projects[0], `${BACKEND_OWNER}:backend`)?.dtag,
    "backend",
  );
  assert.equal(
    selectProjectRepository(projects[0], "missing:repository")?.dtag,
    "frontend",
  );
  assert.equal(selectProjectRepository(projects[0], null)?.dtag, "frontend");
});
