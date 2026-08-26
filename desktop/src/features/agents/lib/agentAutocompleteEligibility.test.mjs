import assert from "node:assert/strict";
import test from "node:test";

import {
  coalesceAgentAutocompleteCandidates,
  filterAdmittedMentionPubkeys,
  filterCachedAgentSuggestions,
  getAgentMentionAdmission,
  getMentionableAgentPubkeys,
  getSharedChannelIds,
  isAgentIdentityInAllowedList,
  isAgentMentionChannelType,
  relayAgentCanRespondInChannel,
  relayAgentIsSharedWithUser,
  shouldHideAgentFromMentions,
  uniqueAutocompleteLabels,
} from "./agentAutocompleteEligibility.ts";

const CURRENT_PUBKEY = "a".repeat(64);
const OWNER_PUBKEY = "b".repeat(64);
const OTHER_OWNER_PUBKEY = "c".repeat(64);
const PUB_A = "1".repeat(64);
const PUB_B = "2".repeat(64);
const PUB_C = "3".repeat(64);
const PUB_D = "4".repeat(64);

function coalesce(candidates, options = {}) {
  return coalesceAgentAutocompleteCandidates(candidates, {
    currentPubkey: CURRENT_PUBKEY,
    getLabel: (candidate) => candidate.displayName,
    ...options,
  });
}

function makeAgent(overrides = {}) {
  return {
    pubkey: PUB_A,
    displayName: "Pinky",
    isAgent: true,
    isMember: false,
    ...overrides,
  };
}

test("getSharedChannelIds: includes only active joined channels", () => {
  assert.deepEqual(
    getSharedChannelIds([
      { id: "joined", isMember: true, archivedAt: null },
      { id: "not-joined", isMember: false, archivedAt: null },
      { id: "archived", isMember: true, archivedAt: "2026-01-01T00:00:00Z" },
    ]),
    new Set(["joined"]),
  );
});

test("relayAgentIsSharedWithUser: accepts shared anyone agents and rejects unshared ones", () => {
  const sharedChannelIds = new Set(["general"]);

  assert.equal(
    relayAgentIsSharedWithUser(
      { respondTo: "anyone", respondToAllowlist: [], channelIds: ["general"] },
      sharedChannelIds,
    ),
    true,
  );
  assert.equal(
    relayAgentIsSharedWithUser(
      {
        ownerPubkey: OTHER_OWNER_PUBKEY,
        respondTo: "owner-only",
        respondToAllowlist: [],
        channelIds: ["general"],
      },
      sharedChannelIds,
      CURRENT_PUBKEY,
    ),
    false,
  );
  assert.equal(
    relayAgentIsSharedWithUser(
      { respondTo: "anyone", respondToAllowlist: [], channelIds: ["other"] },
      sharedChannelIds,
    ),
    false,
  );
});

test("relayAgentIsSharedWithUser: accepts verified same-owner agents across machines", () => {
  assert.equal(
    relayAgentIsSharedWithUser(
      {
        ownerPubkey: CURRENT_PUBKEY.toUpperCase(),
        respondTo: "owner-only",
        respondToAllowlist: [],
        channelIds: ["general"],
      },
      new Set(["general"]),
      CURRENT_PUBKEY,
    ),
    true,
  );
});

test("relayAgentIsSharedWithUser: accepts allowlist agents for the current user", () => {
  const sharedChannelIds = new Set(["general"]);

  assert.equal(
    relayAgentIsSharedWithUser(
      {
        respondTo: "allowlist",
        respondToAllowlist: [OTHER_OWNER_PUBKEY, CURRENT_PUBKEY.toUpperCase()],
        channelIds: ["other"],
      },
      sharedChannelIds,
      CURRENT_PUBKEY,
    ),
    true,
  );
  assert.equal(
    relayAgentIsSharedWithUser(
      {
        respondTo: "allowlist",
        respondToAllowlist: [OTHER_OWNER_PUBKEY],
        channelIds: ["general"],
      },
      sharedChannelIds,
      CURRENT_PUBKEY,
    ),
    false,
  );
});

test("relayAgentCanRespondInChannel: requires exact channel membership and viewer access", () => {
  const agent = {
    respondTo: "allowlist",
    respondToAllowlist: [CURRENT_PUBKEY],
    channelIds: ["general"],
  };

  assert.equal(
    relayAgentCanRespondInChannel(agent, "general", CURRENT_PUBKEY),
    true,
  );
  assert.equal(
    relayAgentCanRespondInChannel(agent, "other", CURRENT_PUBKEY),
    false,
  );
  assert.equal(
    relayAgentCanRespondInChannel(agent, "general", OTHER_OWNER_PUBKEY),
    false,
  );
});

