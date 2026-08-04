import assert from "node:assert/strict";
import test from "node:test";

import {
  coalesceAgentAutocompleteCandidates,
  getMentionableAgentPubkeys,
  getOwnedMemberAgentPubkeys,
  getSharedChannelIds,
  isAgentIdentityInManagedList,
  isAgentMentionEligible,
  relayAgentIsSharedWithUser,
  shouldHideAgentFromMentions,
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

test("isAgentIdentityInManagedList: keeps owned agents that are channel members", () => {
  // An agent whose process lives on another machine is absent from this
  // machine's managed list, but its owner must still be able to mention it.
  const managedAgentPubkeys = new Set([PUB_A]);

  assert.equal(
    isAgentIdentityInManagedList(
      {
        isAgent: true,
        isMember: true,
        pubkey: PUB_B,
        ownerPubkey: CURRENT_PUBKEY,
      },
      managedAgentPubkeys,
      CURRENT_PUBKEY,
    ),
    true,
  );
});

test("isAgentIdentityInManagedList: normalizes pubkeys on the ownership branch", () => {
  assert.equal(
    isAgentIdentityInManagedList(
      {
        isAgent: true,
        isMember: true,
        pubkey: PUB_B,
        ownerPubkey: CURRENT_PUBKEY.toUpperCase(),
      },
      new Set(),
      CURRENT_PUBKEY,
    ),
    true,
  );
});

test("isAgentIdentityInManagedList: still hides owned agents that are not members", () => {
  // Ownership alone is not reachability: a profile-only agent we own has no
  // evidence of running anywhere, and #1243 hides it on purpose.
  assert.equal(
    isAgentIdentityInManagedList(
      {
        isAgent: true,
        isMember: false,
        pubkey: PUB_B,
        ownerPubkey: CURRENT_PUBKEY,
      },
      new Set(),
      CURRENT_PUBKEY,
    ),
    false,
  );
  assert.equal(
    isAgentIdentityInManagedList(
      { isAgent: true, pubkey: PUB_B, ownerPubkey: CURRENT_PUBKEY },
      new Set(),
      CURRENT_PUBKEY,
    ),
    false,
  );
});

test("isAgentIdentityInManagedList: still hides agents owned by someone else", () => {
  assert.equal(
    isAgentIdentityInManagedList(
      {
        isAgent: true,
        isMember: true,
        pubkey: PUB_B,
        ownerPubkey: PUB_D,
      },
      new Set(),
      CURRENT_PUBKEY,
    ),
    false,
  );
});

test("isAgentIdentityInManagedList: ownership branch is inert without a current pubkey", () => {
  // Callers that do not pass `currentPubkey` keep the previous behaviour.
  assert.equal(
    isAgentIdentityInManagedList(
      {
        isAgent: true,
        isMember: true,
        pubkey: PUB_B,
        ownerPubkey: CURRENT_PUBKEY,
      },
      new Set(),
    ),
    false,
  );
  assert.equal(
    isAgentIdentityInManagedList(
      { isAgent: true, isMember: true, pubkey: PUB_B, ownerPubkey: null },
      new Set(),
      CURRENT_PUBKEY,
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

test("shouldHideAgentFromMentions: shows directory agents owned by the current user", () => {
  // An `owner-only` agent is absent from `mentionableAgentPubkeys` by
  // construction, so its directory entry must not read as not-invocable for
  // the one account it always answers.
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      ownerPubkey: CURRENT_PUBKEY.toUpperCase(),
      currentPubkey: CURRENT_PUBKEY,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set([PUB_A]),
    }),
    false,
  );
});

test("shouldHideAgentFromMentions: still hides directory agents owned by someone else", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      ownerPubkey: PUB_D,
      currentPubkey: CURRENT_PUBKEY,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set([PUB_A]),
    }),
    true,
  );
});

