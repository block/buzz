import assert from "node:assert/strict";
import test from "node:test";

import {
  appendMentionReferenceTags,
  withMentionReferenceTags,
} from "./mentionReferenceTags.ts";

const PUBKEY_A = "A".repeat(64);
const PUBKEY_B = "b".repeat(64);

test("appendMentionReferenceTags appends normalized explicit mention tags", () => {
  const tags = [["h", "channel-1"]];

  appendMentionReferenceTags(tags, [PUBKEY_A, PUBKEY_B]);

  assert.deepEqual(tags, [
    ["h", "channel-1"],
    ["mention", PUBKEY_A.toLowerCase()],
    ["mention", PUBKEY_B],
  ]);
});

test("appendMentionReferenceTags dedupes existing mention reference tags", () => {
  const tags = [["mention", PUBKEY_A.toLowerCase()]];

  appendMentionReferenceTags(tags, [
    PUBKEY_A,
    PUBKEY_B,
    PUBKEY_B.toUpperCase(),
  ]);

  assert.deepEqual(tags, [
    ["mention", PUBKEY_A.toLowerCase()],
    ["mention", PUBKEY_B],
  ]);
});

test("withMentionReferenceTags does not mutate the input tag array", () => {
  const tags = [["p", PUBKEY_A.toLowerCase()]];

  const next = withMentionReferenceTags(tags, [PUBKEY_A]);

  assert.deepEqual(tags, [["p", PUBKEY_A.toLowerCase()]]);
  assert.deepEqual(next, [
    ["p", PUBKEY_A.toLowerCase()],
    ["mention", PUBKEY_A.toLowerCase()],
  ]);
});
