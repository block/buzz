import assert from "node:assert/strict";
import test from "node:test";

import { hasMentionForEvent, shouldNotifyForEvent } from "./shouldNotify.ts";

const PUBKEY = "a".repeat(64);
const OTHER_PUBKEY = "b".repeat(64);
const CHANNEL_ID =
  "channel-0000000000000000000000000000000000000000000000000000";
const ROOT_ID = `root-${"0".repeat(59)}`;
const PARENT_ID = `parent-${"0".repeat(57)}`;

const EMPTY = new Set();

function makeEvent(tags = [], overrides = {}) {
  return {
    id: `event-${"0".repeat(59)}`,
    pubkey: OTHER_PUBKEY,
    created_at: 1700000000,
    kind: 9,
    tags,
    content: "hello",
    sig: "s".repeat(128),
    ...overrides,
  };
}

const rootTag = (id) => ["e", id, "", "root"];
const replyTag = (id) => ["e", id, "", "reply"];
const pTag = (pubkey) => ["p", pubkey];
const broadcastTag = () => ["broadcast", "1"];
const hTag = (channelId) => ["h", channelId];

test("hasMentionForEvent: p-tag matching currentPubkey returns true", () => {
  const event = makeEvent([pTag(PUBKEY)]);
  assert.equal(hasMentionForEvent(event, PUBKEY), true);
});

test("hasMentionForEvent: p-tag case-insensitive match returns true", () => {
  const event = makeEvent([pTag(PUBKEY.toUpperCase())]);
  assert.equal(hasMentionForEvent(event, PUBKEY), true);
});

test("hasMentionForEvent: p-tag not matching currentPubkey returns false", () => {
  const event = makeEvent([pTag(OTHER_PUBKEY)]);
  assert.equal(hasMentionForEvent(event, PUBKEY), false);
});

test("hasMentionForEvent: no p-tags returns false", () => {
  const event = makeEvent([hTag(CHANNEL_ID)]);
  assert.equal(hasMentionForEvent(event, PUBKEY), false);
});

test("hasMentionForEvent: empty currentPubkey returns false", () => {
  const event = makeEvent([pTag(PUBKEY)]);
  assert.equal(hasMentionForEvent(event, ""), false);
});

test("top-level message in muted channel is suppressed", () => {
  const event = makeEvent([hTag(CHANNEL_ID)]);
  assert.equal(
    shouldNotifyForEvent(event, PUBKEY, {
      participatedRootIds: EMPTY,
      followedRootIds: EMPTY,
      authoredRootIds: EMPTY,
      mutedChannelIds: new Set([CHANNEL_ID]),
      channelId: CHANNEL_ID,
    }),
    false,
  );
});

test("mention in muted channel still notifies (mention fires before mute check)", () => {
  const event = makeEvent([hTag(CHANNEL_ID), pTag(PUBKEY)]);
  assert.equal(
    shouldNotifyForEvent(event, PUBKEY, {
      participatedRootIds: EMPTY,
      followedRootIds: EMPTY,
      authoredRootIds: EMPTY,
      mutedChannelIds: new Set([CHANNEL_ID]),
      channelId: CHANNEL_ID,
    }),
    true,
  );
});

test("thread reply in muted channel is suppressed", () => {
  const event = makeEvent([
    hTag(CHANNEL_ID),
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
  ]);
  assert.equal(
    shouldNotifyForEvent(event, PUBKEY, {
      participatedRootIds: new Set([ROOT_ID]),
      followedRootIds: EMPTY,
      authoredRootIds: EMPTY,
      mutedChannelIds: new Set([CHANNEL_ID]),
      channelId: CHANNEL_ID,
    }),
    false,
  );
});

test("broadcast reply in muted channel still notifies (broadcast fires before mute check)", () => {
  const event = makeEvent([
    hTag(CHANNEL_ID),
    replyTag(ROOT_ID),
    broadcastTag(),
  ]);
  assert.equal(
    shouldNotifyForEvent(event, PUBKEY, {
      participatedRootIds: EMPTY,
      followedRootIds: EMPTY,
      authoredRootIds: EMPTY,
      mutedChannelIds: new Set([CHANNEL_ID]),
      channelId: CHANNEL_ID,
    }),
    true,
  );
});

test("top-level message in unmuted channel notifies", () => {
  const event = makeEvent([hTag(CHANNEL_ID)]);
  assert.equal(
    shouldNotifyForEvent(event, PUBKEY, {
      participatedRootIds: EMPTY,
      followedRootIds: EMPTY,
      authoredRootIds: EMPTY,
      channelId: CHANNEL_ID,
    }),
    true,
  );
});

test("no channelId passed behaves as if unmuted (top-level notifies)", () => {
  const event = makeEvent([hTag(CHANNEL_ID)]);
  // mutedChannelIds has the channel but channelId is null (default)
  assert.equal(
    shouldNotifyForEvent(event, PUBKEY, {
      participatedRootIds: EMPTY,
      followedRootIds: EMPTY,
      authoredRootIds: EMPTY,
      mutedChannelIds: new Set([CHANNEL_ID]),
    }),
    true,
  );
});

test("thread in mutedRootIds AND in muted channel is suppressed", () => {
  const event = makeEvent([
    hTag(CHANNEL_ID),
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
  ]);
  // Both the root thread and the channel are muted; mute channel check fires first
  assert.equal(
    shouldNotifyForEvent(event, PUBKEY, {
      participatedRootIds: new Set([ROOT_ID]),
      followedRootIds: EMPTY,
      authoredRootIds: EMPTY,
      mutedRootIds: new Set([ROOT_ID]),
      mutedChannelIds: new Set([CHANNEL_ID]),
      channelId: CHANNEL_ID,
    }),
    false,
  );
});
