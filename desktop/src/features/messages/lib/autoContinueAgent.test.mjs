import assert from "node:assert/strict";
import test from "node:test";

import { computeAutoContinueAgentMentions } from "./autoContinueAgent.ts";

const AGENT = "a".repeat(64);
const HUMAN = "b".repeat(64);

const anchor = {
  signerPubkey: AGENT,
  author: AGENT,
  tags: [["p", HUMAN]],
};

test("auto-continues when the agent anchor p-tagged the current user", () => {
  const result = computeAutoContinueAgentMentions({
    anchor,
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [],
  });
  assert.deepEqual(result, [AGENT]);
});

test("no-op when the agent did not p-tag the current user", () => {
  const result = computeAutoContinueAgentMentions({
    anchor: { ...anchor, tags: [] },
    currentPubkey: HUMAN,
    agentPubkeys: new Set([AGENT]),
    existingMentionPubkeys: [],
  });
  assert.deepEqual(result, []);
});
