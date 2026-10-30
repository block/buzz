import assert from "node:assert/strict";
import test from "node:test";

import {
  extractMentionPubkeys,
  extractMentionRanges,
} from "./extractMentionPubkeys.ts";

const options = (text, overrides = {}) => ({
  text,
  selectedMentions: new Map(),
  memberCandidates: [
    { displayName: "Alice", isMember: true, pubkey: "a" },
    { displayName: "Alice Chen", isMember: true, pubkey: "ac" },
    { displayName: "Bob", isMember: true, pubkey: "b" },
  ],
  ...overrides,
});

test("ranges are in text order and the longest label owns the offset", () => {
  const text = "@Bob @Alice Chen hi @Alice";
  assert.deepEqual(extractMentionRanges(options(text)), [
    { pubkey: "b", start: 0, end: 4 },
    { pubkey: "ac", start: 5, end: 16 },
    { pubkey: "a", start: 20, end: 26 },
  ]);
  assert.deepEqual(extractMentionPubkeys(options(text)).sort(), [
    "a",
    "ac",
    "b",
  ]);
});

test("ranges ignore names that resolve to no key", () => {
  assert.deepEqual(extractMentionRanges(options("@Ghost hi")), []);
});

test("an ambiguous label throws for ranges as it does for pubkeys", () => {
  const ambiguous = options("@Scout hi", {
    memberCandidates: [
      { displayName: "Scout", isMember: true, pubkey: "s1" },
      { displayName: "Scout", isMember: true, pubkey: "s2" },
    ],
  });
  assert.throws(() => extractMentionRanges(ambiguous), /ambiguous/);
  assert.throws(() => extractMentionPubkeys(ambiguous), /ambiguous/);
});