test("getMentionableAgentPubkeys: keeps managed agents and shared relay agents", () => {
  const result = getMentionableAgentPubkeys({
    eligibilityScope: { type: "community" },
    managedAgentPubkeys: [PUB_A],
    currentPubkey: CURRENT_PUBKEY,
    relayAgents: [
      {
        pubkey: PUB_B,
        respondTo: "anyone",
        respondToAllowlist: [],
        channelIds: ["general"],
      },
      {
        pubkey: PUB_C,
        respondTo: "allowlist",
        respondToAllowlist: [CURRENT_PUBKEY],
        channelIds: ["other"],
      },
      {
        pubkey: PUB_D,
        respondTo: "anyone",
        respondToAllowlist: [],
        channelIds: ["other"],
      },
    ],
    sharedChannelIds: new Set(["general"]),
  });

  assert.deepEqual(result, new Set([PUB_A, PUB_B, PUB_C]));
});

test("getMentionableAgentPubkeys: scopes channel composers and fails closed without context", () => {
  const relayAgents = [
    {
      pubkey: PUB_B,
      respondTo: "allowlist",
      respondToAllowlist: [CURRENT_PUBKEY],
      channelIds: ["general"],
    },
  ];
  const base = {
    currentPubkey: CURRENT_PUBKEY,
    managedAgentPubkeys: [PUB_A],
    relayAgents,
    sharedChannelIds: new Set(["general"]),
  };

  assert.deepEqual(
    getMentionableAgentPubkeys({
      ...base,
      eligibilityScope: { type: "channel", channelId: "general" },
    }),
    new Set([PUB_A, PUB_B]),
  );
  assert.deepEqual(
    getMentionableAgentPubkeys({
      ...base,
      eligibilityScope: { type: "channel", channelId: "other" },
    }),
    new Set([PUB_A]),
  );
  assert.deepEqual(
    getMentionableAgentPubkeys({
      ...base,
      eligibilityScope: { type: "managed-only" },
    }),
    new Set([PUB_A]),
  );
});

test("autocomplete helper extraction preserves safe filtering and labels", () => {
  assert.equal(isAgentMentionChannelType("stream"), true);
  assert.equal(isAgentMentionChannelType("forum"), true);
  assert.equal(isAgentMentionChannelType("dm"), true);
  assert.equal(isAgentMentionChannelType(null), false);

  assert.deepEqual(
    uniqueAutocompleteLabels([
      { displayName: " Alice ", personaName: "alice" },
      { displayName: null, secondaryLabel: "Bob" },
      { displayName: "BOB" },
    ]),
    ["Alice", "Bob"],
  );

  const person = { pubkey: PUB_A, isAgent: false };
  const admittedAgent = { pubkey: PUB_B.toUpperCase(), isAgent: true };
  const removedAgent = { pubkey: PUB_C, isAgent: true };
  const persona = { isAgent: true };
  assert.deepEqual(
    filterCachedAgentSuggestions(
      [person, admittedAgent, removedAgent, persona],
      [{ pubkey: PUB_B, isAgent: true }],
    ),
    [person, admittedAgent, persona],
  );
});

test("isAgentIdentityInAllowedList: keeps people and only explicitly allowed agent identities", () => {
  const allowedAgentPubkeys = new Set([PUB_A]);

  assert.equal(
    isAgentIdentityInAllowedList(
      { isAgent: false, pubkey: PUB_B },
      allowedAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityInAllowedList(
      { isAgent: true, pubkey: PUB_A.toUpperCase() },
      allowedAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityInAllowedList(
      { isAgent: true, pubkey: PUB_B },
      allowedAgentPubkeys,
    ),
    false,
  );
});

test("shouldHideAgentFromMentions: never hides non-agents", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: false,
      isMember: false,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set([PUB_A]),
    }),
    false,
  );
});

test("shouldHideAgentFromMentions: shows invocable agents even when non-member", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: false,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set([PUB_A]),
      directoryAgentPubkeys: new Set([PUB_A]),
    }),
    false,
  );
});

test("shouldHideAgentFromMentions: hides non-member non-invocable agents", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: false,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set(),
    }),
    true,
  );
});

test("shouldHideAgentFromMentions: hides member agents with an explicit not-invocable directory entry (Fizz)", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set([PUB_A]),
    }),
    true,
  );
});

test("shouldHideAgentFromMentions: hides member agents without an affirmative directory grant", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set(),
    }),
    true,
  );
});

