import assert from "node:assert/strict";
import test from "node:test";

import { getMentionableAgentPubkeys } from "@/features/agents/lib/agentAutocompleteEligibility.ts";
import { rankMentionCandidates } from "./mentionRanking.ts";
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
      managedAgentNamesByPubkey: new Map([
        [ARCHIVED_PUBKEY, "Archived Managed"],
      ]),
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

test("foreign identically named agents are hidden unless owned, allowlisted, or community-admitted", () => {
  const CURRENT = "e".repeat(64);
  const OWNER_A = "f".repeat(64);
  const OWNER_B = "0".repeat(64);
  const OWN_SCOUT = "1".repeat(64);
  const FOREIGN_SCOUT = "2".repeat(64);
  const ALLOWLISTED_SCOUT = "3".repeat(64);
  const FOREIGN_ORB = "4".repeat(64);
  const OWN_ORB = "5".repeat(64);
  const COMMUNITY_CAPTAIN = "6".repeat(64);
  const ARCHIVED_SCOUT = "7".repeat(64);

  const channelId = "general";
  const relayAgents = [
    {
      pubkey: OWN_SCOUT,
      name: "Scout",
      ownerPubkey: CURRENT,
      status: "online",
      respondTo: "anyone",
      respondToAllowlist: [],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: FOREIGN_SCOUT,
      name: "Scout",
      ownerPubkey: OWNER_A,
      status: "online",
      respondTo: "anyone",
      respondToAllowlist: [],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: ALLOWLISTED_SCOUT,
      name: "Scout",
      ownerPubkey: OWNER_B,
      status: "online",
      respondTo: "allowlist",
      respondToAllowlist: [CURRENT],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: FOREIGN_ORB,
      name: "Orb",
      ownerPubkey: OWNER_A,
      status: "online",
      respondTo: "anyone",
      respondToAllowlist: [],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: OWN_ORB,
      name: "Orb",
      ownerPubkey: CURRENT,
      status: "online",
      respondTo: "owner-only",
      respondToAllowlist: [],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: COMMUNITY_CAPTAIN,
      name: "Captain",
      ownerPubkey: OWNER_A,
      status: "online",
      respondTo: "anyone",
      respondToAllowlist: [],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: ARCHIVED_SCOUT,
      name: "Scout",
      ownerPubkey: CURRENT,
      status: "online",
      respondTo: "anyone",
      respondToAllowlist: [],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
  ];

  // Mirrors useMentions: includePublicAnyone false + community extras.
  const mentionableAgentPubkeys = getMentionableAgentPubkeys({
    currentPubkey: CURRENT,
    phase: "publish",
    eligibilityScope: { type: "channel", channelId },
    managedAgentPubkeys: [],
    relayAgents,
    sharedChannelIds: new Set([channelId]),
    includePublicAnyone: false,
    extraMentionablePubkeys: [COMMUNITY_CAPTAIN],
  });

  const candidates = buildMentionCandidates(
    input({
      currentPubkey: CURRENT,
      mentionChannelId: channelId,
      memberPubkeys: new Set([
        OWN_SCOUT,
        FOREIGN_SCOUT,
        ALLOWLISTED_SCOUT,
        FOREIGN_ORB,
        OWN_ORB,
        COMMUNITY_CAPTAIN,
        ARCHIVED_SCOUT,
        MEMBER_PUBKEY,
      ]),
      members: [
        { pubkey: MEMBER_PUBKEY, displayName: "Ada", isAgent: false },
        {
          pubkey: COMMUNITY_CAPTAIN,
          displayName: "Captain",
          isAgent: true,
          role: "bot",
        },
      ],
      mentionableAgentPubkeys,
      relayAgents,
      isArchived: (pubkey) => pubkey === ARCHIVED_SCOUT,
      reservedCommunityBotRoutes: new Map([["captain", COMMUNITY_CAPTAIN]]),
    }),
  );

  const byName = (name) =>
    candidates.filter((candidate) => candidate.displayName === name);

  // Same display name, multiple owners: only owned + allowlisted remain.
  assert.deepEqual(
    byName("Scout")
      .map((candidate) => candidate.pubkey)
      .sort(),
    [ALLOWLISTED_SCOUT, OWN_SCOUT].sort(),
  );
  assert.equal(
    byName("Scout").some((candidate) => candidate.pubkey === FOREIGN_SCOUT),
    false,
  );

  // Unrelated second name proves the rule is global, not name-specific.
  assert.deepEqual(
    byName("Orb").map((candidate) => candidate.pubkey),
    [OWN_ORB],
  );
  assert.equal(
    byName("Orb").some((candidate) => candidate.pubkey === FOREIGN_ORB),
    false,
  );

  // Community bot still appears even though foreign+anyone.
  assert.equal(byName("Captain").length, 1);
  assert.equal(byName("Captain")[0]?.pubkey, COMMUNITY_CAPTAIN);

  // Human members still appear.
  assert.equal(
    candidates.some((candidate) => candidate.pubkey === MEMBER_PUBKEY),
    true,
  );

  // Archived still excluded.
  assert.equal(
    candidates.some((candidate) => candidate.pubkey === ARCHIVED_SCOUT),
    false,
  );

  // Alphabetical ranking among surviving identities still applies.
  const ranked = rankMentionCandidates(candidates, "").map(
    (entry) => entry.candidate.displayName,
  );
  const sorted = [...ranked].sort((left, right) =>
    left.localeCompare(right, undefined, { sensitivity: "base" }),
  );
  assert.deepEqual(ranked, sorted);
});

test("owned same-named clones outside the channel stay hidden (Fizz flood)", () => {
  const CURRENT = "e".repeat(64);
  const OTHER_OWNER = "f".repeat(64);
  const LOCAL_FIZZ = "1".repeat(64);
  const OTHER_ROOM_FIZZ = "2".repeat(64);
  const UNJOINED_FIZZ = "3".repeat(64);
  const FOREIGN_FIZZ = "4".repeat(64);
  const ARCHIVED_FIZZ = "5".repeat(64);
  const channelId = "general";

  const relayAgents = [
    {
      pubkey: LOCAL_FIZZ,
      name: "Fizz",
      ownerPubkey: CURRENT,
      status: "online",
      respondTo: "anyone",
      respondToAllowlist: [],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: OTHER_ROOM_FIZZ,
      name: "Fizz",
      ownerPubkey: CURRENT,
      status: "online",
      respondTo: "owner-only",
      respondToAllowlist: [],
      channelIds: ["elsewhere"],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: UNJOINED_FIZZ,
      name: "Fizz",
      ownerPubkey: CURRENT,
      status: "unknown",
      respondTo: "anyone",
      respondToAllowlist: [],
      channelIds: [],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: FOREIGN_FIZZ,
      name: "Fizz",
      ownerPubkey: OTHER_OWNER,
      status: "online",
      respondTo: "anyone",
      respondToAllowlist: [],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
    {
      pubkey: ARCHIVED_FIZZ,
      name: "Fizz",
      ownerPubkey: CURRENT,
      status: "online",
      respondTo: "anyone",
      respondToAllowlist: [],
      channelIds: [channelId],
      agentType: "openclaw",
      channels: [],
      capabilities: [],
    },
  ];

  const mentionableAgentPubkeys = getMentionableAgentPubkeys({
    currentPubkey: CURRENT,
    phase: "publish",
    eligibilityScope: { type: "channel", channelId },
    managedAgentPubkeys: [],
    relayAgents,
    sharedChannelIds: new Set([channelId, "elsewhere"]),
    includePublicAnyone: false,
  });

  const candidates = buildMentionCandidates(
    input({
      currentPubkey: CURRENT,
      mentionChannelId: channelId,
      memberPubkeys: new Set([LOCAL_FIZZ, FOREIGN_FIZZ, ARCHIVED_FIZZ]),
      mentionableAgentPubkeys,
      relayAgents,
      isArchived: (pubkey) => pubkey === ARCHIVED_FIZZ,
    }),
  );

  const fizz = candidates.filter(
    (candidate) => candidate.displayName === "Fizz",
  );
  assert.deepEqual(
    fizz.map((candidate) => candidate.pubkey),
    [LOCAL_FIZZ],
  );
});
