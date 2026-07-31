import assert from "node:assert/strict";
import test from "node:test";

import { resolveReplyTargetAgent } from "./replyTargetAgentMention.ts";

const agentPubkey = "a".repeat(64);
const humanPubkey = "c".repeat(64);
const emptySet = new Set();

test("resolves agent via the message isAgent flag", () => {
  assert.deepEqual(
    resolveReplyTargetAgent(
      { id: "evt1", pubkey: agentPubkey, author: "Fizz", isAgent: true },
      emptySet,
    ),
    { targetId: "evt1", pubkey: agentPubkey, displayName: "Fizz" },
  );
});

test("resolves agent via the known-agent pubkey set", () => {
  assert.deepEqual(
    resolveReplyTargetAgent(
      { id: "evt1", pubkey: agentPubkey, author: "Fizz" },
      new Set([agentPubkey]),
    ),
    { targetId: "evt1", pubkey: agentPubkey, displayName: "Fizz" },
  );
});

test("resolves agent via the profile isAgent flag", () => {
  assert.deepEqual(
    resolveReplyTargetAgent({ id: "evt1", pubkey: agentPubkey }, emptySet, {
      [agentPubkey]: {
        displayName: "Fizz",
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: null,
        isAgent: true,
      },
    }),
    { targetId: "evt1", pubkey: agentPubkey, displayName: "Fizz" },
  );
});

test("human reply target resolves to null", () => {
  assert.equal(
    resolveReplyTargetAgent(
      { id: "evt1", pubkey: humanPubkey, author: "Fulki" },
      new Set([agentPubkey]),
    ),
    null,
  );
});

test("absent target or pubkey resolves to null", () => {
  assert.equal(resolveReplyTargetAgent(null, emptySet), null);
  assert.equal(resolveReplyTargetAgent(undefined, emptySet), null);
  assert.equal(
    resolveReplyTargetAgent(
      { id: "evt1", author: "Fizz", isAgent: true },
      emptySet,
    ),
    null,
  );
});

test("normalizes uppercase pubkeys before matching", () => {
  assert.deepEqual(
    resolveReplyTargetAgent(
      { id: "evt1", pubkey: agentPubkey.toUpperCase(), author: "Fizz" },
      new Set([agentPubkey]),
    ),
    { targetId: "evt1", pubkey: agentPubkey, displayName: "Fizz" },
  );
});

test("prefers profile display name over the message author label", () => {
  assert.deepEqual(
    resolveReplyTargetAgent(
      {
        id: "evt1",
        pubkey: agentPubkey,
        author: "fizz-fallback",
        isAgent: true,
      },
      emptySet,
      {
        [agentPubkey]: {
          displayName: "Fizz",
          avatarUrl: null,
          nip05Handle: null,
          ownerPubkey: null,
        },
      },
    ),
    { targetId: "evt1", pubkey: agentPubkey, displayName: "Fizz" },
  );
});

test("falls back to the profile name, then null without any name", () => {
  assert.deepEqual(
    resolveReplyTargetAgent(
      { id: "evt1", pubkey: agentPubkey, isAgent: true },
      emptySet,
      {
        [agentPubkey]: {
          displayName: null,
          name: "fizz",
          avatarUrl: null,
          nip05Handle: null,
          ownerPubkey: null,
        },
      },
    ),
    { targetId: "evt1", pubkey: agentPubkey, displayName: "fizz" },
  );
  assert.equal(
    resolveReplyTargetAgent(
      { id: "evt1", pubkey: agentPubkey, isAgent: true },
      emptySet,
    ),
    null,
  );
});