test("shouldHideAgentFromMentions: still hides non-member agents owned by the current user", () => {
  // The ownership escape is scoped to members: membership is the reachability
  // evidence, ownership only the permission.
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: false,
      pubkey: PUB_A,
      ownerPubkey: CURRENT_PUBKEY,
      currentPubkey: CURRENT_PUBKEY,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set(),
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

test("isAgentMentionEligible: offers an owned remote agent that is a channel member", () => {
  // The reported bug: the agent's process runs on another machine, so it is
  // absent from `managedAgentPubkeys`, and its default `owner-only` policy
  // keeps it out of `mentionableAgentPubkeys`. Its directory entry then reads
  // as an explicit exclusion. It is still a member, still answering, and still
  // ours.
  assert.equal(
    isAgentMentionEligible({
      candidate: {
        isAgent: true,
        isMember: true,
        pubkey: PUB_B,
        ownerPubkey: CURRENT_PUBKEY,
      },
      currentPubkey: CURRENT_PUBKEY,
      directoryAgentPubkeys: new Set([PUB_B]),
      managedAgentPubkeys: new Set(),
      mentionableAgentPubkeys: new Set(),
    }),
    true,
  );
});

test("isAgentMentionEligible: keeps hiding an owned agent that is not a member", () => {
  assert.equal(
    isAgentMentionEligible({
      candidate: {
        isAgent: true,
        isMember: false,
        pubkey: PUB_B,
        ownerPubkey: CURRENT_PUBKEY,
      },
      currentPubkey: CURRENT_PUBKEY,
      directoryAgentPubkeys: new Set(),
      managedAgentPubkeys: new Set(),
      mentionableAgentPubkeys: new Set(),
    }),
    false,
  );
});

test("isAgentMentionEligible: keeps hiding a member agent owned by someone else", () => {
  assert.equal(
    isAgentMentionEligible({
      candidate: {
        isAgent: true,
        isMember: true,
        pubkey: PUB_B,
        ownerPubkey: PUB_D,
      },
      currentPubkey: CURRENT_PUBKEY,
      directoryAgentPubkeys: new Set([PUB_B]),
      managedAgentPubkeys: new Set(),
      mentionableAgentPubkeys: new Set(),
    }),
    false,
  );
});

test("isAgentMentionEligible: leaves people alone", () => {
  assert.equal(
    isAgentMentionEligible({
      candidate: { isAgent: false, isMember: false, pubkey: PUB_B },
      currentPubkey: CURRENT_PUBKEY,
      directoryAgentPubkeys: new Set([PUB_B]),
      managedAgentPubkeys: new Set(),
      mentionableAgentPubkeys: new Set(),
    }),
    true,
  );
});

test("isAgentMentionEligible: applies both gates, not just the managed-list one", () => {
  // A managed agent still goes through the invocability gate; a directory
  // entry that excludes us wins over local management for a member.
  assert.equal(
    isAgentMentionEligible({
      candidate: { isAgent: true, isMember: true, pubkey: PUB_A },
      currentPubkey: CURRENT_PUBKEY,
      directoryAgentPubkeys: new Set([PUB_A]),
      managedAgentPubkeys: new Set([PUB_A]),
      mentionableAgentPubkeys: new Set(),
    }),
    false,
  );
});

test("getOwnedMemberAgentPubkeys: collects in-channel agents owned by the current user", () => {
  // These are exactly the agents the ownership branch newly admits, so
  // downstream agent classification must recognise them too.
  assert.deepEqual(
    getOwnedMemberAgentPubkeys(
      [
        { isAgent: true, isMember: true, pubkey: PUB_A.toUpperCase(), ownerPubkey: CURRENT_PUBKEY },
        { isAgent: true, isMember: false, pubkey: PUB_B, ownerPubkey: CURRENT_PUBKEY },
        { isAgent: true, isMember: true, pubkey: PUB_C, ownerPubkey: PUB_D },
        { isAgent: false, isMember: true, pubkey: PUB_D, ownerPubkey: CURRENT_PUBKEY },
        { isAgent: true, isMember: true, ownerPubkey: CURRENT_PUBKEY },
      ],
      CURRENT_PUBKEY,
    ),
    new Set([PUB_A]),
  );
});

test("getOwnedMemberAgentPubkeys: empty without a current pubkey", () => {
  assert.deepEqual(
    getOwnedMemberAgentPubkeys(
      [{ isAgent: true, isMember: true, pubkey: PUB_A, ownerPubkey: CURRENT_PUBKEY }],
      null,
    ),
    new Set(),
  );
});
