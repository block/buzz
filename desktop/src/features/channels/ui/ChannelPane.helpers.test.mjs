import assert from "node:assert/strict";
import test from "node:test";

import { canOpenAgentConversationInChannel } from "./ChannelPane.helpers.ts";

function channel(overrides = {}) {
  return {
    id: "channel",
    name: "Channel",
    channelType: "stream",
    visibility: "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

test("new agent conversations require a writable channel", () => {
  assert.equal(
    canOpenAgentConversationInChannel({
      channel: channel(),
    }),
    true,
  );
  assert.equal(
    canOpenAgentConversationInChannel({
      channel: channel({ archivedAt: "2026-06-27T00:00:00.000Z" }),
    }),
    false,
  );
  assert.equal(
    canOpenAgentConversationInChannel({
      channel: channel({ isMember: false }),
    }),
    false,
  );
});

test("existing agent conversation markers can open in read-only channels", () => {
  assert.equal(
    canOpenAgentConversationInChannel({
      channel: channel({ archivedAt: "2026-06-27T00:00:00.000Z" }),
      publishMarker: false,
    }),
    true,
  );
  assert.equal(
    canOpenAgentConversationInChannel({
      channel: channel({ isMember: false }),
      publishMarker: false,
    }),
    true,
  );
});
