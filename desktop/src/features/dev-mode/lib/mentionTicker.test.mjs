import assert from "node:assert/strict";
import test from "node:test";

import { toDevMentionTickerItem } from "./mentionTicker.ts";

const SELF = "a".repeat(64);
const EVENT = {
  id: "e".repeat(64),
  pubkey: "b".repeat(64),
  kind: 9,
  content: "  waiting on access\nfrom the owner ",
  created_at: 1,
  tags: [
    ["h", "channel-id"],
    ["p", SELF],
    ["e", "c".repeat(64), "", "root"],
    ["e", "d".repeat(64), "", "reply"],
    ["buzz-notification", "blocked"],
  ],
};

test("builds a blocked ticker target from a mentioned thread reply", () => {
  assert.deepEqual(
    toDevMentionTickerItem(
      EVENT,
      SELF,
      [{ id: "channel-id", name: "agent-work" }],
      new Set([EVENT.pubkey]),
    ),
    {
      channelId: "channel-id",
      channelName: "agent-work",
      content: "waiting on access from the owner",
      eventId: EVENT.id,
      blocked: true,
      threadRootId: "c".repeat(64),
    },
  );
});

test("ignores messages that do not mention the current user", () => {
  assert.equal(
    toDevMentionTickerItem(
      { ...EVENT, tags: [["h", "channel-id"]] },
      SELF,
      [],
      new Set([EVENT.pubkey]),
    ),
    null,
  );
});

test("does not treat a human-authored blocked tag as an agent blocker", () => {
  assert.equal(
    toDevMentionTickerItem(EVENT, SELF, [], new Set())?.blocked,
    false,
  );
});
