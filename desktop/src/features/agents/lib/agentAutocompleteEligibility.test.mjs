import assert from "node:assert/strict";
import test from "node:test";

import {
  coalesceAgentAutocompleteCandidates,
  getMentionableAgentPubkeys,
  getSharedChannelIds,
  isAgentIdentityInManagedList,
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

function hideArgs(overrides = {}) {
  const {
    candidate = {},
    currentPubkey = CURRENT_PUBKEY,
    managedAgentPubkeys = new Set(),
    relayAgentPolicies = new Map(),
  } = overrides;
  return {
    candidate: {
      isAgent: true,
      isMember: true,
      ownerPubkey: null,
      pubkey: PUB_A,
      ...candidate,
    },
    currentPubkey,
    managedAgentPubkeys,
    relayAgentPolicies,
  };
}

function policy(respondTo, respondToAllowlist = []) {
  return new Map([[PUB_A, { respondTo, respondToAllowlist }]]);
}

test("shouldHideAgentFromMentions: never hides non-agents", () => {
  assert.equal(
    shouldHideAgentFromMentions(
      hideArgs({ candidate: { isAgent: false, isMember: false } }),
    ),
    false,
  );
});

test("shouldHideAgentFromMentions: always offers managed agents, member or not", () => {
  // The local record is the desktop's own proof the agent will answer; it
  // also covers the non-member auto-add flow for managed agents.
  for (const isMember of [true, false]) {
    assert.equal(
      shouldHideAgentFromMentions(
        hideArgs({
          candidate: { isMember },
          managedAgentPubkeys: new Set([PUB_A]),
        }),
      ),
      false,
    );
  }
});

test("shouldHideAgentFromMentions: offers external member agents declaring respond-to anyone", () => {
  assert.equal(
    shouldHideAgentFromMentions(
      hideArgs({ relayAgentPolicies: policy("anyone") }),
    ),
    false,
  );
});

test("shouldHideAgentFromMentions: allowlist admits only listed users", () => {
  assert.equal(
    shouldHideAgentFromMentions(
      hideArgs({ relayAgentPolicies: policy("allowlist", [CURRENT_PUBKEY]) }),
    ),
    false,
  );
  assert.equal(
    shouldHideAgentFromMentions(
      hideArgs({ relayAgentPolicies: policy("allowlist", [OWNER_PUBKEY]) }),
    ),
    true,
  );
});

test("shouldHideAgentFromMentions: owner-only admits the verified owner", () => {
  // ownerPubkey is the NIP-OA-verified value the "managed by" surface
  // renders — external agents owned by the current user mention like
  // managed ones.
  assert.equal(
    shouldHideAgentFromMentions(
      hideArgs({
        candidate: { ownerPubkey: CURRENT_PUBKEY },
        relayAgentPolicies: policy("owner-only"),
      }),
    ),
    false,
  );
  assert.equal(
    shouldHideAgentFromMentions(
      hideArgs({
        candidate: { ownerPubkey: OTHER_OWNER_PUBKEY },
        relayAgentPolicies: policy("owner-only"),
      }),
    ),
    true,
  );
  // Unverified ownership is no proof at all.
  assert.equal(
    shouldHideAgentFromMentions(
      hideArgs({ relayAgentPolicies: policy("owner-only") }),
    ),
    true,
  );
});

test("shouldHideAgentFromMentions: hides external agents without a directory declaration", () => {
  assert.equal(shouldHideAgentFromMentions(hideArgs()), true);
});

test("shouldHideAgentFromMentions: hides non-member external agents regardless of policy", () => {
  // Offering these would need the add-via-mention flow to honor the
  // directory's channel_add_policy — an explicit follow-up.
  assert.equal(
    shouldHideAgentFromMentions(
      hideArgs({
        candidate: { isMember: false },
        relayAgentPolicies: policy("anyone"),
      }),
    ),
    true,
  );
});

test("shouldHideAgentFromMentions: hides agents when the viewer is unknown", () => {
  for (const respondTo of ["allowlist", "owner-only"]) {
    assert.equal(
      shouldHideAgentFromMentions(
        hideArgs({
          candidate: { ownerPubkey: OWNER_PUBKEY },
          currentPubkey: null,
          relayAgentPolicies: policy(respondTo, [CURRENT_PUBKEY]),
        }),
      ),
      true,
    );
  }
});

test("shouldHideAgentFromMentions: normalizes pubkeys before every comparison", () => {
  const mixedCase = "Ab".repeat(32);
  const normalized = mixedCase.toLowerCase();

  assert.equal(
    shouldHideAgentFromMentions(
      hideArgs({
        candidate: {
          ownerPubkey: CURRENT_PUBKEY.toUpperCase(),
          pubkey: mixedCase,
        },
        relayAgentPolicies: new Map([
          [normalized, { respondTo: "owner-only", respondToAllowlist: [] }],
        ]),
      }),
    ),
    false,
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
