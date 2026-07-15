import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTeamMentionCandidates,
  formatTeamMention,
} from "./mentionCandidates.ts";

function persona(id, displayName, isActive = true) {
  return {
    id,
    displayName,
    avatarUrl: null,
    systemPrompt: `${displayName} prompt`,
    runtime: null,
    model: null,
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };
}

function team(id, personaIds, overrides = {}) {
  return {
    id,
    name: "Launch Team",
    description: null,
    instructions: null,
    personaIds,
    isBuiltin: false,
    sourceDir: null,
    isSymlink: false,
    symlinkTarget: null,
    version: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function identity(personaId, displayName, overrides = {}) {
  return {
    kind: "identity",
    personaId,
    displayName,
    isAgent: true,
    isMember: false,
    ...overrides,
  };
}

test("team mentions preserve team order and prefer concrete managed agents", () => {
  const personas = [
    persona("planner", "Planner"),
    persona("builder", "Builder"),
    persona("reviewer", "Reviewer"),
  ];
  const candidates = [
    identity("builder", "Build Bot", {
      isManagedAgent: true,
      pubkey: "2".repeat(64),
    }),
    identity("planner", "Plan Bot", {
      isManagedAgent: true,
      pubkey: "1".repeat(64),
    }),
    identity("planner", "Planner in channel", {
      isMember: true,
      pubkey: "3".repeat(64),
    }),
  ];

  const [suggestion] = buildTeamMentionCandidates(
    [team("launch", ["planner", "builder", "reviewer"])],
    personas,
    candidates,
  );

  assert.equal(suggestion.kind, "team");
  assert.deepEqual(suggestion.teamMembers, [
    {
      displayName: "Planner in channel",
      kind: "identity",
      personaId: "planner",
      pubkey: "3".repeat(64),
    },
    {
      displayName: "Build Bot",
      kind: "identity",
      personaId: "builder",
      pubkey: "2".repeat(64),
    },
    {
      displayName: "Reviewer",
      kind: "persona",
      personaId: "reviewer",
    },
  ]);
  assert.equal(
    formatTeamMention(suggestion.displayName, suggestion.teamMembers),
    "Launch Team(@Planner in channel @Build Bot @Reviewer) ",
  );
});

test("only complete, owned teams with mentionable members are suggested", () => {
  const active = persona("active", "Active");
  const inactive = persona("inactive", "Inactive", false);
  const teams = [
    team("owned", ["active"]),
    team("builtin", ["active"], { isBuiltin: true }),
    team("missing", ["missing"]),
    team("inactive", ["inactive"]),
  ];

  assert.deepEqual(
    buildTeamMentionCandidates(teams, [active, inactive], []).map(
      (candidate) => candidate.teamId,
    ),
    ["owned"],
  );
});

test("teams with duplicate identity display names are not suggested", () => {
  const personas = [
    persona("builder-one", "First"),
    persona("builder-two", "Second"),
  ];
  const candidates = [
    identity("builder-one", "Builder", { pubkey: "1".repeat(64) }),
    identity("builder-two", "Builder", { pubkey: "2".repeat(64) }),
  ];

  assert.deepEqual(
    buildTeamMentionCandidates(
      [team("duplicate-identities", ["builder-one", "builder-two"])],
      personas,
      candidates,
    ),
    [],
  );
});

test("teams with identity and persona display-name collisions are not suggested", () => {
  const personas = [
    persona("managed-builder", "Managed Builder"),
    persona("persona-builder", "builder"),
  ];
  const candidates = [
    identity("managed-builder", "Builder", { pubkey: "1".repeat(64) }),
  ];

  assert.deepEqual(
    buildTeamMentionCandidates(
      [
        team("identity-persona-collision", [
          "managed-builder",
          "persona-builder",
        ]),
      ],
      personas,
      candidates,
    ),
    [],
  );
});
