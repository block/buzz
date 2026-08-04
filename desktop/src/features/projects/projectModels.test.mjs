import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  addRepositoryToProject,
  buildProjectReadModels,
  eventToExplicitProject,
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

// ── NIP-MP ingest-fixture conformance (TypeScript parser) ──────────────────
//
// The shared NIP-MP.fixtures.json is the ingest oracle used by the relay
// validator and the Rust CLI builder. The TypeScript read model must agree
// on every accept/reject case so that relay and desktop never diverge about
// which signed project heads are valid.

test("buildProjectReadModels conforms to the shared NIP-MP ingest fixtures", () => {
  const fixtures = JSON.parse(
    readFileSync(
      new URL("../../../../docs/nips/NIP-MP.fixtures.json", import.meta.url),
      "utf8",
    ),
  );

  const SIGNER = "a".repeat(64);

  for (const fixtureCase of fixtures.cases) {
    const event = {
      id: "e".repeat(64),
      kind: fixtureCase.template.kind,
      pubkey: SIGNER,
      created_at: 1_000,
      content: fixtureCase.template.content,
      tags: fixtureCase.template.tags,
    };

    // Use eventToExplicitProject directly — buildProjectReadModels only returns
    // listed projects, so an unlisted-but-valid envelope (e.g. valid_unlisted)
    // would be filtered out before we could observe it. The ingest oracle only
    // cares whether the envelope parses successfully, not whether it reaches
    // the rendered collection; eventToExplicitProject is the correct gate.
    const parsed = eventToExplicitProject(event, new Map(), new Map());
    const accepted = parsed !== null;

    if (fixtureCase.expect === "accept") {
      assert.equal(
        accepted,
        true,
        `${fixtureCase.name}: expected accept, got reject`,
      );
    } else {
      assert.equal(
        accepted,
        false,
        `${fixtureCase.name}: expected reject, got accept`,
      );
    }
  }
});

// ── NIP-09 tombstone deletion ───────────────────────────────────────────────

test("buildProjectReadModels applies kind:5 tombstones to project events", () => {
  const owner = "a".repeat(64);
  const projectAddress = `30621:${owner}:platform`;
  const projectEvent = {
    id: "p".repeat(64),
    kind: 30621,
    pubkey: owner,
    created_at: 100,
    content: "",
    tags: [
      ["d", "platform"],
      ["name", "Platform"],
    ],
  };
  const deletionEvent = {
    id: "d".repeat(64),
    kind: 5,
    pubkey: owner,
    created_at: 200,
    content: "",
    tags: [["a", projectAddress]],
  };

  const withDeletion = buildProjectReadModels({
    projectEvents: [projectEvent],
    repositoryEvents: [],
    deletionEvents: [deletionEvent],
  });
  assert.equal(
    withDeletion.filter((p) => !p.legacy).length,
    0,
    "deleted project should not appear",
  );

  const withoutDeletion = buildProjectReadModels({
    projectEvents: [projectEvent],
    repositoryEvents: [],
  });
  assert.equal(
    withoutDeletion.filter((p) => !p.legacy).length,
    1,
    "project without deletion should appear",
  );
});

test("buildProjectReadModels applies kind:5 tombstones to repository events", () => {
  const owner = "b".repeat(64);
  const repoAddress = `30617:${owner}:relay`;
  const repoEvent = {
    id: "r".repeat(64),
    kind: 30617,
    pubkey: owner,
    created_at: 100,
    content: "",
    tags: [
      ["d", "relay"],
      ["name", "relay"],
    ],
  };
  const deletionEvent = {
    id: "d".repeat(64),
    kind: 5,
    pubkey: owner,
    created_at: 200,
    content: "",
    tags: [["a", repoAddress]],
  };

  const withDeletion = buildProjectReadModels({
    projectEvents: [],
    repositoryEvents: [repoEvent],
    deletionEvents: [deletionEvent],
  });
  assert.equal(withDeletion.length, 0, "deleted repository should not appear");
});

test("buildProjectReadModels ignores a tombstone that predates the live head", () => {
  const owner = "a".repeat(64);
  const projectAddress = `30621:${owner}:platform`;
  const projectEvent = {
    id: "p".repeat(64),
    kind: 30621,
    pubkey: owner,
    created_at: 200,
    content: "",
    tags: [["d", "platform"]],
  };
  // Deletion is at t=100, but the live head is at t=200 — should not apply.
  const staleDeletion = {
    id: "d".repeat(64),
    kind: 5,
    pubkey: owner,
    created_at: 100,
    content: "",
    tags: [["a", projectAddress]],
  };

  const projects = buildProjectReadModels({
    projectEvents: [projectEvent],
    repositoryEvents: [],
    deletionEvents: [staleDeletion],
  });
  assert.equal(
    projects.filter((p) => !p.legacy).length,
    1,
    "head newer than tombstone must survive",
  );
});

test("buildProjectReadModels rejects a tombstone signed by a different pubkey", () => {
  const owner = "a".repeat(64);
  const impostor = "b".repeat(64);
  const projectAddress = `30621:${owner}:platform`;
  const projectEvent = {
    id: "p".repeat(64),
    kind: 30621,
    pubkey: owner,
    created_at: 100,
    content: "",
    tags: [["d", "platform"]],
  };
  const foreignDeletion = {
    id: "d".repeat(64),
    kind: 5,
    pubkey: impostor,
    created_at: 200,
    content: "",
    tags: [["a", projectAddress]],
  };

  const projects = buildProjectReadModels({
    projectEvents: [projectEvent],
    repositoryEvents: [],
    deletionEvents: [foreignDeletion],
  });
  assert.equal(
    projects.filter((p) => !p.legacy).length,
    1,
    "a stranger's tombstone must not delete someone else's project",
  );
});

// ── Route contract ──────────────────────────────────────────────────────────

test("projectMatchesRouteId resolves a 30617 coordinate to the containing project", () => {
  const projectOwner = "a".repeat(64);
  const repoOwner = "c".repeat(64);
  const repoAddress = `30617:${repoOwner}:relay`;
  const projects = buildProjectReadModels({
    projectEvents: [
      {
        id: "project-event",
        kind: 30621,
        pubkey: projectOwner,
        created_at: 200,
        content: "",
        tags: [
          ["d", "platform"],
          ["a", repoAddress],
        ],
      },
    ],
    repositoryEvents: [
      {
        id: "repo-event",
        kind: 30617,
        pubkey: repoOwner,
        created_at: 100,
        content: "",
        tags: [
          ["d", "relay"],
          ["name", "relay"],
          ["maintainers", projectOwner],
        ],
      },
    ],
  });
  const explicitProject = projects.find((p) => !p.legacy);
  assert.ok(explicitProject, "explicit project must be present");

  // Entity link navigates with the 30617 coordinate (repo dtag ≠ project dtag).
  assert.equal(
    projectMatchesRouteId(explicitProject, repoAddress),
    true,
    "30617 route must resolve to the containing project",
  );
  // Legacy project (implicit card for an unclaimed repo) must NOT match the
  // explicit project's 30621 coordinate.
  assert.equal(
    projectMatchesRouteId(explicitProject, `30617:${projectOwner}:platform`),
    false,
    "unrelated 30617 address must not match",
  );
});
