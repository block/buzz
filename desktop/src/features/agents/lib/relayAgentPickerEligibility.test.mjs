import assert from "node:assert/strict";
import test from "node:test";

import { getRelayAgentPickerCandidates } from "./relayAgentPickerEligibility.ts";

const OWNER = "a".repeat(64);
const OTHER_OWNER = "b".repeat(64);
const OWNED = "1".repeat(64);
const FOREIGN = "2".repeat(64);
const MANAGED = "3".repeat(64);
const MEMBER = "4".repeat(64);

function agent(pubkey, name) {
  return {
    pubkey,
    name,
    agentType: "hermes",
    channels: [],
    channelIds: [],
    capabilities: ["messages", "channels", "tools"],
    status: "online",
    respondTo: "anyone",
    respondToAllowlist: [],
  };
}

function profile(ownerPubkey, displayName = null, avatarUrl = null) {
  return {
    displayName,
    avatarUrl,
    nip05Handle: null,
    ownerPubkey,
    isAgent: true,
  };
}

test("includes owned relay-native agents with canonical profile metadata", () => {
  const result = getRelayAgentPickerCandidates({
    currentPubkey: OWNER,
    managedAgentPubkeys: [],
    memberPubkeys: [],
    profiles: {
      [OWNED]: profile(OWNER, "Sigma", "https://relay.example/sigma.png"),
    },
    relayAgents: [agent(OWNED, "fallback")],
  });

  assert.deepEqual(result, [
    {
      agent: agent(OWNED, "fallback"),
      avatarUrl: "https://relay.example/sigma.png",
      displayName: "Sigma",
      pubkey: OWNED,
    },
  ]);
});

test("excludes foreign, locally managed, and existing channel agents", () => {
  const result = getRelayAgentPickerCandidates({
    currentPubkey: OWNER,
    managedAgentPubkeys: [MANAGED.toUpperCase()],
    memberPubkeys: [MEMBER.toUpperCase()],
    profiles: {
      [FOREIGN]: profile(OTHER_OWNER),
      [MANAGED]: profile(OWNER),
      [MEMBER]: profile(OWNER),
    },
    relayAgents: [
      agent(FOREIGN, "foreign"),
      agent(MANAGED, "managed"),
      agent(MEMBER, "member"),
    ],
  });

  assert.deepEqual(result, []);
});

test("normalizes and deduplicates pubkeys and falls back to directory name", () => {
  const result = getRelayAgentPickerCandidates({
    currentPubkey: OWNER.toUpperCase(),
    managedAgentPubkeys: [],
    memberPubkeys: [],
    profiles: {
      [OWNED]: profile(OWNER.toUpperCase(), "   ", null),
    },
    relayAgents: [
      agent(OWNED.toUpperCase(), "Winston"),
      agent(OWNED, "duplicate"),
    ],
  });

  assert.equal(result.length, 1);
  assert.equal(result[0].displayName, "Winston");
  assert.equal(result[0].pubkey, OWNED);
});

test("returns no candidates until identity and ownership profiles resolve", () => {
  assert.deepEqual(
    getRelayAgentPickerCandidates({
      currentPubkey: null,
      managedAgentPubkeys: [],
      memberPubkeys: [],
      profiles: {},
      relayAgents: [agent(OWNED, "Sigma")],
    }),
    [],
  );
  assert.deepEqual(
    getRelayAgentPickerCandidates({
      currentPubkey: OWNER,
      managedAgentPubkeys: [],
      memberPubkeys: [],
      profiles: {},
      relayAgents: [agent(OWNED, "Sigma")],
    }),
    [],
  );
});
