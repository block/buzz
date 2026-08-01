import assert from "node:assert/strict";
import test from "node:test";

import { messageMentionPubkeys } from "./messageMentionPubkeys.ts";

function channel(overrides = {}) {
  return {
    id: "dm-1",
    name: "DM",
    channelType: "dm",
    visibility: "private",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: ["OWNER", "AGENT"],
    participantPubkeys: ["owner", "agent"],
    participants: [],
    lastMessageAt: null,
    archivedAt: null,
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

test("plain DM messages p-tag every recipient except the sender", () => {
  assert.deepEqual(messageMentionPubkeys(channel(), "owner"), ["agent"]);
});

test("DM recipients and explicit mentions are normalized and deduplicated", () => {
  assert.deepEqual(
    messageMentionPubkeys(channel(), "OWNER", ["AGENT", "third"]),
    ["agent", "third"],
  );
});

test("stream messages preserve explicit-mention semantics", () => {
  assert.deepEqual(
    messageMentionPubkeys(
      channel({ channelType: "stream", memberPubkeys: ["owner", "agent"] }),
      "owner",
      [],
    ),
    [],
  );
});

test("stream messages p-tag selected agent mentions", () => {
  const agentPubkey =
    "e3066c9a6081fcd4d0945c8f14f52bbc71a87c74b34a6594fe540f73e41acf09";

  assert.deepEqual(
    messageMentionPubkeys(
      channel({ channelType: "stream", memberPubkeys: ["owner", agentPubkey] }),
      "owner",
      [agentPubkey.toUpperCase()],
    ),
    [agentPubkey],
  );
});
