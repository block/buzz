import assert from "node:assert/strict";
import test from "node:test";

import {
  allowsFeedItemForChannel,
  hasMentionForEvent,
  isHighPriorityEventForUser,
  notifyDecisionForEvent,
  tagsMentionPubkey,
} from "./shouldNotify.ts";
import { resolveChannelNotifyState } from "./resolveChannelNotifyState.ts";

const PUBKEY = "a".repeat(64);
const OTHER_PUBKEY = "b".repeat(64);
const CHANNEL_ID =
  "channel-0000000000000000000000000000000000000000000000000000";
const ROOT_ID = `root-${"0".repeat(59)}`;
const PARENT_ID = `parent-${"0".repeat(57)}`;

const EMPTY = new Set();

const unreadFor = (event, pubkey, options) =>
  notifyDecisionForEvent(event, pubkey, options).unread;

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

test("tagsMentionPubkey: matches a p-tag case-insensitively", () => {
  assert.equal(tagsMentionPubkey([pTag(PUBKEY)], PUBKEY), true);
  assert.equal(tagsMentionPubkey([pTag(PUBKEY.toUpperCase())], PUBKEY), true);
});

test("tagsMentionPubkey: no p-tag for the reader returns false", () => {
  assert.equal(tagsMentionPubkey([pTag(OTHER_PUBKEY)], PUBKEY), false);
  assert.equal(tagsMentionPubkey([hTag(CHANNEL_ID)], PUBKEY), false);
  assert.equal(tagsMentionPubkey([], PUBKEY), false);
  assert.equal(tagsMentionPubkey(undefined, PUBKEY), false);
});

test("tagsMentionPubkey: an empty pubkey never matches", () => {
  assert.equal(tagsMentionPubkey([pTag(PUBKEY)], ""), false);
});

