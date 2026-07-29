import assert from "node:assert/strict";
import test from "node:test";

import {
  filterVisibleTimelineMessages,
  isJoinLeaveSystemMessage,
} from "./ChannelPane.helpers.ts";

const KIND_SYSTEM_MESSAGE = 40099;
const KIND_CHANNEL_MESSAGE = 9;

function systemMessage(id, type) {
  return {
    id,
    createdAt: 0,
    author: "system",
    body: JSON.stringify({ type }),
    kind: KIND_SYSTEM_MESSAGE,
  };
}

function chatMessage(id) {
  return {
    id,
    createdAt: 0,
    author: "alice",
    body: "hello",
    kind: KIND_CHANNEL_MESSAGE,
  };
}

function channel(overrides = {}) {
  return {
    id: "chan-1",
    name: "general",
    description: "",
    topic: null,
    purpose: null,
    visibility: "open",
    channelType: "stream",
    createdAt: "2025-01-01T00:00:00Z",
    archivedAt: null,
    memberCount: 1,
    lastMessageAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

test("isJoinLeaveSystemMessage matches membership-change system rows", () => {
  assert.equal(
    isJoinLeaveSystemMessage(systemMessage("a", "member_joined")),
    true,
  );
  assert.equal(
    isJoinLeaveSystemMessage(systemMessage("b", "member_left")),
    true,
  );
  assert.equal(
    isJoinLeaveSystemMessage(systemMessage("c", "member_removed")),
    true,
  );
  assert.equal(
    isJoinLeaveSystemMessage(systemMessage("d", "topic_changed")),
    false,
  );
  assert.equal(isJoinLeaveSystemMessage(chatMessage("e")), false);
});

test("join/leave rows are hidden when the setting is off (the default)", () => {
  const messages = [
    chatMessage("m1"),
    systemMessage("j1", "member_joined"),
    systemMessage("l1", "member_left"),
    systemMessage("r1", "member_removed"),
    systemMessage("t1", "topic_changed"),
  ];

  for (const activeChannel of [channel(), null]) {
    const visible = filterVisibleTimelineMessages(
      messages,
      activeChannel,
      false,
    );
    assert.deepEqual(
      visible.map((m) => m.id),
      ["m1", "t1"],
    );
  }
});

test("join/leave rows are shown when the setting is on", () => {
  const messages = [
    chatMessage("m1"),
    systemMessage("j1", "member_joined"),
    systemMessage("l1", "member_left"),
    systemMessage("r1", "member_removed"),
  ];
  const visible = filterVisibleTimelineMessages(messages, channel(), true);
  assert.deepEqual(
    visible.map((m) => m.id),
    ["m1", "j1", "l1", "r1"],
  );
});

test("welcome channels hide setup rows even with join/leave enabled", () => {
  const messages = [
    chatMessage("m1"),
    systemMessage("j1", "member_joined"),
    systemMessage("c1", "channel_created"),
  ];
  const visible = filterVisibleTimelineMessages(
    messages,
    channel({ name: "welcome-everyone" }),
    true,
  );
  assert.deepEqual(
    visible.map((m) => m.id),
    ["m1"],
  );
});
