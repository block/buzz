import assert from "node:assert/strict";
import { test } from "node:test";
import { sortConversationsByRecency } from "./conversationRecency.ts";

const channel = (id, lastMessageAt, extra = {}) => ({
  id,
  name: id,
  lastMessageAt,
  isMember: true,
  archivedAt: null,
  ...extra,
});

test("combined recency mixes DMs and channels using the newest available timestamp", () => {
  const channels = [
    channel("channel", "2026-09-09T12:00:00Z"),
    channel("dm", "2026-09-09T11:00:00Z", { channelType: "dm" }),
    channel("empty", null),
    channel("archived", "2026-09-10T12:00:00Z", { archivedAt: "2026-09-10" }),
    channel("unjoined", "2026-09-10T12:00:00Z", { isMember: false }),
  ];
  const messages = [
    {
      channel: channels[1],
      latestAt: Date.parse("2026-09-09T13:00:00Z") / 1000,
    },
  ];
  assert.deepEqual(
    sortConversationsByRecency(channels, messages).map((c) => c.id),
    ["dm", "channel", "empty"],
  );
  assert.equal(channels[0].id, "channel");
  assert.deepEqual(
    sortConversationsByRecency(channels, []).map((c) => c.id),
    ["channel", "dm", "empty"],
  );
});

test("missing and invalid recency have a deterministic order", () => {
  const channels = [
    channel("z", "invalid"),
    channel("b", null, { name: "Same" }),
    channel("a", null, { name: "Same" }),
  ];
  assert.deepEqual(
    sortConversationsByRecency(channels, []).map((c) => c.id),
    ["a", "b", "z"],
  );
});
