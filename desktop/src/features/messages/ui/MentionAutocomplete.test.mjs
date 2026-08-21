import assert from "node:assert/strict";
import test from "node:test";

import { mentionAgentLabel } from "./MentionAutocomplete.tsx";

function suggestion(agentProvenance) {
  return {
    pubkey: "1".repeat(64),
    displayName: "Carl",
    isAgent: true,
    agentProvenance,
  };
}

test("duplicate owned agents show their management provenance", () => {
  assert.equal(
    mentionAgentLabel(suggestion("managed-here"), true),
    "agent · managed here",
  );
  assert.equal(
    mentionAgentLabel(suggestion("managed-elsewhere"), true),
    "agent · managed elsewhere",
  );
});

test("unique agents keep the compact generic label", () => {
  assert.equal(mentionAgentLabel(suggestion("managed-here"), false), "agent");
});

test("agents without trustworthy provenance keep the generic label", () => {
  assert.equal(mentionAgentLabel(suggestion(undefined), true), "agent");
});
