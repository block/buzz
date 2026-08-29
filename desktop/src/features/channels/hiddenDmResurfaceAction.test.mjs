import assert from "node:assert/strict";
import test from "node:test";

import { resurfaceHiddenDmMessage } from "./hiddenDmResurfaceAction.ts";

const SELF = "1".repeat(64);
const ALICE = "2".repeat(64);
const BOB = "3".repeat(64);

function event() {
  return {
    id: "event-1",
    kind: 40002,
    pubkey: ALICE,
    content: "hello",
    created_at: 10,
    // Message p tags are intentionally incomplete for this group DM.
    tags: [
      ["h", "hidden-dm"],
      ["p", SELF],
    ],
    sig: "",
  };
}

function member(pubkey) {
  return {
    pubkey,
    role: "member",
    isAgent: false,
    joinedAt: "",
    displayName: null,
  };
}

test("reopens the source hidden group DM from authoritative membership", async () => {
  const inputs = [];
  assert.equal(
    await resurfaceHiddenDmMessage({
      event: event(),
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: SELF,
      hiddenDmIds: new Set(["hidden-dm"]),
      fetchMembers: async () => [member(SELF), member(ALICE), member(BOB)],
      isCurrent: () => true,
      reopen: async (input) => {
        inputs.push(input);
        return { id: "hidden-dm" };
      },
    }),
    true,
  );
  assert.deepEqual(inputs, [
    {
      pubkeys: [ALICE, BOB],
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: SELF,
    },
  ]);
});

test("ignores an event for a channel outside the hidden set", async () => {
  let reopenCount = 0;
  assert.equal(
    await resurfaceHiddenDmMessage({
      event: event(),
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: SELF,
      hiddenDmIds: new Set(["other-dm"]),
      fetchMembers: async () => [member(SELF), member(ALICE)],
      isCurrent: () => true,
      reopen: async () => {
        reopenCount += 1;
        return { id: "hidden-dm" };
      },
    }),
    false,
  );
  assert.equal(reopenCount, 0);
});

test("a suspended old-community read cannot reopen a DM", async () => {
  let current = true;
  let resume;
  const members = new Promise((resolve) => {
    resume = resolve;
  });
  let reopenCount = 0;
  const result = resurfaceHiddenDmMessage({
    event: event(),
    expectedRelayUrl: "wss://old.example",
    expectedSignerPubkey: SELF,
    hiddenDmIds: new Set(["hidden-dm"]),
    fetchMembers: async () => members,
    isCurrent: () => current,
    reopen: async () => {
      reopenCount += 1;
      return { id: "hidden-dm" };
    },
  });
  await Promise.resolve();
  current = false;
  resume([member(SELF), member(ALICE)]);
  assert.equal(await result, false);
  assert.equal(reopenCount, 0);
});

test("rejects a reopen result for any channel other than the source", async () => {
  await assert.rejects(
    resurfaceHiddenDmMessage({
      event: event(),
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: SELF,
      hiddenDmIds: new Set(["hidden-dm"]),
      fetchMembers: async () => [member(SELF), member(ALICE)],
      isCurrent: () => true,
      reopen: async () => ({ id: "alternate-dm" }),
    }),
    /different DM conversation/,
  );
});
