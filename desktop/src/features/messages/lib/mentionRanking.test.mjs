import assert from "node:assert/strict";
import test from "node:test";

import {
  pickDefaultAgentCandidate,
  rankMentionCandidates,
} from "./mentionRanking.ts";

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

test("rankMentionCandidates: owned teams rank with runnable personas", () => {
  const remoteAgent = candidate({
    displayName: "Launch Agent",
    isAgent: true,
  });
  const team = candidate({
    kind: "team",
    displayName: "Launch Team",
    pubkey: undefined,
  });

  assert.deepEqual(
    rankMentionCandidates([remoteAgent, team], "launch").map(
      (item) => item.candidate.kind,
    ),
    ["team", "identity"],
  );
});

test("pickDefaultAgentCandidate: active agents outrank stopped channel members", () => {
  const stoppedMember = candidate({
    displayName: "Ada",
    isActiveAgent: false,
    isAgent: true,
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });
  const runningNonMember = candidate({
    displayName: "Bea",
    isActiveAgent: true,
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.equal(
    pickDefaultAgentCandidate([stoppedMember, runningNonMember]),
    runningNonMember,
  );
});

test("pickDefaultAgentCandidate: stable labels break ties instead of roster order", () => {
  const vogue = candidate({
    displayName: "Vogue",
    isActiveAgent: true,
    isAgent: true,
    isMember: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });
  const morgarita = candidate({
    displayName: "Morgarita",
    isActiveAgent: true,
    isAgent: true,
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });

  assert.equal(pickDefaultAgentCandidate([vogue, morgarita]), morgarita);
  assert.equal(pickDefaultAgentCandidate([morgarita, vogue]), morgarita);
});

test("pickDefaultAgentCandidate: runnable personas break otherwise equal ties", () => {
  const plain = candidate({
    displayName: "Zulu",
    isActiveAgent: true,
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });
  const runnable = candidate({
    displayName: "Zulu 2",
    isActiveAgent: true,
    isAgent: true,
    personaId: "active-persona",
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });

  assert.equal(
    pickDefaultAgentCandidate([plain, runnable], new Set(["active-persona"])),
    runnable,
  );
});

test("pickDefaultAgentCandidate: recent eligible mentions outrank the fallback ranking", () => {
  const stoppedRecentMember = candidate({
    displayName: "Ada",
    isActiveAgent: false,
    isAgent: true,
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });
  const runningNonMember = candidate({
    displayName: "Bea",
    isActiveAgent: true,
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.equal(
    pickDefaultAgentCandidate(
      [runningNonMember, stoppedRecentMember],
      new Set(),
      [CHANNEL_BRAIN_PUBKEY],
    ),
    stoppedRecentMember,
  );
});

test("pickDefaultAgentCandidate: skips recent pubkeys that are not eligible candidates", () => {
  const runningAgent = candidate({
    isActiveAgent: true,
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.equal(
    pickDefaultAgentCandidate([runningAgent], new Set(), ["f".repeat(64)]),
    runningAgent,
  );
});

test("pickDefaultAgentCandidate: returns null without an addressable agent", () => {
  assert.equal(pickDefaultAgentCandidate([]), null);
  assert.equal(pickDefaultAgentCandidate([candidate()]), null);
});
