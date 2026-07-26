import assert from "node:assert/strict";
import test from "node:test";

import { resolveOutgoingMentionPubkeys } from "./outgoingMentionResolution.ts";

function candidate(displayName, pubkey) {
  return { displayName, isMember: true, pubkey };
}

test("one longer mention emits only the longer identity pubkey", () => {
  assert.deepEqual(
    resolveOutgoingMentionPubkeys({
      candidates: [
        candidate("OriginalName", "short-pubkey"),
        candidate("OriginalName copy", "long-pubkey"),
      ],
      selectedMentions: new Map([["OriginalName copy", "long-pubkey"]]),
      selectedPersonaMentions: new Map(),
      text: "@OriginalName copy please respond",
    }),
    ["long-pubkey"],
  );
});

test("separate short and long mention spans emit both pubkeys", () => {
  assert.deepEqual(
    resolveOutgoingMentionPubkeys({
      candidates: [
        candidate("OriginalName", "short-pubkey"),
        candidate("OriginalName copy", "long-pubkey"),
      ],
      selectedMentions: new Map([
        ["OriginalName", "short-pubkey"],
        ["OriginalName copy", "long-pubkey"],
      ]),
      selectedPersonaMentions: new Map(),
      text: "@OriginalName then @OriginalName copy",
    }),
    ["short-pubkey", "long-pubkey"],
  );
});

test("a selected short mention does not override a later independent longer span", () => {
  assert.deepEqual(
    resolveOutgoingMentionPubkeys({
      candidates: [
        candidate("OriginalName", "short-pubkey"),
        candidate("OriginalName copy", "long-pubkey"),
      ],
      selectedMentions: new Map([["OriginalName", "short-pubkey"]]),
      selectedPersonaMentions: new Map(),
      text: "@OriginalName then @OriginalName copy",
    }),
    ["short-pubkey", "long-pubkey"],
  );
});

test("pasted shared-prefix mentions resolve to the longest member name", () => {
  assert.deepEqual(
    resolveOutgoingMentionPubkeys({
      candidates: [
        candidate("Agent", "agent-pubkey"),
        candidate("Agent.copy", "copy-pubkey"),
      ],
      selectedMentions: new Map(),
      selectedPersonaMentions: new Map(),
      text: "@Agent.copy please respond",
    }),
    ["copy-pubkey"],
  );
});
