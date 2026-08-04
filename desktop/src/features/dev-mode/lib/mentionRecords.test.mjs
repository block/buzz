import assert from "node:assert/strict";
import test from "node:test";

import { extractMentions } from "./mentionRecords.ts";

const joah = { name: "Joah", pubkey: "AA11", isAgent: false };
const amp = { name: "amp", pubkey: "bb22", isAgent: true };
const ampLocal = { name: "amp (local)", pubkey: "cc33", isAgent: true };

test("finds a mention at the start of the text", () => {
  const found = extractMentions("@Joah take a look", [joah]);
  assert.deepEqual(
    found.map((record) => record.pubkey),
    ["AA11"],
  );
});

test("finds a mid-sentence mention after whitespace", () => {
  const found = extractMentions("ping @Joah about this", [joah]);
  assert.equal(found.length, 1);
});

test("matching is case-insensitive", () => {
  const found = extractMentions("@joah hello", [joah]);
  assert.equal(found.length, 1);
});

test("trailing punctuation does not break the match", () => {
  const found = extractMentions("@Joah, thoughts?", [joah]);
  assert.equal(found.length, 1);
});

test("an email address is not a mention", () => {
  const found = extractMentions("mail joah@example.com please", [
    joah,
    { name: "example.com", pubkey: "dd44", isAgent: false },
  ]);
  assert.equal(found.length, 0);
});

test("a record whose name is absent from the text is not extracted", () => {
  const found = extractMentions("no mentions here", [joah, amp]);
  assert.equal(found.length, 0);
});

test("longest name wins: @amp (local) never counts as @amp", () => {
  const found = extractMentions("@amp (local) run the tests", [amp, ampLocal]);
  assert.deepEqual(
    found.map((record) => record.pubkey),
    ["cc33"],
  );
});

test("both names extract when both are genuinely mentioned", () => {
  const found = extractMentions("@amp (local) and @amp should sync", [
    amp,
    ampLocal,
  ]);
  assert.deepEqual(found.map((record) => record.pubkey).sort(), [
    "bb22",
    "cc33",
  ]);
});

test("duplicate pubkeys collapse to one record", () => {
  const alias = { name: "Joah G", pubkey: "aa11", isAgent: false };
  const found = extractMentions("@Joah and @Joah G", [joah, alias]);
  assert.equal(found.length, 1);
});

test("a mention inside a word does not match", () => {
  const found = extractMentions("email@Joah is not a mention", [joah]);
  assert.equal(found.length, 0);
});

test("bracketed mentions match", () => {
  const found = extractMentions("(@Joah) and [@amp]", [joah, amp]);
  assert.equal(found.length, 2);
});
