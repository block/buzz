import assert from "node:assert/strict";
import test from "node:test";

import {
  noteMayMention,
  resolveForwardNoteMentions,
} from "./forwardNoteMentions.ts";

const ALICE =
  "1111111111111111111111111111111111111111111111111111111111111111";
const BOB = "2222222222222222222222222222222222222222222222222222222222222222";
const LI = "3333333333333333333333333333333333333333333333333333333333333333";
const ELODIE =
  "4444444444444444444444444444444444444444444444444444444444444444";
const ROBOT =
  "5555555555555555555555555555555555555555555555555555555555555555";

function member(pubkey, displayName) {
  return {
    pubkey,
    role: "member",
    isAgent: false,
    joinedAt: "2026-01-01T00:00:00Z",
    displayName,
  };
}

const MEMBERS = [member(ALICE, "Alice"), member(BOB.toUpperCase(), "Bob")];
const NON_ASCII_MEMBERS = [
  member(LI, "李"),
  member(ELODIE, "Élodie"),
  member(ROBOT, "🤖bot"),
];

test("resolves a note mention to its destination member", () => {
  const mentions = resolveForwardNoteMentions("hey @Alice look", MEMBERS);
  assert.deepEqual(mentions.pubkeys, [ALICE]);
  assert.deepEqual(mentions.names, ["Alice"]);
  assert.deepEqual(mentions.pubkeysByName, { alice: ALICE });
});

test("member pubkeys are normalized to lowercase", () => {
  const mentions = resolveForwardNoteMentions("@Bob ptal", MEMBERS);
  assert.deepEqual(mentions.pubkeys, [BOB]);
  assert.deepEqual(mentions.pubkeysByName, { bob: BOB });
});

test("matches every mentioned member, case-insensitively", () => {
  const mentions = resolveForwardNoteMentions("@alice and @bob", MEMBERS);
  assert.deepEqual(mentions.pubkeys.sort(), [ALICE, BOB].sort());
  assert.equal(mentions.names.length, 2);
});

test("non-members and non-mentions resolve to nothing", () => {
  assert.deepEqual(
    resolveForwardNoteMentions("@carol are you there", MEMBERS).pubkeys,
    [],
  );
  assert.deepEqual(
    resolveForwardNoteMentions("Alice wrote this", MEMBERS).pubkeys,
    [],
  );
});

test("mentions inside code are not recipients", () => {
  assert.deepEqual(
    resolveForwardNoteMentions("run `@Alice --help`", MEMBERS).pubkeys,
    [],
  );
  assert.deepEqual(
    resolveForwardNoteMentions("```\n@Alice\n```", MEMBERS).pubkeys,
    [],
  );
});

test("empty notes and unloaded members resolve to nothing", () => {
  assert.deepEqual(resolveForwardNoteMentions("", MEMBERS).pubkeys, []);
  assert.deepEqual(resolveForwardNoteMentions("   ", MEMBERS).pubkeys, []);
  assert.deepEqual(resolveForwardNoteMentions("@Alice", undefined).pubkeys, []);
  assert.deepEqual(resolveForwardNoteMentions("@Alice", []).pubkeys, []);
});

test("members without a display name are skipped", () => {
  assert.deepEqual(
    resolveForwardNoteMentions("@Alice", [member(ALICE, null)]).pubkeys,
    [],
  );
});

test("resolves non-ASCII display names", () => {
  assert.deepEqual(
    resolveForwardNoteMentions("看看 @李 谢谢", NON_ASCII_MEMBERS).pubkeys,
    [LI],
  );
  assert.deepEqual(
    resolveForwardNoteMentions("@Élodie ptal", NON_ASCII_MEMBERS).pubkeys,
    [ELODIE],
  );
  const robot = resolveForwardNoteMentions(
    "@🤖bot can you look",
    NON_ASCII_MEMBERS,
  );
  assert.deepEqual(robot.pubkeys, [ROBOT]);
  assert.deepEqual(robot.names, ["🤖bot"]);
  assert.deepEqual(robot.pubkeysByName, { "🤖bot": ROBOT });
});

test("noteMayMention gates sending on a note that could mention someone", () => {
  assert.equal(noteMayMention("hey @Alice look"), true);
  assert.equal(noteMayMention("@a"), true);
  // Over-eager on purpose: a false positive only waits for members to load.
  assert.equal(noteMayMention("mail me at bob@example.com"), true);
});

test("noteMayMention covers non-ASCII mentions", () => {
  // `\w` would miss all three, publishing the forward with no `p` tag.
  for (const note of ["看看 @李 谢谢", "@Élodie ptal", "@🤖bot can you look"]) {
    assert.equal(noteMayMention(note), true);
    assert.equal(
      resolveForwardNoteMentions(note, NON_ASCII_MEMBERS).pubkeys.length,
      1,
    );
  }
});

test("noteMayMention is false when no mention is possible", () => {
  assert.equal(noteMayMention(""), false);
  assert.equal(noteMayMention("ptal"), false);
  assert.equal(noteMayMention("costs @ 5 dollars"), false);
});
