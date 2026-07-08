import assert from "node:assert/strict";
import test from "node:test";

import { computeAutoContinueAgentMentions } from "./autoContinueAgent.ts";

const AGENT = "a".repeat(64);
const HUMAN = "b".repeat(64);
const OTHER_AGENT = "c".repeat(64);

function agentAnchor(overrides = {}) {
  return {
    signerPubkey: AGENT,
    author: AGENT,
    tags: [
      ["h", "chan-1"],
      ["p", HUMAN],
      ["e", "root-id", "", "root"],
    ],
    ...overrides,
  };
}

test("auto-continues when agent anchor p-tagged the current user", () => {
  const result = computeAutoContinueAgentMentions({
    anchor: agentAnchor(),
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [],
  });
  assert.deepEqual(result, [AGENT]);
});

test("normalizes case on current pubkey and agent set", () => {
  const result = computeAutoContinueAgentMentions({
    anchor: agentAnchor({
      signerPubkey: AGENT.toUpperCase(),
      tags: [["p", HUMAN.toUpperCase()]],
    }),
    currentPubkey: HUMAN.toUpperCase(),
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [],
  });
  assert.deepEqual(result, [AGENT]);
});

test("no-op when anchor author is not a known agent", () => {
  const result = computeAutoContinueAgentMentions({
    anchor: agentAnchor({ signerPubkey: HUMAN, author: HUMAN }),
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [],
  });
  assert.deepEqual(result, []);
});

test("no-op when the agent did not p-tag the current user", () => {
  const result = computeAutoContinueAgentMentions({
    anchor: agentAnchor({ tags: [["p", OTHER_AGENT]] }),
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [],
  });
  assert.deepEqual(result, []);
});

test("no-op when the reply already mentions the agent", () => {
  const result = computeAutoContinueAgentMentions({
    anchor: agentAnchor(),
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [AGENT],
  });
  assert.deepEqual(result, []);
});

test("dedupes case-insensitively against existing mentions", () => {
  const result = computeAutoContinueAgentMentions({
    anchor: agentAnchor(),
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [AGENT.toUpperCase()],
  });
  assert.deepEqual(result, []);
});

test("never auto-mentions ourselves", () => {
  const result = computeAutoContinueAgentMentions({
    anchor: agentAnchor({ signerPubkey: HUMAN, author: HUMAN }),
    currentPubkey: HUMAN,
    // Pathological: current user is in the agent set.
    agentPubkeys: new Set([HUMAN]),
    existingMentionPubkeys: [],
  });
  assert.deepEqual(result, []);
});

test("prefers signerPubkey over display pubkey/author", () => {
  // Display author spoofs a human, but the real signer is the agent.
  const result = computeAutoContinueAgentMentions({
    anchor: agentAnchor({ signerPubkey: AGENT, pubkey: HUMAN, author: HUMAN }),
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [],
  });
  assert.deepEqual(result, [AGENT]);
});

test("no-op on missing anchor, pubkey, or empty agent set", () => {
  const base = {
    anchor: agentAnchor(),
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [],
  };
  assert.deepEqual(
    computeAutoContinueAgentMentions({ ...base, anchor: null }),
    [],
  );
  assert.deepEqual(
    computeAutoContinueAgentMentions({ ...base, currentPubkey: null }),
    [],
  );
  assert.deepEqual(
    computeAutoContinueAgentMentions({ ...base, agentPubkeys: new Set() }),
    [],
  );
});

test("no-op when anchor has no tags", () => {
  const result = computeAutoContinueAgentMentions({
    anchor: agentAnchor({ tags: undefined }),
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [],
  });
  assert.deepEqual(result, []);
});
