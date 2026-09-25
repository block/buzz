import assert from "node:assert/strict";
import test from "node:test";

import { truncateNpub, truncatePubkey } from "@/shared/lib/pubkey.ts";

import {
  buildChannelMemberMentionCandidate,
  buildTeamMentionCandidates,
  formatTeamMention,
  resolveMentionMemberDisplayName,
  shouldAcceptMentionIdentityCandidate,
} from "./mentionCandidates.ts";

const MO_PUBKEY = `${"d5c38517".padEnd(60, "0")}83ae`;
const ADA_PUBKEY = "aa".repeat(32);

const moBot = {
  id: "mo",
  name: "mo",
  pubkey: MO_PUBKEY,
  source: "openclaw",
};

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

test("teams with duplicate identity display names remain selectable with their exact keys", () => {
  const personas = [
    persona("builder-one", "First"),
    persona("builder-two", "Second"),
  ];
  const candidates = [
    identity("builder-one", "Builder", { pubkey: "1".repeat(64) }),
    identity("builder-two", "Builder", { pubkey: "2".repeat(64) }),
  ];

  const suggestions = buildTeamMentionCandidates(
    [team("duplicate-identities", ["builder-one", "builder-two"])],
    personas,
    candidates,
  );
  assert.deepEqual(
    suggestions[0].teamMembers.map((member) => member.pubkey),
    ["1".repeat(64), "2".repeat(64)],
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

test("resolveMentionMemberDisplayName uses the community-bots catalog overlay", () => {
  assert.equal(
    resolveMentionMemberDisplayName({
      pubkey: MO_PUBKEY,
      communityBots: [moBot],
    }),
    "mo",
  );
  assert.equal(
    resolveMentionMemberDisplayName({
      pubkey: MO_PUBKEY,
      memberDisplayName: truncatePubkey(MO_PUBKEY),
      communityBots: [moBot],
    }),
    "mo",
  );
});

test("resolveMentionMemberDisplayName keeps member, kind 0, and managed-agent names", () => {
  assert.equal(
    resolveMentionMemberDisplayName({
      pubkey: ADA_PUBKEY,
      memberDisplayName: "Ada",
      communityBots: [moBot],
    }),
    "Ada",
  );
  assert.equal(
    resolveMentionMemberDisplayName({
      pubkey: ADA_PUBKEY,
      profileDisplayName: "Ada Lovelace",
      profileNip05: "ada@example.com",
    }),
    "Ada Lovelace",
  );
  assert.equal(
    resolveMentionMemberDisplayName({
      pubkey: ADA_PUBKEY,
      agentName: "Honey",
    }),
    "Honey",
  );
  assert.equal(
    resolveMentionMemberDisplayName({
      pubkey: ADA_PUBKEY,
      profileNip05: "ada@example.com",
    }),
    "ada@example.com",
  );
});

test("resolveMentionMemberDisplayName falls back to the truncated npub", () => {
  assert.equal(
    resolveMentionMemberDisplayName({ pubkey: ADA_PUBKEY }),
    truncateNpub(ADA_PUBKEY),
  );
});

test("buildChannelMemberMentionCandidate labels a catalog bot and unnamed members", () => {
  const bot = buildChannelMemberMentionCandidate({
    member: {
      pubkey: MO_PUBKEY,
      displayName: null,
      isAgent: false,
      role: "bot",
    },
    communityBots: [moBot],
  });
  assert.equal(bot.displayName, "mo");
  assert.equal(bot.isMember, true);
  assert.equal(bot.isAgent, true);
  assert.equal(bot.role, "bot");
  assert.equal(bot.pubkey, MO_PUBKEY);

  const unnamed = buildChannelMemberMentionCandidate({
    member: {
      pubkey: ADA_PUBKEY,
      displayName: null,
      isAgent: false,
      role: "member",
    },
  });
  assert.equal(unnamed.displayName, truncateNpub(ADA_PUBKEY));
  assert.equal(unnamed.isMember, true);
  assert.equal(unnamed.isAgent, false);

  const fizz = buildChannelMemberMentionCandidate({
    member: {
      pubkey: ADA_PUBKEY,
      displayName: null,
      isAgent: true,
      role: "bot",
    },
    agentName: "Fizz",
    isDirectoryAgent: true,
  });
  assert.equal(fizz.displayName, "Fizz");
  assert.equal(fizz.isAgent, true);
});

test("archived catalog or bot-role room members stay out of mention candidates", () => {
  assert.equal(
    shouldAcceptMentionIdentityCandidate({
      isArchived: true,
      isMember: true,
      hideAgent: false,
    }),
    false,
  );
  assert.equal(
    shouldAcceptMentionIdentityCandidate({
      isArchived: true,
      isMember: false,
      hideAgent: false,
    }),
    false,
  );
});

test("unarchived catalog or bot-role room members stay mentionable", () => {
  assert.equal(
    shouldAcceptMentionIdentityCandidate({
      isArchived: false,
      isMember: true,
      hideAgent: true,
    }),
    true,
  );
  assert.equal(
    shouldAcceptMentionIdentityCandidate({
      isArchived: false,
      isMember: false,
      hideAgent: false,
    }),
    true,
  );
});

test("non-member agents still honor the hide-agent directory gate", () => {
  assert.equal(
    shouldAcceptMentionIdentityCandidate({
      isArchived: false,
      isMember: false,
      hideAgent: true,
    }),
    false,
  );
});
