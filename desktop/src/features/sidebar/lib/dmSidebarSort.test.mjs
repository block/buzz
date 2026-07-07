import assert from "node:assert/strict";
import test from "node:test";

import {
  sortDmChannelsByLabel,
  sortDmChannelsForSidebar,
} from "./dmSidebarSort.ts";

function makeDm(id, name, lastMessageAt = null) {
  return {
    archivedAt: null,
    channelType: "dm",
    description: "",
    id,
    isMember: true,
    lastMessageAt,
    memberCount: 2,
    memberPubkeys: [],
    name,
    participantPubkeys: [],
    participants: [],
    purpose: null,
    topic: null,
    ttlDeadline: null,
    ttlSeconds: null,
    visibility: "private",
  };
}

test("sorts direct messages by resolved display label", () => {
  const sorted = sortDmChannelsByLabel(
    [
      makeDm("c", "Group DM (3)"),
      makeDm("a", "Group DM (3)"),
      makeDm("b", "Group DM (3)"),
    ],
    {
      a: "Fizz",
      b: "Brain",
      c: "Wes",
    },
  );

  assert.deepEqual(
    sorted.map((channel) => channel.id),
    ["b", "a", "c"],
  );
});

test("falls back to channel name until labels resolve", () => {
  const sorted = sortDmChannelsByLabel(
    [makeDm("b", "Zed"), makeDm("a", "Amy")],
    {},
  );

  assert.deepEqual(
    sorted.map((channel) => channel.id),
    ["a", "b"],
  );
});

test("uses channel id as a deterministic tie breaker", () => {
  const sorted = sortDmChannelsByLabel(
    [makeDm("b", "Group DM (3)"), makeDm("a", "Group DM (3)")],
    {
      a: "Wes",
      b: "Wes",
    },
  );

  assert.deepEqual(
    sorted.map((channel) => channel.id),
    ["a", "b"],
  );
});

test("sortDmChannelsForSidebar: alpha mode matches label sort", () => {
  const sorted = sortDmChannelsForSidebar(
    [makeDm("b", "Zed"), makeDm("a", "Amy")],
    {},
    "alpha",
  );

  assert.deepEqual(
    sorted.map((channel) => channel.id),
    ["a", "b"],
  );
});

test("sortDmChannelsForSidebar: recent mode puts newest message first", () => {
  const sorted = sortDmChannelsForSidebar(
    [
      makeDm("old", "Amy", "2026-01-01T00:00:00Z"),
      makeDm("new", "Zed", "2026-06-01T00:00:00Z"),
    ],
    {},
    "recent",
  );

  assert.deepEqual(
    sorted.map((channel) => channel.id),
    ["new", "old"],
  );
});

test("sortDmChannelsForSidebar: recent mode sinks quiet DMs in label order", () => {
  const sorted = sortDmChannelsForSidebar(
    [
      makeDm("quiet-z", "Zed"),
      makeDm("active", "Amy", "2026-06-01T00:00:00Z"),
      makeDm("quiet-a", "Bea"),
    ],
    {},
    "recent",
  );

  assert.deepEqual(
    sorted.map((channel) => channel.id),
    ["active", "quiet-a", "quiet-z"],
  );
});
