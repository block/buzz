import assert from "node:assert/strict";
import test from "node:test";

import {
  coalesceAgentAutocompleteCandidates,
  filterAdmittedMentionPubkeys,
  filterCachedAgentSuggestions,
  getAgentMentionAdmission,
  getDirectoryGatedAgentPubkeys,
  getMentionableAgentPubkeys,
  getSharedChannelIds,
  isAgentDirectoryReady,
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

test("isAgentDirectoryReady: requires successful cached directory evidence", () => {
  assert.equal(isAgentDirectoryReady({ data: [], error: null }), true);
  assert.equal(isAgentDirectoryReady({ data: undefined, error: null }), false);
  assert.equal(
    isAgentDirectoryReady({ data: [], error: new Error("offline") }),
    false,
  );
});

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
  assert.equal(isAgentMentionChannelType("dm"), false);
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
  const archivedPerson = { pubkey: PUB_D, isAgent: false };
  const persona = { isAgent: true };
  assert.deepEqual(
    filterCachedAgentSuggestions(
      [person, admittedAgent, removedAgent, archivedPerson, persona],
      [
        { pubkey: PUB_B, isAgent: true },
        // Live set omits PUB_A / PUB_C / PUB_D (e.g. archived or no longer eligible).
      ],
    ),
    [admittedAgent, persona],
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

test("getDirectoryGatedAgentPubkeys: current-channel catalog bots are not directory-gated", () => {
  const catalogBot = PUB_A;
  const managedAgent = PUB_B;
  const nonMemberAgent = PUB_C;
  const humanMember = PUB_D;

  const gated = getDirectoryGatedAgentPubkeys({
    agentPubkeys: [catalogBot, managedAgent, nonMemberAgent],
    memberPubkeys: [catalogBot, humanMember],
    managedAgentPubkeys: [managedAgent],
    relayAgents: [],
  });

  assert.equal(gated.has(catalogBot), false);
  assert.equal(gated.has(humanMember), false);
  assert.equal(gated.has(managedAgent), true);
  assert.equal(gated.has(nonMemberAgent), true);
});

test("getDirectoryGatedAgentPubkeys: current-channel managed agents stay gated", () => {
  const gated = getDirectoryGatedAgentPubkeys({
    agentPubkeys: [PUB_A],
    memberPubkeys: [PUB_A],
    managedAgentPubkeys: [PUB_A],
    relayAgents: [],
  });

  assert.deepEqual(gated, new Set([PUB_A]));
});

test("getDirectoryGatedAgentPubkeys: current-channel relay agents stay gated", () => {
  const gated = getDirectoryGatedAgentPubkeys({
    agentPubkeys: [PUB_B],
    memberPubkeys: [PUB_B],
    managedAgentPubkeys: [],
    relayAgents: [{ pubkey: PUB_B }],
  });

  assert.deepEqual(gated, new Set([PUB_B]));
});

test("getDirectoryGatedAgentPubkeys: non-member agents stay gated without a directory grant", () => {
  const gated = getDirectoryGatedAgentPubkeys({
    agentPubkeys: [PUB_C],
    memberPubkeys: [PUB_A],
    managedAgentPubkeys: [],
    relayAgents: [],
  });

  assert.deepEqual(gated, new Set([PUB_C]));
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

test("owners remain admitted by allowlist policy without listing themselves", () => {
  assert.equal(
    relayAgentCanRespondInChannel(
      {
        ownerPubkey: CURRENT_PUBKEY,
        respondTo: "allowlist",
        respondToAllowlist: [],
        channelIds: ["general"],
      },
      "general",
      CURRENT_PUBKEY,
    ),
    true,
  );
});

test("owned discovery does not require a shared channel, but sending does", () => {
  for (const respondTo of ["owner-only", "allowlist", "anyone"]) {
    const agent = {
      pubkey: PUB_B,
      ownerPubkey: CURRENT_PUBKEY,
      respondTo,
      respondToAllowlist: [],
      channelIds: [],
    };
    assert.equal(
      relayAgentIsSharedWithUser(agent, new Set(), CURRENT_PUBKEY),
      true,
    );
    assert.equal(
      relayAgentCanRespondInChannel(agent, "general", CURRENT_PUBKEY),
      false,
    );
  }
});

test("DM ownership is independent of local configuration and still requires membership", () => {
  const base = {
    currentPubkey: CURRENT_PUBKEY,
    managedAgentPubkeys: [PUB_A],
    sharedChannelIds: new Set(),
    relayAgents: [
      {
        pubkey: PUB_B,
        ownerPubkey: CURRENT_PUBKEY,
        respondTo: "allowlist",
        respondToAllowlist: [],
        channelIds: ["dm"],
      },
      {
        pubkey: PUB_C,
        ownerPubkey: OTHER_OWNER_PUBKEY,
        respondTo: "anyone",
        respondToAllowlist: [],
        channelIds: ["dm"],
      },
      {
        pubkey: PUB_D,
        ownerPubkey: CURRENT_PUBKEY,
        respondTo: "nobody",
        respondToAllowlist: [],
        channelIds: ["dm"],
      },
    ],
  };
  assert.deepEqual(
    getMentionableAgentPubkeys({
      ...base,
      eligibilityScope: { type: "owned", channelId: "dm" },
    }),
    new Set([PUB_A, PUB_B]),
  );
  assert.deepEqual(
    getMentionableAgentPubkeys({
      ...base,
      eligibilityScope: { type: "owned", channelId: "other" },
    }),
    new Set([PUB_A]),
  );
  assert.deepEqual(
    getMentionableAgentPubkeys({
      ...base,
      eligibilityScope: { type: "owned", channelId: null },
      phase: "prepare",
    }),
    new Set([PUB_A, PUB_B]),
  );
});

test("relayAgentIsSharedWithUser: includePublicAnyone false keeps owners and allowlists, drops foreign anyone", () => {
  const sharedChannelIds = new Set(["general"]);
  const foreignAnyone = {
    ownerPubkey: OTHER_OWNER_PUBKEY,
    respondTo: "anyone",
    respondToAllowlist: [],
    channelIds: ["general"],
  };
  assert.equal(
    relayAgentIsSharedWithUser(
      foreignAnyone,
      sharedChannelIds,
      CURRENT_PUBKEY,
      { includePublicAnyone: false },
    ),
    false,
  );
  assert.equal(
    relayAgentIsSharedWithUser(
      {
        ownerPubkey: CURRENT_PUBKEY,
        respondTo: "anyone",
        respondToAllowlist: [],
        channelIds: ["general"],
      },
      sharedChannelIds,
      CURRENT_PUBKEY,
      { includePublicAnyone: false },
    ),
    true,
  );
  assert.equal(
    relayAgentIsSharedWithUser(
      {
        ownerPubkey: OTHER_OWNER_PUBKEY,
        respondTo: "allowlist",
        respondToAllowlist: [CURRENT_PUBKEY],
        channelIds: ["general"],
      },
      sharedChannelIds,
      CURRENT_PUBKEY,
      { includePublicAnyone: false },
    ),
    true,
  );
});

test("getMentionableAgentPubkeys: channel publish hides owned clones outside this channel", () => {
  const inChannel = {
    pubkey: PUB_A,
    ownerPubkey: CURRENT_PUBKEY,
    respondTo: "anyone",
    respondToAllowlist: [],
    channelIds: ["general"],
  };
  const otherChannelClone = {
    pubkey: PUB_B,
    ownerPubkey: CURRENT_PUBKEY,
    respondTo: "owner-only",
    respondToAllowlist: [],
    channelIds: ["elsewhere"],
  };
  const unjoinedClone = {
    pubkey: PUB_C,
    ownerPubkey: CURRENT_PUBKEY,
    respondTo: "anyone",
    respondToAllowlist: [],
    channelIds: [],
  };
  const foreignInChannel = {
    pubkey: PUB_D,
    ownerPubkey: OTHER_OWNER_PUBKEY,
    respondTo: "anyone",
    respondToAllowlist: [],
    channelIds: ["general"],
  };
  const base = {
    currentPubkey: CURRENT_PUBKEY,
    eligibilityScope: { type: "channel", channelId: "general" },
    managedAgentPubkeys: [],
    relayAgents: [
      inChannel,
      otherChannelClone,
      unjoinedClone,
      foreignInChannel,
    ],
    sharedChannelIds: new Set(["general", "elsewhere"]),
    includePublicAnyone: false,
  };

  // prepare admitted every owned clone (the @Fizz flood).
  assert.deepEqual(
    getMentionableAgentPubkeys({ ...base, phase: "prepare" }),
    new Set([PUB_A, PUB_B, PUB_C]),
  );
  // publish matches send-time: only the in-channel owned agent.
  assert.deepEqual(
    getMentionableAgentPubkeys({ ...base, phase: "publish" }),
    new Set([PUB_A]),
  );
});

test("getMentionableAgentPubkeys: mention autocomplete hides foreign anyone agents unless extras admit them", () => {
  const own = {
    pubkey: PUB_A,
    ownerPubkey: CURRENT_PUBKEY,
    respondTo: "anyone",
    respondToAllowlist: [],
    channelIds: ["general"],
  };
  const foreignAnyone = {
    pubkey: PUB_B,
    ownerPubkey: OTHER_OWNER_PUBKEY,
    respondTo: "anyone",
    respondToAllowlist: [],
    channelIds: ["general"],
  };
  const allowlisted = {
    pubkey: PUB_C,
    ownerPubkey: OTHER_OWNER_PUBKEY,
    respondTo: "allowlist",
    respondToAllowlist: [CURRENT_PUBKEY],
    channelIds: ["general"],
  };
  const communityBot = {
    pubkey: PUB_D,
    ownerPubkey: OTHER_OWNER_PUBKEY,
    respondTo: "anyone",
    respondToAllowlist: [],
    channelIds: ["general"],
  };
  const base = {
    currentPubkey: CURRENT_PUBKEY,
    eligibilityScope: { type: "channel", channelId: "general" },
    phase: "prepare",
    managedAgentPubkeys: [],
    relayAgents: [own, foreignAnyone, allowlisted, communityBot],
    sharedChannelIds: new Set(["general"]),
  };

  assert.deepEqual(
    getMentionableAgentPubkeys({
      ...base,
      includePublicAnyone: false,
    }),
    new Set([PUB_A, PUB_C]),
  );
  assert.deepEqual(
    getMentionableAgentPubkeys({
      ...base,
      includePublicAnyone: false,
      extraMentionablePubkeys: [PUB_D],
    }),
    new Set([PUB_A, PUB_C, PUB_D]),
  );
  // Default remains permissive for directory pickers.
  assert.deepEqual(
    getMentionableAgentPubkeys(base),
    new Set([PUB_A, PUB_B, PUB_C, PUB_D]),
  );
});
