import assert from "node:assert/strict";
import test from "node:test";

import { mentionAgentLabel } from "./MentionAutocomplete.tsx";

function suggestion(agentDevice) {
  return {
    pubkey: "1".repeat(64),
    displayName: "Carl",
    isAgent: true,
    agentDevice,
  };
}

test("duplicate owned agents show their device provenance", () => {
  assert.equal(
    mentionAgentLabel(suggestion("this"), true),
    "agent · this device",
  );
  assert.equal(
    mentionAgentLabel(suggestion("other"), true),
    "agent · other device",
  );
});

test("unique agents keep the compact generic label", () => {
  assert.equal(mentionAgentLabel(suggestion("this"), false), "agent");
});

test("agents without trustworthy provenance keep the generic label", () => {
  assert.equal(mentionAgentLabel(suggestion(undefined), true), "agent");
});
