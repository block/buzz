import assert from "node:assert/strict";
import test from "node:test";
import { QueryClient } from "@tanstack/react-query";
import {
  consumeHydratedChannel,
  hydrateChannelHeads,
} from "./channelHeadCache.ts";
import { channelMessagesKey } from "./messageQueryKeys.ts";
import { reconcileFetchedChannelWindow } from "../hooks.ts";
const channelId = "channel-a";
const root = {
  id: "a".repeat(64),
  pubkey: "b".repeat(64),
  created_at: 10,
  kind: 40002,
  tags: [["h", channelId]],
  content: "persisted",
  sig: "",
};
const replacement = {
  ...root,
  id: "c".repeat(64),
  created_at: 11,
  content: "relay",
};
function bounds() {
  return {
    id: "d".repeat(64),
    pubkey: "e".repeat(64),
    created_at: 12,
    kind: 39006,
    tags: [
      ["h", channelId],
      ["d", `${channelId}:head`],
    ],
    content: JSON.stringify({ has_more: false, next_cursor: null }),
    sig: "",
  };
}
function install(entries) {
  globalThis.window = {
    localStorage: { getItem: () => null },
    __TAURI_INTERNALS__: {
      invoke: async (command) =>
        command === "channel_head_cache_load" ? entries : null,
    },
  };
}
test("hydrates stale data and consumes its mount gate once", async () => {
  install([
    { channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 },
  ]);
  const client = new QueryClient();
  await hydrateChannelHeads(client, {
    pubkey: "f".repeat(64),
    relayUrl: "wss://relay",
  });
  assert.deepEqual(client.getQueryData(channelMessagesKey(channelId)), [root]);
  assert.equal(
    client.getQueryState(channelMessagesKey(channelId)).dataUpdatedAt,
    0,
  );
  assert.equal(consumeHydratedChannel(client, channelId), true);
  assert.equal(consumeHydratedChannel(client, channelId), false);
});
test("authoritative refresh deletes a vanished hydrated row", async () => {
  install([
    { channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 },
  ]);
  const client = new QueryClient();
  await hydrateChannelHeads(client, {
    pubkey: "f".repeat(64),
    relayUrl: "wss://relay",
  });
  const next = reconcileFetchedChannelWindow(
    client,
    channelId,
    [replacement, bounds()],
    client.getQueryData(channelMessagesKey(channelId)),
    new AbortController().signal,
  );
  assert.deepEqual(
    next.map((e) => e.id),
    [replacement.id],
  );
});
test("drops malformed entries independently", async () => {
  install([
    { channelId: "bad", events: [root], savedAt: 1, lastVisitedAt: 2 },
    { channelId, events: [root, bounds()], savedAt: 1, lastVisitedAt: 1 },
  ]);
  const client = new QueryClient();
  await hydrateChannelHeads(client, {
    pubkey: "f".repeat(64),
    relayUrl: "wss://relay",
  });
  assert.equal(client.getQueryData(channelMessagesKey("bad")), undefined);
  assert.deepEqual(client.getQueryData(channelMessagesKey(channelId)), [root]);
});
