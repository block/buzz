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

// ── Gate composition (mirrors useMentions.addCandidate) ────────────────
//
// `useMentions` runs two gates in order: `isAgentIdentityInManagedList`, then
// `shouldHideAgentFromMentions`. The first was originally passed the
// locally-managed set, which dropped every relay-published (headless/BYO)
// agent before the directory-aware second gate could admit it — so such an
// agent was never mentionable regardless of its kind:10100 entry. The call
// site now passes the invocable set; these tests pin that composition.

function survivesMentionGates({
  candidate,
  managedAgentPubkeys,
  relayAgents,
  sharedChannelIds,
  currentPubkey = CURRENT_PUBKEY,
}) {
  const mentionableAgentPubkeys = getMentionableAgentPubkeys({
    currentPubkey,
    managedAgentPubkeys,
    relayAgents,
    sharedChannelIds,
  });
  const directoryAgentPubkeys = new Set(
    relayAgents.map((agent) => agent.pubkey),
  );

  // Gate 1 — must use the invocable set, not the locally-managed one.
  if (!isAgentIdentityInManagedList(candidate, mentionableAgentPubkeys)) {
    return false;
  }
  // Gate 2 — the directory-aware policy.
  return !shouldHideAgentFromMentions({
    isAgent: candidate.isAgent === true,
    isMember: candidate.isMember === true,
    pubkey: candidate.pubkey,
    mentionableAgentPubkeys,
    directoryAgentPubkeys,
  });
}

test("mention gates: a shared relay agent survives without being locally managed", () => {
  const relayAgents = [
    {
      pubkey: PUB_B,
      channelIds: ["chan-1"],
      respondTo: "anyone",
      respondToAllowlist: [],
    },
  ];

  assert.equal(
    survivesMentionGates({
      candidate: { isAgent: true, isMember: true, pubkey: PUB_B },
      managedAgentPubkeys: new Set(),
      relayAgents,
      sharedChannelIds: new Set(["chan-1"]),
    }),
    true,
    "a relay agent advertising respond_to=anyone in a shared channel must be mentionable",
  );
});

test("mention gates: an allowlisted relay agent survives for the listed user", () => {
  const relayAgents = [
    {
      pubkey: PUB_B,
      channelIds: ["chan-1"],
      respondTo: "allowlist",
      respondToAllowlist: [CURRENT_PUBKEY],
    },
  ];

  assert.equal(
    survivesMentionGates({
      candidate: { isAgent: true, isMember: true, pubkey: PUB_B },
      managedAgentPubkeys: new Set(),
      relayAgents,
      sharedChannelIds: new Set(["chan-1"]),
    }),
    true,
  );
});

test("mention gates: a non-invocable relay agent is still dropped", () => {
  const relayAgents = [
    {
      pubkey: PUB_B,
      channelIds: ["chan-other"],
      respondTo: "anyone",
      respondToAllowlist: [],
    },
  ];

  assert.equal(
    survivesMentionGates({
      candidate: { isAgent: true, isMember: true, pubkey: PUB_B },
      managedAgentPubkeys: new Set(),
      relayAgents,
      sharedChannelIds: new Set(["chan-1"]),
    }),
    false,
    "widening gate 1 must not admit agents that share no channel with us",
  );
});

test("mention gates: locally managed agents keep working", () => {
  assert.equal(
    survivesMentionGates({
      candidate: { isAgent: true, isMember: true, pubkey: PUB_A },
      managedAgentPubkeys: new Set([PUB_A]),
      relayAgents: [],
      sharedChannelIds: new Set(),
    }),
    true,
  );
});

// The composition tests above pin the *policy*, but they call the gates
// directly — they cannot catch the call site in `useMentions` narrowing gate 1
// back to the locally-managed set, which is exactly the regression that made
// every relay-published agent un-mentionable. Guard the call site itself, in
// the spirit of desktop/scripts/check-px-text.mjs.
test("useMentions gates agent identities on the invocable set, not the managed set", async () => {
  const { readFile } = await import("node:fs/promises");
  const source = await readFile(
    new URL("../../messages/lib/useMentions.ts", import.meta.url),
    "utf8",
  );

  const call = source.match(
    /isAgentIdentityInManagedList\(\s*candidate,\s*(\w+)/,
  );
  assert.ok(call, "expected an isAgentIdentityInManagedList call site");
  assert.equal(
    call[1],
    "mentionableAgentPubkeys",
    "gate 1 must receive the invocable set; passing managedAgentPubkeys drops " +
      "every relay-published agent before the directory-aware gate runs",
  );
});
