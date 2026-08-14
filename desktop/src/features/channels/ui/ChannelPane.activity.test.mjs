import assert from "node:assert/strict";
import test from "node:test";

import {
  completeActivityAgentRoster,
  composeThreadActivityPubkeys,
} from "./threadComposerActivity.ts";

test("observer-only channel activity reaches the open thread composer", () => {
  const observerAgent = "ABCDEF";

  const channelComposerPubkeys = [observerAgent];
  const matchingThreadTypingPubkeys = [];
  const threadComposerPubkeys = composeThreadActivityPubkeys(
    channelComposerPubkeys,
    matchingThreadTypingPubkeys,
  );

  assert.deepEqual(threadComposerPubkeys, [observerAgent]);
  assert.deepEqual(threadComposerPubkeys, channelComposerPubkeys);
});

test("thread activity unions matching typing without case-insensitive duplicates", () => {
  assert.deepEqual(
    composeThreadActivityPubkeys(["ABCDEF"], ["abcdef", "123456"]),
    ["ABCDEF", "123456"],
  );
});

test("externally owned working profiles complete the Activity display roster", () => {
  assert.deepEqual(
    completeActivityAgentRoster(
      [{ pubkey: "LOCAL", name: "Local agent" }],
      ["EXTERNAL", "local"],
      {
        external: { displayName: "Habeler" },
        local: { displayName: "Ignored duplicate" },
      },
    ),
    [
      { pubkey: "LOCAL", name: "Local agent" },
      { pubkey: "EXTERNAL", name: "Habeler" },
    ],
  );
});
