import assert from "node:assert/strict";
import test from "node:test";

import {
  dmPeerPubkeysFromMembers,
  isIncomingDmMessageFeedItem,
  isIncomingDmMessageRelayEvent,
  markHiddenDmFeedItems,
} from "./dmResurface.ts";

const SELF = "1".repeat(64);
const ALICE = "2".repeat(64);
const BOB = "3".repeat(64);

function item(overrides = {}) {
  return {
    id: "event-1",
    kind: 9,
    pubkey: ALICE,
    content: "hello",
    createdAt: 10,
    channelId: "dm-1",
    channelName: "",
    tags: [
      ["h", "dm-1"],
      ["p", SELF],
      ["p", BOB],
    ],
    category: "mention",
    ...overrides,
  };
}

test("DM resurface derives peers from authoritative membership", () => {
  const members = [{ pubkey: SELF }, { pubkey: ALICE }, { pubkey: BOB }];
  assert.deepEqual(dmPeerPubkeysFromMembers(members, SELF), [ALICE, BOB]);
  assert.deepEqual(dmPeerPubkeysFromMembers([{ pubkey: ALICE }], SELF), []);
});

test("only external addressed human messages qualify", () => {
  assert.equal(isIncomingDmMessageFeedItem(item(), SELF), true);
  assert.equal(isIncomingDmMessageFeedItem(item({ kind: 7 }), SELF), false);
  assert.equal(
    isIncomingDmMessageFeedItem(item({ pubkey: SELF }), SELF),
    false,
  );
  assert.equal(
    isIncomingDmMessageFeedItem(item({ tags: [["h", "dm-1"]] }), SELF),
    false,
  );
});

test("hidden feed items are projected as DMs for Inbox presentation", () => {
  const feed = {
    feed: {
      mentions: [item()],
      needsAction: [],
      activity: [],
      agentActivity: [],
    },
    meta: { since: 0, total: 1, generatedAt: 10 },
  };
  const marked = markHiddenDmFeedItems(feed, new Set(["dm-1"]));
  assert.equal(marked.feed.mentions[0].channelType, "dm");
});

test("relay events use the same eligibility contract", () => {
  const relayEvent = {
    id: "event-1",
    kind: 40002,
    pubkey: ALICE,
    content: "hello",
    created_at: 10,
    tags: [
      ["h", "dm-1"],
      ["p", SELF],
      ["p", BOB],
    ],
    sig: "",
  };
  assert.equal(isIncomingDmMessageRelayEvent(relayEvent, SELF), true);
});
