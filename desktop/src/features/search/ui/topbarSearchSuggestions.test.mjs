import assert from "node:assert/strict";
import test from "node:test";

import { getSuggestedChannels } from "./topbarSearchSuggestions.ts";

function channel(id, options = {}) {
  return {
    archivedAt: null,
    channelType: "stream",
    description: "",
    id,
    isMember: true,
    lastMessageAt: null,
    name: id,
    ...options,
  };
}

test("unread conversations come before recent conversations in activity order", () => {
  const channels = [
    channel("recent", { lastMessageAt: "2026-08-24T12:00:00Z" }),
    channel("older-unread", { lastMessageAt: "2026-08-24T10:00:00Z" }),
    channel("newer-unread", { lastMessageAt: "2026-08-24T11:00:00Z" }),
  ];

  const result = getSuggestedChannels(
    channels,
    new Set(["older-unread", "newer-unread"]),
  );

  assert.deepEqual(
    result.unreadChannels.map(({ id }) => id),
    ["newer-unread", "older-unread"],
  );
  assert.deepEqual(
    result.recentChannels.map(({ id }) => id),
    ["recent"],
  );
});

test("all unread conversations remain visible while recent results stay capped", () => {
  const channels = Array.from({ length: 10 }, (_, index) =>
    channel(`channel-${index}`, {
      lastMessageAt: `2026-08-24T${String(index).padStart(2, "0")}:00:00Z`,
    }),
  );

  const result = getSuggestedChannels(
    channels,
    new Set(channels.slice(0, 6).map(({ id }) => id)),
  );

  assert.equal(result.unreadChannels.length, 6);
  assert.equal(result.recentChannels.length, 4);
});

test("archived and unjoined non-DM conversations are omitted", () => {
  const result = getSuggestedChannels(
    [
      channel("archived", { archivedAt: "2026-08-24T12:00:00Z" }),
      channel("unjoined", { isMember: false }),
      channel("dm", { channelType: "dm", isMember: false }),
    ],
    new Set(["archived", "unjoined", "dm"]),
  );

  assert.deepEqual(
    result.unreadChannels.map(({ id }) => id),
    ["dm"],
  );
});
