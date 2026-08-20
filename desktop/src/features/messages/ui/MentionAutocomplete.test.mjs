import assert from "node:assert/strict";
import test from "node:test";

import { showMentionAgentProvenanceMarker } from "./MentionAutocomplete.tsx";

function suggestion(agentProvenance) {
  return {
    pubkey: "1".repeat(64),
    displayName: "Carl",
    isAgent: true,
    agentProvenance,
  };
}

test("duplicate owned agents mark only the other setup", () => {
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion("managed-here"), true),
    false,
  );
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion("managed-elsewhere"), true),
    true,
  );
});

test("unique agents omit management provenance", () => {
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion("managed-here"), false),
    false,
  );
});

test("agents without trustworthy provenance omit management provenance", () => {
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion(undefined), true),
    false,
  );
});
