import assert from "node:assert/strict";
import test from "node:test";

import { mapMentionCandidateToSuggestion } from "./mentionSuggestionMapping.ts";

const OWNER = "a".repeat(64);

function candidate(overrides = {}) {
  return {
    kind: "identity",
    pubkey: "b".repeat(64),
    isAgent: true,
    isMember: true,
    ownerPubkey: OWNER,
    ...overrides,
  };
}

function suggestion(overrides = {}) {
  return mapMentionCandidateToSuggestion({
    candidate: candidate(overrides),
    currentPubkey: OWNER,
    label: "Carl",
  });
}

test("labels Desktop-managed agent identities as managed here", () => {
  assert.equal(
    suggestion({ isManagedAgent: true }).agentProvenance,
    "managed-here",
  );
});

test("labels same-owner relay agent identities as managed elsewhere", () => {
  assert.equal(suggestion().agentProvenance, "managed-elsewhere");
});

test("does not attribute another owner's agent to a device", () => {
  assert.equal(
    suggestion({ ownerPubkey: "c".repeat(64) }).agentProvenance,
    undefined,
  );
});

test("does not attribute people or personas to a device", () => {
  assert.equal(suggestion({ isAgent: false }).agentProvenance, undefined);
  assert.equal(
    suggestion({ kind: "persona", pubkey: undefined }).agentProvenance,
    undefined,
  );
});
