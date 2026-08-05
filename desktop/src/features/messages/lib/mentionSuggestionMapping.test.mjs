import assert from "node:assert/strict";
import test from "node:test";

import { mapMentionCandidateToSuggestion } from "./mentionSuggestionMapping.ts";

test("external bot-role channel members are not labeled out of channel", () => {
  const suggestion = mapMentionCandidateToSuggestion({
    candidate: {
      kind: "identity",
      pubkey: "a".repeat(64),
      isAgent: true,
      isMember: true,
      role: "bot",
    },
    label: "Copper",
    channelType: "stream",
  });

  assert.equal(suggestion.displayName, "Copper");
  assert.equal(suggestion.isAgent, true);
  assert.equal(suggestion.notInChannel, false);
});
