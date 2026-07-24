import assert from "node:assert/strict";
import test from "node:test";

import { diffAddedMentionPubkeys } from "./threading.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const SELF = "c".repeat(64);

test("returns mentions the edit newly adds", () => {
  // Original mentioned Alice; edit adds Bob.
  assert.deepEqual(diffAddedMentionPubkeys([ALICE], [ALICE, BOB], SELF), [BOB]);
});

test("typo-fix edit with unchanged mentions re-wakes nobody", () => {
  assert.deepEqual(diffAddedMentionPubkeys([ALICE], [ALICE], SELF), []);
});

test("adding the first mention to a previously unmentioned body", () => {
  assert.deepEqual(diffAddedMentionPubkeys([], [ALICE], SELF), [ALICE]);
});

test("removing a mention adds nothing", () => {
  assert.deepEqual(diffAddedMentionPubkeys([ALICE, BOB], [ALICE], SELF), []);
});

test("case-only difference is not treated as newly added", () => {
  // Original stored uppercase, edit resolves lowercase (or vice versa).
  assert.deepEqual(
    diffAddedMentionPubkeys([ALICE.toUpperCase()], [ALICE], SELF),
    [],
  );
});

test("self-mention added in the edit is scrubbed, never notified", () => {
  assert.deepEqual(diffAddedMentionPubkeys([ALICE], [ALICE, SELF], SELF), []);
});

test("duplicate added mention collapses to one", () => {
  assert.deepEqual(diffAddedMentionPubkeys([], [BOB, BOB, BOB], SELF), [BOB]);
});

test("re-adding a removed mention counts as newly added", () => {
  // Original had no Bob (he was removed in a prior state); this edit adds him.
  assert.deepEqual(diffAddedMentionPubkeys([ALICE], [ALICE, BOB], SELF), [BOB]);
});
