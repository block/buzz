import assert from "node:assert/strict";
import test from "node:test";

import { memberChannelIdsKey } from "./HuddlePresenceContext.tsx";

test("member channel key ignores ordinary channel recency updates", () => {
  const before = [
    { id: "general", isMember: true, lastMessageAt: 10 },
    { id: "design", isMember: true, lastMessageAt: 20 },
  ];
  const after = [{ ...before[0], lastMessageAt: 30 }, before[1]];

  assert.equal(memberChannelIdsKey(before), memberChannelIdsKey(after));
});

test("member channel key changes when membership changes", () => {
  const before = [
    { id: "general", isMember: true },
    { id: "design", isMember: false },
  ];
  const after = before.map((channel) =>
    channel.id === "design" ? { ...channel, isMember: true } : channel,
  );

  assert.notEqual(memberChannelIdsKey(before), memberChannelIdsKey(after));
});
