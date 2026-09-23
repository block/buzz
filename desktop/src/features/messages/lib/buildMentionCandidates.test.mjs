import assert from "node:assert/strict";
import test from "node:test";

import { buildMentionCandidates } from "./buildMentionCandidates.ts";

const MEMBER_PUBKEY = "a".repeat(64);
const AGENT_PUBKEY = "b".repeat(64);
const ARCHIVED_PUBKEY = "c".repeat(64);
const SEARCHED_PUBKEY = "d".repeat(64);

function input(overrides = {}) {
  return {
    activeAgentPubkeys: new Set(),
    activePersonaById: new Map(),
    activePersonas: [],
    canSearchGlobalUsers: false,
    currentPubkey: null,
    isArchived: () => false,
    managedAgentDirectoryReady: true,
    managedAgentNamesByPubkey: new Map(),
    managedAgentPersonaIds: new Set(),
    managedAgentPersonaIdsByPubkey: new Map(),
    managedAgents: [],
    memberPubkeys: new Set(),
    members: [],
    mentionChannelId: null,
    mentionableAgentPubkeys: new Set(),
    personaNameByPubkey: new Map(),
    profiles: undefined,
    relayAgentDirectoryReady: true,
    relayAgentNamesByPubkey: new Map(),
    relayAgents: [],
    userSearchResults: [],
    ...overrides,
  };
}

test("a roster entry and its relay agent record coalesce into one candidate", () => {
  const candidates = buildMentionCandidates(
    input({
      members: [
        { pubkey: AGENT_PUBKEY, displayName: null, isAgent: true, role: "bot" },
      ],
      mentionableAgentPubkeys: new Set([AGENT_PUBKEY]),
      relayAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: "Scout",
          ownerPubkey: null,
          status: "online",
        },
      ],
    }),
  );

  assert.equal(candidates.length, 1);
  assert.equal(candidates[0].pubkey, AGENT_PUBKEY);
  // The roster contributes membership, the directory contributes the name.
  assert.equal(candidates[0].isMember, true);
  assert.equal(candidates[0].displayName, "Scout");
  assert.equal(candidates[0].isActiveAgent, true);
});

test("archived identities never become candidates", () => {
  const candidates = buildMentionCandidates(
    input({
      isArchived: (pubkey) => pubkey === ARCHIVED_PUBKEY,
      members: [
        { pubkey: MEMBER_PUBKEY, displayName: "Ada", isAgent: false },
        { pubkey: ARCHIVED_PUBKEY, displayName: "Gone", isAgent: false },
      ],
    }),
  );

  assert.deepEqual(
    candidates.map((candidate) => candidate.pubkey),
    [MEMBER_PUBKEY],
  );
});

test("archived roster members and directory agents stay out even when still cached", () => {
  const candidates = buildMentionCandidates(
    input({
      isArchived: (pubkey) => pubkey === ARCHIVED_PUBKEY,
      memberPubkeys: new Set([ARCHIVED_PUBKEY, MEMBER_PUBKEY]),
      members: [
        {
          pubkey: ARCHIVED_PUBKEY,
          displayName: "Archived Bot",
          isAgent: true,
          role: "bot",
        },
        { pubkey: MEMBER_PUBKEY, displayName: "Ada", isAgent: false },
      ],
      mentionableAgentPubkeys: new Set([ARCHIVED_PUBKEY, AGENT_PUBKEY]),
      managedAgents: [
        {
          pubkey: ARCHIVED_PUBKEY,
          name: "Archived Managed",
          status: "stopped",
        },
      ],
      relayAgents: [
        {
          pubkey: ARCHIVED_PUBKEY,
          name: "Archived Relay",
          ownerPubkey: null,
          status: "online",
        },
        {
          pubkey: AGENT_PUBKEY,
          name: "Scout",
          ownerPubkey: null,
          status: "online",
        },
      ],
      managedAgentNamesByPubkey: new Map([[ARCHIVED_PUBKEY, "Archived Managed"]]),
      relayAgentNamesByPubkey: new Map([
        [ARCHIVED_PUBKEY, "Archived Relay"],
        [AGENT_PUBKEY, "Scout"],
      ]),
    }),
  );

  assert.deepEqual(
    candidates.map((candidate) => candidate.pubkey).sort(),
    [AGENT_PUBKEY, MEMBER_PUBKEY].sort(),
  );
  assert.equal(
    candidates.some((candidate) => candidate.pubkey === ARCHIVED_PUBKEY),
    false,
  );
});

test("an agent outside the mentionable set is hidden once its directory is ready", () => {
  const relayAgents = [
    { pubkey: AGENT_PUBKEY, name: "Scout", ownerPubkey: null, status: "away" },
  ];

  assert.deepEqual(buildMentionCandidates(input({ relayAgents })), []);
  assert.equal(
    buildMentionCandidates(
      input({ mentionableAgentPubkeys: new Set([AGENT_PUBKEY]), relayAgents }),
    ).length,
    1,
  );
});

