import assert from "node:assert/strict";
import test from "node:test";

import { openHiddenDmInboxContext } from "./hiddenDmInboxAction.ts";

const SELF = "1".repeat(64);
const ALICE = "2".repeat(64);
const BOB = "3".repeat(64);
const inboxItem = {
  id: "event-1",
  item: {
    channelType: "dm",
    // Incomplete message tags must not choose the recreated membership.
    tags: [
      ["h", "hidden-dm"],
      ["p", SELF],
    ],
  },
};

function member(pubkey) {
  return {
    pubkey,
    role: "member",
    isAgent: false,
    joinedAt: "",
    displayName: null,
  };
}

function options(overrides = {}) {
  return {
    item: inboxItem,
    channelId: "hidden-dm",
    messageId: "event-1",
    availableChannelIds: new Set(),
    expectedRelayUrl: "wss://relay.example",
    expectedSignerPubkey: SELF,
    pendingChannelIds: new Set(),
    fetchMembers: async () => [member(SELF), member(ALICE), member(BOB)],
    openDm: async () => ({ id: "hidden-dm" }),
    isCurrent: () => true,
    onOpenContext: () => {},
    onError: () => {},
    onPendingChange: () => {},
    ...overrides,
  };
}

test("Inbox reopens the original hidden group DM from channel membership", async () => {
  const inputs = [];
  const navigations = [];
  const result = await openHiddenDmInboxContext(
    options({
      openDm: async (input) => {
        inputs.push(input);
        return { id: "hidden-dm" };
      },
      onOpenContext: (...args) => navigations.push(args),
    }),
  );
  assert.equal(result, true);
  assert.deepEqual(inputs[0].pubkeys, [ALICE, BOB]);
  assert.deepEqual(navigations, [["hidden-dm", "event-1", undefined]]);
});

test("double activation is deduplicated while a reopen is pending", async () => {
  let resume;
  const members = new Promise((resolve) => {
    resume = resolve;
  });
  let openCount = 0;
  const shared = options({
    fetchMembers: async () => members,
    openDm: async () => {
      openCount += 1;
      return { id: "hidden-dm" };
    },
  });
  const first = openHiddenDmInboxContext(shared);
  const second = openHiddenDmInboxContext(shared);
  resume([member(SELF), member(ALICE)]);
  assert.equal(await second, false);
  assert.equal(await first, true);
  assert.equal(openCount, 1);
});

test("a failed reopen stays put, reports an error, and can be retried", async () => {
  let attempts = 0;
  let errors = 0;
  let navigations = 0;
  const shared = options({
    openDm: async () => {
      attempts += 1;
      if (attempts === 1) throw new Error("offline");
      return { id: "hidden-dm" };
    },
    onError: () => {
      errors += 1;
    },
    onOpenContext: () => {
      navigations += 1;
    },
  });
  assert.equal(await openHiddenDmInboxContext(shared), false);
  assert.equal(navigations, 0);
  assert.equal(errors, 1);
  assert.equal(shared.pendingChannelIds.size, 0);

  assert.equal(await openHiddenDmInboxContext(shared), true);
  assert.equal(attempts, 2);
  assert.equal(navigations, 1);
});

test("an unmounted Inbox action cannot navigate after reopen settles", async () => {
  let current = true;
  let resume;
  const reopened = new Promise((resolve) => {
    resume = resolve;
  });
  let navigations = 0;
  let pendingChanges = 0;
  const result = openHiddenDmInboxContext(
    options({
      openDm: async () => reopened,
      isCurrent: () => current,
      onOpenContext: () => {
        navigations += 1;
      },
      onPendingChange: () => {
        pendingChanges += 1;
      },
    }),
  );
  await Promise.resolve();
  current = false;
  resume({ id: "hidden-dm" });
  assert.equal(await result, false);
  assert.equal(navigations, 0);
  assert.equal(pendingChanges, 1);
});
