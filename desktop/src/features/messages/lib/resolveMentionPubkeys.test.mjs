import assert from "node:assert/strict";
import test from "node:test";

import { resolveMentionPubkeys } from "./resolveMentionPubkeys.ts";

const ALICE = "a".repeat(64);
const HERE_MEMBER = "b".repeat(64);

function member(displayName, pubkey, overrides = {}) {
  return { displayName, pubkey, isMember: true, ...overrides };
}

test("selected names resolve to their pubkey", () => {
  const pubkeys = resolveMentionPubkeys(
    "hi @Alice",
    new Map([["Alice", ALICE]]),
    [],
    [],
  );
  assert.deepEqual(pubkeys, [ALICE]);
});

test("members are matched by literal display name without a selection", () => {
  const pubkeys = resolveMentionPubkeys(
    "hi @Alice",
    new Map(),
    [],
    [member("Alice", ALICE)],
  );
  assert.deepEqual(pubkeys, [ALICE]);
});

test("@channel and @here never resolve to a pubkey", () => {
  assert.deepEqual(
    resolveMentionPubkeys(
      "@channel ship it",
      new Map(),
      [],
      [member("channel", ALICE)],
    ),
    [],
  );
  assert.deepEqual(
    resolveMentionPubkeys(
      "@here ship it",
      new Map(),
      [],
      [member("here", HERE_MEMBER)],
    ),
    [],
  );
});

test("a member literally named here loses to the reserved token", () => {
  const pubkeys = resolveMentionPubkeys(
    "@here and @Alice",
    new Map([
      ["here", HERE_MEMBER],
      ["Alice", ALICE],
    ]),
    [],
    [member("here", HERE_MEMBER)],
  );
  assert.deepEqual(pubkeys, [ALICE]);
});

test("non-members and duplicate pubkeys are dropped", () => {
  const pubkeys = resolveMentionPubkeys(
    "@Alice @Bob",
    new Map(),
    [],
    [
      member("Alice", ALICE),
      member("Alice", ALICE),
      member("Bob", "c".repeat(64), { isMember: false }),
    ],
  );
  assert.deepEqual(pubkeys, [ALICE]);
});

test("persona names already selected are not re-matched as members", () => {
  const pubkeys = resolveMentionPubkeys(
    "@Planner",
    new Map(),
    ["Planner"],
    [member("Planner", ALICE)],
  );
  assert.deepEqual(pubkeys, []);
});