test("shouldHideAgentFromMentions: hides unknown member agents while directories load", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set(),
      directoryReady: false,
    }),
    true,
  );
});

test("shouldHideAgentFromMentions: hides mentionable member agents while directories load", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set([PUB_A]),
      directoryAgentPubkeys: new Set(),
      directoryReady: false,
    }),
    true,
  );
});

test("shouldHideAgentFromMentions: shows non-agent members while directories load", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: false,
      isMember: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set([PUB_A]),
      directoryReady: false,
    }),
    false,
  );
});

test("shouldHideAgentFromMentions: hides unknown member agents after empty directories settle", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set(),
      directoryReady: true,
    }),
    true,
  );
});

test("shouldHideAgentFromMentions: shows authorized agents without managed-owner policy", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set([PUB_A]),
      directoryReady: true,
    }),
    false,
  );
});

test("shouldHideAgentFromMentions: normalizes the pubkey before lookup", () => {
  const mixedCase = "Ab".repeat(32);
  const normalized = mixedCase.toLowerCase();

  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: mixedCase,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set([normalized]),
    }),
    true,
  );
});

test("getAgentMentionAdmission: authorized relay agents are independent of owner", () => {
  const common = {
    isAgent: true,
    pubkey: PUB_A,
    mentionableAgentPubkeys: new Set([PUB_A]),
    directoryReady: true,
  };

  assert.equal(getAgentMentionAdmission(common), "allow");
  assert.equal(
    getAgentMentionAdmission({
      ...common,
      mentionableAgentPubkeys: new Set(),
    }),
    "deny",
  );
});

test("getAgentMentionAdmission: unresolved directory state stays unknown", () => {
  assert.equal(
    getAgentMentionAdmission({
      isAgent: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set([PUB_A]),
      directoryReady: false,
    }),
    "unknown",
  );
});

test("filterAdmittedMentionPubkeys: rechecks agent admission without dropping people", () => {
  assert.deepEqual(
    filterAdmittedMentionPubkeys(
      [PUB_A, PUB_B, PUB_C],
      new Set([PUB_A, PUB_B]),
      new Set([PUB_B]),
    ),
    [PUB_B, PUB_C],
  );
});

test("coalesceAgentAutocompleteCandidates: keeps agents with the same persona id distinct", () => {
  const first = makeAgent({ pubkey: PUB_A, personaId: "pinky" });
  const second = makeAgent({
    pubkey: PUB_B,
    personaId: "pinky",
    isMember: true,
  });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

test("coalesceAgentAutocompleteCandidates: keeps agents with the same owner and name distinct", () => {
  const first = makeAgent({ pubkey: PUB_A, ownerPubkey: OWNER_PUBKEY });
  const second = makeAgent({
    pubkey: PUB_B,
    ownerPubkey: OWNER_PUBKEY,
    isMember: true,
  });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

test("coalesceAgentAutocompleteCandidates: keeps same-name agents with different owners distinct", () => {
  const first = makeAgent({ pubkey: PUB_A, ownerPubkey: OWNER_PUBKEY });
  const second = makeAgent({
    pubkey: PUB_B,
    ownerPubkey: OTHER_OWNER_PUBKEY,
  });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

test("coalesceAgentAutocompleteCandidates: keeps owner-less same-name agents distinct", () => {
  const first = makeAgent({ pubkey: PUB_A });
  const second = makeAgent({ pubkey: PUB_B });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

test("coalesceAgentAutocompleteCandidates: keeps owner-less managed same-name agents distinct", () => {
  const first = makeAgent({ pubkey: PUB_A, isManagedAgent: true });
  const second = makeAgent({ pubkey: PUB_B, isManagedAgent: true });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

test("coalesceAgentAutocompleteCandidates: keeps current-owner same-name agents distinct", () => {
  const first = makeAgent({ pubkey: PUB_A, ownerPubkey: CURRENT_PUBKEY });
  const second = makeAgent({
    pubkey: PUB_B,
    ownerPubkey: CURRENT_PUBKEY,
    isManagedAgent: true,
  });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

test("coalesceAgentAutocompleteCandidates: coalesces repeated source rows for the same pubkey", () => {
  const first = makeAgent({ pubkey: PUB_A });
  const second = makeAgent({
    pubkey: PUB_A.toUpperCase(),
    isMember: true,
  });

  assert.deepEqual(coalesce([first, second]), [second]);
});

test("coalesceAgentAutocompleteCandidates: leaves non-agents alone", () => {
  const first = makeAgent({ pubkey: PUB_A, isAgent: false });
  const second = makeAgent({ pubkey: PUB_B, isAgent: false });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});
