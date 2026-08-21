import assert from "node:assert/strict";
import test from "node:test";

import { resolveMentionProps } from "@/shared/lib/resolveMentionNames";
import { mergeOutgoingTagsWithReferenceMentions } from "./useMentionSendFlow.helpers.ts";

const SELECTED_AGENT_PUBKEY = "a".repeat(64);
const OTHER_AGENT_PUBKEY = "b".repeat(64);

test("preserves selected mention identities as non-notifying references", () => {
  assert.deepEqual(
    mergeOutgoingTagsWithReferenceMentions(
      [["emoji", "buzz", "https://example.com/buzz.png"]],
      [SELECTED_AGENT_PUBKEY, OTHER_AGENT_PUBKEY],
    ),
    [
      ["emoji", "buzz", "https://example.com/buzz.png"],
      ["mention", SELECTED_AGENT_PUBKEY],
      ["mention", OTHER_AGENT_PUBKEY],
    ],
  );
});

test("normalizes and deduplicates existing mention references", () => {
  assert.deepEqual(
    mergeOutgoingTagsWithReferenceMentions(
      [["mention", SELECTED_AGENT_PUBKEY.toUpperCase()]],
      [SELECTED_AGENT_PUBKEY, SELECTED_AGENT_PUBKEY],
    ),
    [["mention", SELECTED_AGENT_PUBKEY.toUpperCase()]],
  );
});

test("a selected agent reference remains resolvable after the message is sent", () => {
  const tags = mergeOutgoingTagsWithReferenceMentions(undefined, [
    SELECTED_AGENT_PUBKEY,
  ]);
  const { mentionNames, mentionPubkeysByName } = resolveMentionProps(tags, {
    [SELECTED_AGENT_PUBKEY]: {
      displayName: "Debug",
      name: "debug-agent",
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: OTHER_AGENT_PUBKEY,
    },
  });

  assert.ok(mentionNames?.includes("Debug"));
  assert.equal(mentionPubkeysByName?.debug, SELECTED_AGENT_PUBKEY);
});
