import assert from "node:assert/strict";
import test from "node:test";

import { sortDmChannelsByLabel } from "./dmSidebarSort.ts";

function makeDm(id, name) {
  return {
    archivedAt: null,
    channelType: "dm",
    description: "",
    id,
    isMember: true,
    lastMessageAt: null,
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