test("top-level message in muted channel is suppressed", () => {
  const event = makeEvent([hTag(CHANNEL_ID)]);
  assert.equal(
    unreadFor(event, PUBKEY, {
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
    unreadFor(event, PUBKEY, {
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
    unreadFor(event, PUBKEY, {
      participatedRootIds: new Set([ROOT_ID]),
      followedRootIds: EMPTY,
      authoredRootIds: EMPTY,
      mutedChannelIds: new Set([CHANNEL_ID]),
      channelId: CHANNEL_ID,
    }),
    false,
  );
});

test("broadcast reply in muted channel is suppressed (NIP-CN: mute beats broadcast)", () => {
  const event = makeEvent([
    hTag(CHANNEL_ID),
    replyTag(ROOT_ID),
    broadcastTag(),
  ]);
  assert.equal(
    unreadFor(event, PUBKEY, {
      participatedRootIds: EMPTY,
      followedRootIds: EMPTY,
      authoredRootIds: EMPTY,
      mutedChannelIds: new Set([CHANNEL_ID]),
      channelId: CHANNEL_ID,
    }),
    false,
  );
});

test("top-level message in unmuted channel notifies", () => {
  const event = makeEvent([hTag(CHANNEL_ID)]);
  assert.equal(
    unreadFor(event, PUBKEY, {
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
    unreadFor(event, PUBKEY, {
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
    unreadFor(event, PUBKEY, {
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

// ── NIP-CN per-channel levels ─────────────────────────────────────────────────

const NOW = 1_000;
const notifyTag = (mode) => ["notify", mode];

/** Real resolver over a single-channel prefs + legacy pair, as AppShell wires it. */
const prefsLookup =
  (entry, legacyEntry = null, now = NOW) =>
  (channelId) =>
    resolveChannelNotifyState(
      channelId,
      { version: 1, channels: entry ? { [CHANNEL_ID]: entry } : {} },
      {
        version: 1,
        channels: legacyEntry ? { [CHANNEL_ID]: legacyEntry } : {},
      },
      now,
    );

const decide = (event, options = {}) =>
  notifyDecisionForEvent(event, PUBKEY, {
    participatedRootIds: EMPTY,
    followedRootIds: EMPTY,
    authoredRootIds: EMPTY,
    channelId: CHANNEL_ID,
    ...options,
  });

const level = (value) => ({ level: value, updatedAt: 1 });
const NONE = { unread: false, alert: false, highPriority: false };
const ALERT = { unread: true, alert: true, highPriority: false };
const QUIET = { unread: true, alert: false, highPriority: false };
const MENTION = { unread: true, alert: true, highPriority: true };

const topLevel = () => makeEvent([hTag(CHANNEL_ID)]);
const broadcast = () =>
  makeEvent([
    hTag(CHANNEL_ID),
    rootTag(ROOT_ID),
    replyTag(PARENT_ID),
    broadcastTag(),
  ]);
const threadReply = () =>
  makeEvent([hTag(CHANNEL_ID), rootTag(ROOT_ID), replyTag(PARENT_ID)]);
const channelMention = (mode = "channel") =>
  makeEvent([hTag(CHANNEL_ID), notifyTag(mode)]);

test("top-level post: level 'all' alerts, 'mentions' is quiet, 'mute' is silent", () => {
  assert.deepEqual(
    decide(topLevel(), { channelPrefs: prefsLookup(level("all")) }),
    ALERT,
  );
  assert.deepEqual(
    decide(topLevel(), { channelPrefs: prefsLookup(level("mentions")) }),
    QUIET,
  );
  assert.deepEqual(
    decide(topLevel(), { channelPrefs: prefsLookup(level("mute")) }),
    NONE,
  );
});

test("broadcast reply follows the level like a top-level post", () => {
  assert.deepEqual(
    decide(broadcast(), { channelPrefs: prefsLookup(level("all")) }),
    MENTION,
  );
  assert.deepEqual(
    decide(broadcast(), { channelPrefs: prefsLookup(level("mentions")) }),
    QUIET,
  );
  assert.deepEqual(
    decide(broadcast(), { channelPrefs: prefsLookup(level("mute")) }),
    NONE,
  );
});

test("direct p-tag mention pierces every level", () => {
  const event = makeEvent([hTag(CHANNEL_ID), pTag(PUBKEY)]);
  for (const value of ["all", "mentions", "mute"]) {
    assert.deepEqual(
      decide(event, { channelPrefs: prefsLookup(level(value)) }),
      MENTION,
    );
  }
});

test("@channel / @here is mention tier at levels 'all' and 'mentions'", () => {
  for (const mode of ["channel", "here"]) {
    assert.deepEqual(
      decide(channelMention(mode), { channelPrefs: prefsLookup(level("all")) }),
      MENTION,
    );
    assert.deepEqual(
      decide(channelMention(mode), {
        channelPrefs: prefsLookup(level("mentions")),
      }),
      MENTION,
    );
  }
});

test("@channel in a muted channel is silent, not mention tier", () => {
  assert.deepEqual(
    decide(channelMention(), { channelPrefs: prefsLookup(level("mute")) }),
    NONE,
  );
});

test("broadcasts opt-out demotes @channel to an ordinary post", () => {
  assert.deepEqual(
    decide(channelMention(), {
      channelPrefs: prefsLookup({ broadcasts: false, updatedAt: 1 }),
    }),
    ALERT,
  );
  assert.deepEqual(
    decide(channelMention(), {
      channelPrefs: prefsLookup({
        level: "mentions",
        broadcasts: false,
        updatedAt: 1,
      }),
    }),
    QUIET,
  );
});

test("broadcasts opt-out does not gate NIP-CW broadcast replies", () => {
  assert.deepEqual(
    decide(broadcast(), {
      channelPrefs: prefsLookup({ broadcasts: false, updatedAt: 1 }),
    }),
    MENTION,
  );
});

test("an unknown notify value is not treated as a channel mention", () => {
  const event = makeEvent([hTag(CHANNEL_ID), notifyTag("someone-else")]);
  assert.deepEqual(
    decide(event, { channelPrefs: prefsLookup(level("mentions")) }),
    QUIET,
  );
});

test("timed mute silences the channel until it expires", () => {
  const entry = { muteUntil: NOW + 60, updatedAt: 1 };
  assert.deepEqual(
    decide(topLevel(), { channelPrefs: prefsLookup(entry) }),
    NONE,
  );
  assert.deepEqual(
    decide(topLevel(), { channelPrefs: prefsLookup(entry, null, NOW + 61) }),
    ALERT,
  );
});

test("timed mute restores the stored level on expiry", () => {
  const entry = { level: "mentions", muteUntil: NOW + 60, updatedAt: 1 };
  assert.deepEqual(
    decide(topLevel(), { channelPrefs: prefsLookup(entry) }),
    NONE,
  );
  assert.deepEqual(
    decide(topLevel(), { channelPrefs: prefsLookup(entry, null, NOW + 61) }),
    QUIET,
  );
});

test("followAllThreads notifies replies to threads the user never touched", () => {
  const entry = { followAllThreads: true, updatedAt: 1 };
  assert.deepEqual(
    decide(threadReply(), { channelPrefs: prefsLookup(entry) }),
    ALERT,
  );
  assert.deepEqual(
    decide(threadReply(), { channelPrefs: prefsLookup(level("all")) }),
    NONE,
  );
});

test("followAllThreads loses to a thread mute and to channel mute", () => {
  assert.deepEqual(
    decide(threadReply(), {
      channelPrefs: prefsLookup({ followAllThreads: true, updatedAt: 1 }),
      mutedRootIds: new Set([ROOT_ID]),
    }),
    NONE,
  );
  assert.deepEqual(
    decide(threadReply(), {
      channelPrefs: prefsLookup({
        level: "mute",
        followAllThreads: true,
        updatedAt: 1,
      }),
    }),
    NONE,
  );
});

test("explicit thread follows still alert at level 'mentions'", () => {
  assert.deepEqual(
    decide(threadReply(), {
      channelPrefs: prefsLookup(level("mentions")),
      followedRootIds: new Set([ROOT_ID]),
    }),
    ALERT,
  );
});

test("channel mute beats thread participation", () => {
  assert.deepEqual(
    decide(threadReply(), {
      channelPrefs: prefsLookup(level("mute")),
      participatedRootIds: new Set([ROOT_ID]),
    }),
    NONE,
  );
});

test("legacy interop: a newer legacy mute silences prefs level 'mentions'", () => {
  assert.deepEqual(
    decide(topLevel(), {
      channelPrefs: prefsLookup(
        { level: "mentions", updatedAt: 10 },
        { muted: true, updatedAt: 20 },
      ),
    }),
    NONE,
  );
});

test("legacy interop: a newer legacy unmute revives a stale prefs mute", () => {
  assert.deepEqual(
    decide(topLevel(), {
      channelPrefs: prefsLookup(
        { level: "mute", updatedAt: 10 },
        { muted: false, updatedAt: 20 },
      ),
    }),
    ALERT,
  );
});

test("channelPrefs is authoritative over the legacy mutedChannelIds set", () => {
  assert.deepEqual(
    decide(topLevel(), {
      channelPrefs: prefsLookup(level("all")),
      mutedChannelIds: new Set([CHANNEL_ID]),
    }),
    ALERT,
  );
});

// ── isHighPriorityEventForUser ────────────────────────────────────────────────

test("isHighPriorityEventForUser: @channel is mention tier only when heard", () => {
  const event = channelMention();
  const at = (entry) =>
    isHighPriorityEventForUser(event, PUBKEY, {
      channelId: CHANNEL_ID,
      channelPrefs: prefsLookup(entry),
    });
  assert.equal(at(level("all")), true);
  assert.equal(at(level("mentions")), true);
  assert.equal(at(level("mute")), false);
  assert.equal(at({ broadcasts: false, updatedAt: 1 }), false);
});

test("isHighPriorityEventForUser: broadcast reply is mention tier only at level 'all'", () => {
  const event = broadcast();
  const at = (entry) =>
    isHighPriorityEventForUser(event, PUBKEY, {
      channelId: CHANNEL_ID,
      channelPrefs: prefsLookup(entry),
    });
  assert.equal(at(level("all")), true);
  assert.equal(at(level("mentions")), false);
  assert.equal(at(level("mute")), false);
});

test("isHighPriorityEventForUser: p-tag mention stays mention tier in a muted channel", () => {
  const event = makeEvent([hTag(CHANNEL_ID), pTag(PUBKEY)]);
  assert.equal(
    isHighPriorityEventForUser(event, PUBKEY, {
      channelId: CHANNEL_ID,
      channelPrefs: prefsLookup(level("mute")),
    }),
    true,
  );
});

test("isHighPriorityEventForUser: legacy mutedChannelIds demotes a broadcast reply", () => {
  const event = broadcast();
  assert.equal(
    isHighPriorityEventForUser(event, PUBKEY, {
      channelId: CHANNEL_ID,
      mutedChannelIds: new Set([CHANNEL_ID]),
    }),
    false,
  );
  assert.equal(isHighPriorityEventForUser(event, PUBKEY), true);
});

// ── allowsFeedItemForChannel (Home feed / badge seam) ─────────────────────────

const feedState = (overrides = {}) => ({
  level: "all",
  timedMuteActive: false,
  desktop: true,
  followAllThreads: false,
  broadcasts: true,
  hidden: false,
  ...overrides,
});

test("allowsFeedItemForChannel: ordinary item is dropped only while muted", () => {
  assert.equal(allowsFeedItemForChannel(feedState(), false, [], PUBKEY), true);
  assert.equal(
    allowsFeedItemForChannel(
      feedState({ level: "mentions" }),
      false,
      [],
      PUBKEY,
    ),
    true,
  );
  assert.equal(
    allowsFeedItemForChannel(feedState({ level: "mute" }), false, [], PUBKEY),
    false,
  );
});

test("allowsFeedItemForChannel: a direct mention pierces the mute", () => {
  assert.equal(
    allowsFeedItemForChannel(feedState({ level: "mute" }), true, [], PUBKEY),
    true,
  );
  assert.equal(
    allowsFeedItemForChannel(
      feedState({ level: "mute" }),
      false,
      [pTag(PUBKEY)],
      PUBKEY,
    ),
    true,
  );
});

test("allowsFeedItemForChannel: a notify-tag item obeys the level, not the mention exemption", () => {
  const tags = [["notify", "channel"]];
  assert.equal(allowsFeedItemForChannel(feedState(), true, tags, PUBKEY), true);
  assert.equal(
    allowsFeedItemForChannel(
      feedState({ level: "mentions" }),
      true,
      tags,
      PUBKEY,
    ),
    true,
  );
  assert.equal(
    allowsFeedItemForChannel(feedState({ level: "mute" }), true, tags, PUBKEY),
    false,
  );
});

test("allowsFeedItemForChannel: the broadcasts opt-out drops notify-tag items", () => {
  assert.equal(
    allowsFeedItemForChannel(
      feedState({ broadcasts: false }),
      true,
      [["notify", "here"]],
      PUBKEY,
    ),
    false,
  );
});

test("allowsFeedItemForChannel: a notify item that also p-tags the reader pierces the mute", () => {
  const tags = [["notify", "channel"], pTag(PUBKEY)];
  assert.equal(
    allowsFeedItemForChannel(feedState({ level: "mute" }), true, tags, PUBKEY),
    true,
  );
  // The relay's category is irrelevant — the p-tag alone carries the rung.
  assert.equal(
    allowsFeedItemForChannel(feedState({ level: "mute" }), false, tags, PUBKEY),
    true,
  );
});

test("allowsFeedItemForChannel: a notify item that also p-tags the reader survives the broadcasts opt-out", () => {
  assert.equal(
    allowsFeedItemForChannel(
      feedState({ broadcasts: false }),
      true,
      [["notify", "here"], pTag(PUBKEY)],
      PUBKEY,
    ),
    true,
  );
  assert.equal(
    allowsFeedItemForChannel(
      feedState({ level: "mute", broadcasts: false }),
      true,
      [["notify", "here"], pTag(PUBKEY)],
      PUBKEY,
    ),
    true,
  );
});

test("allowsFeedItemForChannel: a p-tag of somebody else does not pierce", () => {
  assert.equal(
    allowsFeedItemForChannel(
      feedState({ level: "mute" }),
      true,
      [["notify", "channel"], pTag(OTHER_PUBKEY)],
      PUBKEY,
    ),
    false,
  );
});

test("allowsFeedItemForChannel: an empty reader pubkey keeps the notify gate", () => {
  const tags = [["notify", "channel"], pTag(PUBKEY)];
  assert.equal(
    allowsFeedItemForChannel(feedState({ level: "mute" }), true, tags, ""),
    false,
  );
  assert.equal(
    allowsFeedItemForChannel(feedState({ broadcasts: false }), true, tags, ""),
    false,
  );
  assert.equal(allowsFeedItemForChannel(feedState(), true, tags, ""), true);
});
