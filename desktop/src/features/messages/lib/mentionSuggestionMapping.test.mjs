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

test("labels locally managed agent identities as this device", () => {
  assert.equal(suggestion({ isManagedAgent: true }).agentDevice, "this");
});

test("labels same-owner relay agent identities as another device", () => {
  assert.equal(suggestion().agentDevice, "other");
});

test("does not attribute another owner's agent to a device", () => {
  assert.equal(
    suggestion({ ownerPubkey: "c".repeat(64) }).agentDevice,
    undefined,
  );
});

test("does not attribute people or personas to a device", () => {
  assert.equal(suggestion({ isAgent: false }).agentDevice, undefined);
  assert.equal(
    suggestion({ kind: "persona", pubkey: undefined }).agentDevice,
    undefined,
  );
});
