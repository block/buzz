import assert from "node:assert/strict";
import test from "node:test";

import { DEFAULT_CHANNEL_NOTIFY_STATE } from "../features/notifications/lib/resolveChannelNotifyState.ts";
import {
  feedOwnsThreadReplyNotification,
  shouldBounceForChannelNotification,
} from "./AppShell.helpers.ts";

test("shouldBounceForChannelNotification_allowsTopLevelChannelMessages", () => {
  assert.equal(shouldBounceForChannelNotification([["h", "channel"]]), true);
});

test("shouldBounceForChannelNotification_suppressesThreadReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
    ]),
    false,
  );
});

test("shouldBounceForChannelNotification_allowsBroadcastReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
      ["broadcast", "1"],
    ]),
    true,
  );
});

// ── feedOwnsThreadReplyNotification (NIP-CN / NIP-CM live-vs-feed ownership) ──

const PUBKEY = "ab".padEnd(64, "0");

function notifyState(overrides = {}) {
  return { ...DEFAULT_CHANNEL_NOTIFY_STATE, ...overrides };
}

const NOTIFY_CHANNEL_REPLY = [
  ["h", "channel"],
  ["e", "root", "", "reply"],
  ["notify", "channel"],
];

test("feedOwnsThreadReplyNotification_suppressesMarkerReplyInADefaultChannel", () => {
  // The feed will carry this as a mention, so the live banner would be the
  // second one for the same event id.
  assert.equal(
    feedOwnsThreadReplyNotification(
      notifyState(),
      NOTIFY_CHANNEL_REPLY,
      PUBKEY,
    ),
    true,
  );
});

test("feedOwnsThreadReplyNotification_keepsMarkerReplyWhenBroadcastsAreOff", () => {
  // The feed suppresses the item, so the single live banner must survive.
  assert.equal(
    feedOwnsThreadReplyNotification(
      notifyState({ broadcasts: false }),
      NOTIFY_CHANNEL_REPLY,
      PUBKEY,
    ),
    false,
  );
});

test("feedOwnsThreadReplyNotification_keepsMarkerReplyInAMutedChannel", () => {
  assert.equal(
    feedOwnsThreadReplyNotification(
      notifyState({ level: "mute" }),
      NOTIFY_CHANNEL_REPLY,
      PUBKEY,
    ),
    false,
  );
});

test("feedOwnsThreadReplyNotification_ignoresRepliesWithoutAMarker", () => {
  // A plain followed-thread reply is live-only; the feed never sees it.
  assert.equal(
    feedOwnsThreadReplyNotification(
      notifyState(),
      [
        ["h", "channel"],
        ["e", "root", "", "reply"],
      ],
      PUBKEY,
    ),
    false,
  );
});

test("feedOwnsThreadReplyNotification_suppressesHereMarkerReply", () => {
  assert.equal(
    feedOwnsThreadReplyNotification(
      notifyState(),
      [
        ["h", "channel"],
        ["e", "root", "", "reply"],
        ["notify", "here"],
      ],
      PUBKEY,
    ),
    true,
  );
});
