import assert from "node:assert/strict";
import test from "node:test";

import { rewriteMentionsToNip27 } from "./rewriteMentionsToNip27.ts";

// Valid hex pubkeys (32 bytes) for npubEncode.
const ALICE =
  "3bf0c63fcb93463407afdbb8b86e4e2c2e4e2c2e4e2c2e4e2c2e4e2c2e4e2c2e";
const BOB =
  "a1b2c3d4e5f60718293a4b5c6d7e8f901234567890abcdef1234567890abcdef";

test("rewriteMentionsToNip27: replaces @name with nostr:npub", () => {
  const out = rewriteMentionsToNip27("hey @Alice", [["Alice", ALICE]]);
  assert.match(out, /^hey nostr:npub1[02-9ac-hj-np-z]+$/);
  assert.equal(out.includes("@Alice"), false);
});

test("rewriteMentionsToNip27: leaves unmatched @names alone", () => {
  const out = rewriteMentionsToNip27("hey @Alice", []);
  assert.equal(out, "hey @Alice");
});

test("rewriteMentionsToNip27: skips code spans", () => {
  const out = rewriteMentionsToNip27("run `@Alice` then @Alice", [
    ["Alice", ALICE],
  ]);
  assert.match(out, /run `@Alice` then nostr:npub1/);
});

test("rewriteMentionsToNip27: longer names first", () => {
  const out = rewriteMentionsToNip27("hi @John Doe", [
    ["John", BOB],
    ["John Doe", ALICE],
  ]);
  assert.match(out, /^hi nostr:npub1/);
  assert.equal(out.includes("@John"), false);
});

test("rewriteMentionsToNip27: team expansion rewrites members", () => {
  const out = rewriteMentionsToNip27("Launch Team(@Planner @Builder)", [
    ["Planner", ALICE],
    ["Builder", BOB],
  ]);
  assert.equal(out.includes("@Planner"), false);
  assert.equal(out.includes("@Builder"), false);
  assert.equal((out.match(/nostr:npub1/g) ?? []).length, 2);
});
