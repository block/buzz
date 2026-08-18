import assert from "node:assert/strict";
import test from "node:test";

import { fromRawFeedItem } from "./tauri.ts";

const rawFeedItem = (overrides = {}) => ({
  id: "event-id",
  kind: 9,
  pubkey: "author",
  content: "hello",
  created_at: 1,
  channel_id: "channel-id",
  channel_name: "",
  channel_type: null,
  tags: [["h", "channel-id"]],
  category: "mention",
  ...overrides,
});

test("canonicalizes the native null channel_type to undefined", () => {
  // Native get_feed serializes FeedItemInfo.channel_type (Option<String>)
  // as `null`, never omitting the key. FeedItem declares channelType as
  // optional, and the DM notification filter distinguishes "unresolved"
  // via `=== undefined` — so null must not survive the boundary.
  const item = fromRawFeedItem(rawFeedItem());

  assert.equal(item.channelType, undefined);
  assert.ok("channelType" in item);
});

test("passes a present channel_type through unchanged", () => {
  const item = fromRawFeedItem(rawFeedItem({ channel_type: "dm" }));

  assert.equal(item.channelType, "dm");
});
