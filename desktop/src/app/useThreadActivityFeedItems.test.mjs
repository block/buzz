import assert from "node:assert/strict";
import test from "node:test";

import { buildThreadActivityFeedItems } from "./useThreadActivityFeedItems.ts";

const CHANNEL_ID = "channel-1";

function threadActivityItem(overrides) {
  const rootId = overrides.rootId ?? "root-1";
  return {
    id: overrides.id ?? "reply-1",
    kind: overrides.kind ?? 9,
    pubkey: overrides.pubkey ?? "author",
    content: overrides.content ?? "reply",
    createdAt: overrides.createdAt ?? 1,
    channelId: overrides.channelId ?? CHANNEL_ID,
    channelName: overrides.channelName ?? "general",
    tags: overrides.tags ?? [
      ["h", overrides.channelId ?? CHANNEL_ID],
      ["e", rootId, "", "root"],
      ["e", overrides.parentId ?? "parent-1", "", "reply"],
    ],
  };
}

test("thread activity feed projection filters muted roots", () => {
  const items = buildThreadActivityFeedItems(
    [
      threadActivityItem({ id: "muted-reply", rootId: "muted-root" }),
      threadActivityItem({ id: "visible-reply", rootId: "visible-root" }),
    ],
    new Set(["muted-root"]),
    [{ id: CHANNEL_ID, name: "general", channelType: "stream" }],
  );

  assert.deepEqual(
    items.map((item) => item.id),
    ["visible-reply"],
  );
  assert.equal(items[0]?.category, "activity");
  assert.equal(items[0]?.channelType, "stream");
});
