import assert from "node:assert/strict";
import test from "node:test";

import {
  coalesceAgentAutocompleteCandidates,
  getMentionableAgentPubkeys,
  getSharedChannelIds,
  isAgentIdentityInManagedList,
  isAgentIdentityReachableForMentions,
  relayAgentIsSharedWithUser,
  shouldHideAgentFromMentions,
  shouldOfferAgentIdentityForMentions,
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
        respondTo: "owner-only",
        respondToAllowlist: [],
        channelIds: ["general"],
      },
      sharedChannelIds,
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

test("getMentionableAgentPubkeys: keeps managed agents and shared relay agents", () => {
  const result = getMentionableAgentPubkeys({
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

test("isAgentIdentityInManagedList: keeps people and only current managed agent identities", () => {
  const managedAgentPubkeys = new Set([PUB_A]);

  assert.equal(
    isAgentIdentityInManagedList(
      { isAgent: false, pubkey: PUB_B },
      managedAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityInManagedList(
      { isAgent: true, pubkey: PUB_A.toUpperCase() },
      managedAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityInManagedList(
      { isAgent: true, pubkey: PUB_B },
      managedAgentPubkeys,
    ),
    false,
  );
});

test("isAgentIdentityReachableForMentions: keeps channel members the viewer does not manage", () => {
  const managedAgentPubkeys = new Set([PUB_A]);

  assert.equal(
    isAgentIdentityReachableForMentions(
      { isAgent: true, isMember: true, pubkey: PUB_B },
      managedAgentPubkeys,
      true,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityReachableForMentions(
      { isAgent: true, isMember: false, pubkey: PUB_B },
      managedAgentPubkeys,
      true,
    ),
    false,
  );
  assert.equal(
    isAgentIdentityReachableForMentions(
      { isAgent: true, isMember: false, pubkey: PUB_A.toUpperCase() },
      managedAgentPubkeys,
      true,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityReachableForMentions(
      { isAgent: false, isMember: false, pubkey: PUB_B },
      managedAgentPubkeys,
      true,
    ),
    true,
  );
});

test("isAgentIdentityReachableForMentions: holds the member pass-through until the relay directory is ready", () => {
  const managedAgentPubkeys = new Set([PUB_A]);

  assert.equal(
    isAgentIdentityReachableForMentions(
      { isAgent: true, isMember: true, pubkey: PUB_B },
      managedAgentPubkeys,
      false,
    ),
    false,
  );
  // A managed agent and a human are unaffected by the directory load state.
  assert.equal(
    isAgentIdentityReachableForMentions(
      { isAgent: true, isMember: true, pubkey: PUB_A },
      managedAgentPubkeys,
      false,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityReachableForMentions(
      { isAgent: false, isMember: true, pubkey: PUB_B },
      managedAgentPubkeys,
      false,
    ),
    true,
  );
});

/**
 * Drives the real admission check `useMentions`' `addCandidate` calls, building
 * its pubkey sets the same way the hook does from `relayAgentsQuery.data`.
 * `tests/e2e/mentions.spec.ts` covers the same scenarios through the composer.
 */
function mentionCandidateIsOffered(
  candidate,
  {
    managedAgentPubkeys,
    relayAgents,
    sharedChannelIds,
    relayAgentDirectoryReady = true,
  },
) {
  return shouldOfferAgentIdentityForMentions({
    candidate,
    managedAgentPubkeys,
    mentionableAgentPubkeys: getMentionableAgentPubkeys({
      currentPubkey: CURRENT_PUBKEY,
      managedAgentPubkeys,
      relayAgents,
      sharedChannelIds,
    }),
    directoryAgentPubkeys: new Set(
      relayAgents.map((agent) => agent.pubkey.toLowerCase()),
    ),
    relayAgentDirectoryReady,
  });
}

test("mention path offers an invocable channel bot from another install", () => {
  const crossOwnerBot = { isAgent: true, isMember: true, pubkey: PUB_B };

  assert.equal(
    mentionCandidateIsOffered(crossOwnerBot, {
      managedAgentPubkeys: new Set([PUB_A]),
      relayAgents: [
        {
          pubkey: PUB_B,
          respondTo: "allowlist",
          respondToAllowlist: [CURRENT_PUBKEY],
          channelIds: ["general"],
        },
      ],
      sharedChannelIds: new Set(["general"]),
    }),
    true,
  );
  assert.equal(
    mentionCandidateIsOffered(crossOwnerBot, {
      managedAgentPubkeys: new Set([PUB_A]),
      relayAgents: [
        {
          pubkey: PUB_B,
          respondTo: "anyone",
          respondToAllowlist: [],
          channelIds: ["general"],
        },
      ],
      sharedChannelIds: new Set(["general"]),
    }),
    true,
  );
});

test("mention path still hides a channel bot whose directory entry excludes the viewer", () => {
  assert.equal(
    mentionCandidateIsOffered(
      { isAgent: true, isMember: true, pubkey: PUB_B },
      {
        managedAgentPubkeys: new Set([PUB_A]),
        relayAgents: [
          {
            pubkey: PUB_B,
            respondTo: "owner-only",
            respondToAllowlist: [],
            channelIds: ["general"],
          },
        ],
        sharedChannelIds: new Set(["general"]),
      },
    ),
    false,
  );
  assert.equal(
    mentionCandidateIsOffered(
      { isAgent: true, isMember: true, pubkey: PUB_B },
      {
        managedAgentPubkeys: new Set([PUB_A]),
        relayAgents: [
          {
            pubkey: PUB_B,
            respondTo: "allowlist",
            respondToAllowlist: [OTHER_OWNER_PUBKEY],
            channelIds: ["general"],
          },
        ],
        sharedChannelIds: new Set(["general"]),
      },
    ),
    false,
  );
});

test("mention path does not offer a channel bot while the relay directory is still loading", () => {
  // In flight, `relayAgents` is empty: the bot is neither invocable nor
  // directory-present, so `shouldHideAgentFromMentions` would read it as
  // unknown-invocability and show it. Readiness is what keeps it hidden until
  // the directory can answer.
  assert.equal(
    mentionCandidateIsOffered(
      { isAgent: true, isMember: true, pubkey: PUB_B },
      {
        managedAgentPubkeys: new Set([PUB_A]),
        relayAgents: [],
        sharedChannelIds: new Set(["general"]),
        relayAgentDirectoryReady: false,
      },
    ),
    false,
  );
  // Once ready, an empty or errored directory falls back to Option B (unknown
  // invocability => show) rather than hiding members indefinitely.
  assert.equal(
    mentionCandidateIsOffered(
      { isAgent: true, isMember: true, pubkey: PUB_B },
      {
        managedAgentPubkeys: new Set([PUB_A]),
        relayAgents: [],
        sharedChannelIds: new Set(["general"]),
        relayAgentDirectoryReady: true,
      },
    ),
    true,
  );
});

test("mention path keeps unreachable non-member agents out of the composer", () => {
  assert.equal(
    mentionCandidateIsOffered(
      { isAgent: true, isMember: false, pubkey: PUB_B },
      {
        managedAgentPubkeys: new Set([PUB_A]),
        relayAgents: [],
        sharedChannelIds: new Set(["general"]),
      },
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

test("shouldHideAgentFromMentions: shows member agents with unknown invocability (not in directory)", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set(),
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

test("coalesceAgentAutocompleteCandidates: merges agents with the same persona id", () => {
  const first = makeAgent({ pubkey: PUB_A, personaId: "pinky" });
  const second = makeAgent({
    pubkey: PUB_B,
    personaId: "pinky",
    isMember: true,
  });

  assert.deepEqual(coalesce([first, second]), [second]);
});

test("coalesceAgentAutocompleteCandidates: merges agents with the same owner and name", () => {
  const first = makeAgent({ pubkey: PUB_A, ownerPubkey: OWNER_PUBKEY });
  const second = makeAgent({
    pubkey: PUB_B,
    ownerPubkey: OWNER_PUBKEY,
    isMember: true,
  });

  assert.deepEqual(coalesce([first, second]), [second]);
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

test("coalesceAgentAutocompleteCandidates: merges current-owner same-name agents", () => {
  const first = makeAgent({ pubkey: PUB_A, ownerPubkey: CURRENT_PUBKEY });
  const second = makeAgent({
    pubkey: PUB_B,
    ownerPubkey: CURRENT_PUBKEY,
    isManagedAgent: true,
  });

  assert.deepEqual(coalesce([first, second]), [second]);
});

test("coalesceAgentAutocompleteCandidates: leaves non-agents alone", () => {
  const first = makeAgent({ pubkey: PUB_A, isAgent: false });
  const second = makeAgent({ pubkey: PUB_B, isAgent: false });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});
