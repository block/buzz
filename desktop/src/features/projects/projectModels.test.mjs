import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  addRepositoryToProject,
  buildProjectReadModels,
  eventToRepository,
  selectProjectRepository,
} from "./projectModels.ts";
import { projectMatchesRouteId } from "./projectRoutes.ts";

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
    content: "ignored by NIP-MP readers",
    tags: [
      ["d", "sprout"],
      ["name", "Sprout"],
      ["description", "A multi-repository project"],
      ["buzz-channel", "11111111-1111-4111-8111-111111111111"],
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

test("buildProjectReadModels resolves repositories with a deterministic selection fallback", () => {
  const frontendAddress = `30617:${PROJECT_OWNER}:frontend`;
  const backendAddress = `30617:${PROJECT_OWNER}:backend`;
  const projects = buildProjectReadModels({
    projectEvents: [
      projectEvent([
        ["a", frontendAddress],
        ["a", backendAddress, "wss://relay.example"],
      ]),
    ],
    repositoryEvents: [
      repositoryEvent(PROJECT_OWNER, "frontend"),
      repositoryEvent(PROJECT_OWNER, "backend"),
    ],
    relayOrigin: RELAY_ORIGIN,
  });

  assert.equal(projects.length, 1);
  assert.equal(projects[0].id, `30621:${PROJECT_OWNER}:sprout`);
  assert.equal(projects[0].projectAddress, `30621:${PROJECT_OWNER}:sprout`);
  assert.equal(projects[0].primaryRepositoryAddress, backendAddress);
  assert.deepEqual(
    projects[0].repositories.map((repository) => repository.repoAddress),
    [backendAddress, frontendAddress],
  );
  assert.equal(
    projects[0].repositoryRelayHints[backendAddress],
    "wss://relay.example",
  );
});

test("buildProjectReadModels keeps unclaimed repositories as implicit projects", () => {
  const frontendAddress = `30617:${PROJECT_OWNER}:frontend`;
  const projects = buildProjectReadModels({
    projectEvents: [projectEvent([["a", frontendAddress]])],
    repositoryEvents: [
      repositoryEvent(PROJECT_OWNER, "frontend"),
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

test("buildProjectReadModels does not let an unauthorized project hide a repository", () => {
  const frontendAddress = `30617:${FRONTEND_OWNER}:frontend`;
  const projects = buildProjectReadModels({
    projectEvents: [projectEvent([["a", frontendAddress]])],
    repositoryEvents: [repositoryEvent(FRONTEND_OWNER, "frontend")],
    relayOrigin: RELAY_ORIGIN,
  });

  assert.equal(projects.length, 2);
  assert.equal(projects.filter((project) => project.legacy).length, 1);
  assert.equal(projects.filter((project) => !project.legacy).length, 1);
});

test("project and repository routes stay distinct when coordinates share a d tag", () => {
  const projects = buildProjectReadModels({
    projectEvents: [projectEvent([])],
    repositoryEvents: [repositoryEvent(PROJECT_OWNER, "sprout")],
    relayOrigin: RELAY_ORIGIN,
  });
  const explicitProject = projects.find((project) => !project.legacy);
  const implicitProject = projects.find((project) => project.legacy);

  assert.notEqual(explicitProject.id, implicitProject.id);
  assert.equal(
    projectMatchesRouteId(explicitProject, explicitProject.projectAddress),
    true,
  );
  assert.equal(
    projectMatchesRouteId(explicitProject, implicitProject.projectAddress),
    false,
  );
});

test("addRepositoryToProject promotes a legacy repository route to a project coordinate", () => {
  const [legacyProject] = buildProjectReadModels({
    projectEvents: [],
    repositoryEvents: [repositoryEvent(PROJECT_OWNER, "sprout")],
    relayOrigin: RELAY_ORIGIN,
  });
  const attachedRepository = eventToRepository(
    repositoryEvent(PROJECT_OWNER, "mobile"),
    RELAY_ORIGIN,
  );
  const updated = addRepositoryToProject(
    legacyProject,
    attachedRepository,
    300,
  );

  assert.equal(updated.id, `30621:${PROJECT_OWNER}:sprout`);
  assert.equal(updated.legacy, false);
  assert.equal(updated.repositories.length, 2);
});

test("selectProjectRepository honors a request and falls back to primary", () => {
  const frontendAddress = `30617:${PROJECT_OWNER}:frontend`;
  const projects = buildProjectReadModels({
    projectEvents: [
      projectEvent([
        ["a", frontendAddress],
        ["a", `30617:${PROJECT_OWNER}:backend`],
      ]),
    ],
    repositoryEvents: [
      repositoryEvent(PROJECT_OWNER, "frontend"),
      repositoryEvent(PROJECT_OWNER, "backend"),
    ],
    relayOrigin: RELAY_ORIGIN,
  });

  assert.equal(
    selectProjectRepository(projects[0], `${PROJECT_OWNER}:backend`)?.dtag,
    "backend",
  );
  assert.equal(
    selectProjectRepository(projects[0], "missing:repository")?.dtag,
    "backend",
  );
  assert.equal(selectProjectRepository(projects[0], null)?.dtag, "backend");
});

function coordinateParts(coordinate) {
  const first = coordinate.indexOf(":");
  const second = coordinate.indexOf(":", first + 1);
  return {
    kind: Number(coordinate.slice(0, first)),
    owner: coordinate.slice(first + 1, second),
    dtag: coordinate.slice(second + 1),
  };
}

function sortedJson(values) {
  return values
    .map((value) => JSON.stringify(value))
    .sort()
    .map((value) => JSON.parse(value));
}

test("buildProjectReadModels conforms to the shared NIP-MP fold fixtures", () => {
  const fixture = JSON.parse(
    readFileSync(
      new URL(
        "../../../../docs/nips/NIP-MP.fold-fixtures.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );

  for (const [caseIndex, fixtureCase] of fixture.cases.entries()) {
    let eventIndex = caseIndex * 100;
    const hiddenAddresses = new Set();
    const repositoryEvents = fixtureCase.repositories.flatMap((repository) => {
      if (repository.viewer_hidden) hiddenAddresses.add(repository.coordinate);
      if (repository.state !== "live") return [];
      const { dtag, owner } = coordinateParts(repository.coordinate);
      return [
        {
          ...repositoryEvent(
            owner,
            dtag,
            repository.created_at ?? 1_000 - caseIndex,
          ),
          id: (++eventIndex).toString(16).padStart(64, "0"),
          tags: [
            ["d", dtag],
            ["name", dtag],
            ...(repository.maintainers?.length
              ? [["maintainers", ...repository.maintainers]]
              : []),
          ],
        },
      ];
    });
    const projectEvents = fixtureCase.projects.flatMap((project) => {
      if (project.viewer_hidden) hiddenAddresses.add(project.coordinate);
      if (project.state !== "live") return [];
      const { dtag, owner } = coordinateParts(project.coordinate);
      return [
        {
          id: (++eventIndex).toString(16).padStart(64, "0"),
          kind: 30621,
          pubkey: owner,
          created_at: project.created_at ?? 900 - caseIndex,
          content: "",
          tags: [
            ["d", dtag],
            ["name", dtag],
            ...(project.visibility === "unlisted"
              ? [["buzz-visibility", "unlisted"]]
              : []),
            ...project.members.map((member) => ["a", member]),
          ],
        },
      ];
    });

    const projects = buildProjectReadModels({
      projectEvents,
      repositoryEvents,
      relayOrigin: RELAY_ORIGIN,
      hiddenAddresses,
    });
    const actualContainers = projects
      .filter((project) => !project.legacy)
      .map((project) => ({
        project: project.projectAddress,
        members: project.repositoryAddresses.flatMap((coordinate) => {
          if (
            project.repositories.some(
              (repository) => repository.repoAddress === coordinate,
            )
          ) {
            return [{ coordinate, render: "resolved" }];
          }
          return project.unavailableRepositoryAddresses?.includes(coordinate)
            ? [{ coordinate, render: "unavailable" }]
            : [];
        }),
      }));
    const expectedContainers = fixtureCase.expect.containers.map(
      (container) => ({
        project: container.project,
        members: sortedJson(container.members),
      }),
    );

    assert.deepEqual(
      sortedJson(
        actualContainers.map((container) => ({
          ...container,
          members: sortedJson(container.members),
        })),
      ),
      sortedJson(expectedContainers),
      fixtureCase.name,
    );
    assert.deepEqual(
      projects
        .filter((project) => project.legacy)
        .map((project) => project.projectAddress)
        .sort(),
      [...fixtureCase.expect.implicit_cards].sort(),
      fixtureCase.name,
    );
  }
});
