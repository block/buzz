import assert from "node:assert/strict";
import test from "node:test";

import { rankMentionCandidates } from "./mentionRanking.ts";

const CHANNEL_BRAIN_PUBKEY = "1".repeat(64);
const OTHER_BRAIN_PUBKEY = "2".repeat(64);

function candidate(overrides = {}) {
  return {
    kind: "identity",
    displayName: "Brain",
    isAgent: false,
    isMember: false,
    pubkey: OTHER_BRAIN_PUBKEY,
    ...overrides,
  };
}

function rankedPubkeys(
  candidates,
  query = "brain",
  activePersonaIds = new Set(),
) {
  return rankMentionCandidates(candidates, query, activePersonaIds).map(
    (item) => item.candidate.pubkey ?? `persona:${item.candidate.personaId}`,
  );
}

test("rankMentionCandidates: channel members outrank runnable personas, people, and other agents", () => {
  const persona = candidate({
    kind: "persona",
    personaId: "brain-persona",
    pubkey: undefined,
  });
  const remoteAgent = candidate({
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });
  const person = candidate({
    pubkey: "6".repeat(64),
  });
  const channelMember = candidate({
    isAgent: true,
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });

  assert.deepEqual(
    rankedPubkeys([persona, remoteAgent, person, channelMember]),
    [
      CHANNEL_BRAIN_PUBKEY,
      "persona:brain-persona",
      "6".repeat(64),
      OTHER_BRAIN_PUBKEY,
    ],
  );
});

test("rankMentionCandidates: exact and prefix quality sort within the channel-member group", () => {
  const wordPrefixMember = candidate({
    displayName: "The Brain",
    isMember: true,
    pubkey: "3".repeat(64),
  });
  const exactMember = candidate({
    displayName: "Brain",
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });
  const prefixMember = candidate({
    displayName: "Brainiac",
    isMember: true,
    pubkey: "4".repeat(64),
  });

  assert.deepEqual(
    rankedPubkeys([wordPrefixMember, exactMember, prefixMember]),
    [CHANNEL_BRAIN_PUBKEY, "4".repeat(64), "3".repeat(64)],
  );
});

test("rankMentionCandidates: matching secondary labels participate in ranking", () => {
  const memberByHandle = candidate({
    displayName: "Acme Bot",
    secondaryLabel: "brain@example.com",
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });
  const nonMemberName = candidate({
    displayName: "Brain",
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.deepEqual(rankedPubkeys([nonMemberName, memberByHandle]), [
    CHANNEL_BRAIN_PUBKEY,
    OTHER_BRAIN_PUBKEY,
  ]);
});

test("rankMentionCandidates: active persona-backed non-members outrank other non-member agents", () => {
  const activePersonaAgent = candidate({
    displayName: "Brain",
    isAgent: true,
    personaId: "brain-persona",
    pubkey: "5".repeat(64),
  });
  const remoteAgent = candidate({
    displayName: "Brain",
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.deepEqual(
    rankedPubkeys(
      [remoteAgent, activePersonaAgent],
      "brain",
      new Set(["brain-persona"]),
    ),
    ["5".repeat(64), OTHER_BRAIN_PUBKEY],
  );
});