test("active personas join unless a managed agent already carries them", () => {
  const activePersonas = [
    { id: "planner", displayName: "Planner", avatarUrl: null, isActive: true },
  ];

  const standalone = buildMentionCandidates(input({ activePersonas }));
  assert.equal(standalone.length, 1);
  assert.equal(standalone[0].kind, "persona");
  assert.equal(standalone[0].personaId, "planner");

  assert.deepEqual(
    buildMentionCandidates(
      input({ activePersonas, managedAgentPersonaIds: new Set(["planner"]) }),
    ),
    [],
  );
});

test("global search results join only while global search is enabled", () => {
  const userSearchResults = [
    {
      pubkey: SEARCHED_PUBKEY,
      displayName: "Dana",
      isAgent: false,
      nip05Handle: null,
      ownerPubkey: null,
    },
  ];

  assert.deepEqual(buildMentionCandidates(input({ userSearchResults })), []);

  const searched = buildMentionCandidates(
    input({ canSearchGlobalUsers: true, userSearchResults }),
  );
  assert.equal(searched.length, 1);
  assert.equal(searched[0].displayName, "Dana");
  assert.equal(searched[0].isGlobalSearchResult, true);
});

test("policy-only discovery stays selectable without claiming active presence", () => {
  const [candidate] = buildMentionCandidates(
    input({
      mentionableAgentPubkeys: new Set([AGENT_PUBKEY]),
      relayAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: "Scout",
          ownerPubkey: MEMBER_PUBKEY,
          status: "unknown",
        },
      ],
    }),
  );
  assert.equal(candidate.pubkey, AGENT_PUBKEY);
  assert.equal(candidate.isActiveAgent, false);
  assert.equal(candidate.ownerPubkey, MEMBER_PUBKEY);
});

for (const locallyManaged of [true, false]) {
  test(`roster candidate preserves exact local management: ${locallyManaged}`, () => {
    const [candidate] = buildMentionCandidates(
      input({
        members: [{ pubkey: AGENT_PUBKEY, displayName: "Scout", role: "bot" }],
        managedAgentNamesByPubkey: new Map(
          locallyManaged ? [[AGENT_PUBKEY, "Scout"]] : [],
        ),
        managedAgents: locallyManaged
          ? [{ pubkey: AGENT_PUBKEY, name: "Scout", status: "deployed" }]
          : [],
        mentionableAgentPubkeys: new Set([AGENT_PUBKEY]),
      }),
    );
    assert.equal(candidate.isMember, true);
    assert.equal(Boolean(candidate.isManagedAgent), locallyManaged);
  });
}

test("in-channel reserved community bot stays isMember after catalog route", () => {
  const channelMo = "b".repeat(64);
  const catalogMo = "a".repeat(64);
  const candidates = buildMentionCandidates(
    input({
      memberPubkeys: new Set([channelMo]),
      members: [
        {
          pubkey: channelMo,
          displayName: "Mo",
          role: "bot",
          isAgent: true,
        },
      ],
      mentionableAgentPubkeys: new Set([channelMo, catalogMo]),
      relayAgents: [
        {
          pubkey: catalogMo,
          name: "Mo",
          ownerPubkey: null,
          agentType: "openclaw",
          channels: [],
          channelIds: [],
          capabilities: [],
          status: "online",
          respondTo: null,
          respondToAllowlist: [],
        },
      ],
      // Channel-preferred route (see reservedCommunityBotRoutes).
      reservedCommunityBotRoutes: new Map([["mo", channelMo]]),
    }),
  );
  const mo = candidates.filter((c) => c.displayName === "Mo");
  assert.equal(mo.length, 1);
  assert.equal(mo[0]?.pubkey, channelMo);
  assert.equal(mo[0]?.isMember, true);
});

test("memberPubkeys re-asserts isMember when routed identity matches roster", () => {
  const mo = "b".repeat(64);
  const candidates = buildMentionCandidates(
    input({
      memberPubkeys: new Set([mo]),
      members: [],
      mentionableAgentPubkeys: new Set([mo]),
      relayAgents: [
        {
          pubkey: mo,
          name: "Mo",
          ownerPubkey: null,
          agentType: "openclaw",
          channels: [],
          channelIds: [],
          capabilities: [],
          status: "online",
          respondTo: null,
          respondToAllowlist: [],
        },
      ],
      reservedCommunityBotRoutes: new Map([["mo", mo]]),
    }),
  );
  const hit = candidates.find((c) => c.displayName === "Mo");
  assert.equal(hit?.isMember, true);
});
