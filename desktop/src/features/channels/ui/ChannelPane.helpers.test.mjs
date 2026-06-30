import assert from "node:assert/strict";
import test from "node:test";

import {
  canOpenAgentConversationInChannel,
  getDmTaskAgentPubkeys,
  getThreadTaskAgentPubkeys,
  mergeAutoRouteMentionPubkeys,
} from "./ChannelPane.helpers.ts";

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

test("auto-routed mentions merge with explicit mentions without duplicates", () => {
  assert.deepEqual(
    mergeAutoRouteMentionPubkeys({
      autoRouteAgentPubkeys: ["AGENT-ONE"],
      mentionPubkeys: ["agent-one", "agent-two"],
    }),
    ["AGENT-ONE", "agent-two"],
  );
});

test("DM task agent inference requires exactly one other known agent", () => {
  const knownAgentPubkeys = new Set(["agent-one", "agent-two"]);

  assert.deepEqual(
    getDmTaskAgentPubkeys({
      channel: channel({
        channelType: "dm",
        participantPubkeys: ["human", "agent-one"],
      }),
      currentPubkey: "human",
      knownAgentPubkeys,
    }),
    ["agent-one"],
  );

  assert.deepEqual(
    getDmTaskAgentPubkeys({
      channel: channel({
        channelType: "dm",
        participantPubkeys: ["human", "agent-one", "agent-two"],
      }),
      currentPubkey: "human",
      knownAgentPubkeys,
    }),
    [],
  );

  assert.deepEqual(
    getDmTaskAgentPubkeys({
      channel: channel({
        channelType: "dm",
        participantPubkeys: ["human", "agent-one", "human-two"],
      }),
      currentPubkey: "human",
      knownAgentPubkeys,
    }),
    [],
  );

  assert.deepEqual(
    getDmTaskAgentPubkeys({
      channel: channel({
        participantPubkeys: ["human", "agent-one"],
      }),
      currentPubkey: "human",
      knownAgentPubkeys,
    }),
    [],
  );
});

test("thread task agent inference requires exactly one known agent and one human", () => {
  const knownAgentPubkeys = new Set(["agent-one", "agent-two"]);

  assert.deepEqual(
    getThreadTaskAgentPubkeys({
      currentPubkey: "human",
      knownAgentPubkeys,
      messages: [
        {
          pubkey: "human",
          tags: [["p", "agent-one"]],
        },
        {
          pubkey: "agent-one",
          tags: [["p", "human"]],
        },
      ],
    }),
    ["agent-one"],
  );

  assert.deepEqual(
    getThreadTaskAgentPubkeys({
      currentPubkey: "human",
      knownAgentPubkeys,
      messages: [
        {
          pubkey: "human",
          tags: [["p", "agent-one"]],
        },
        {
          pubkey: "other-human",
          tags: [["p", "human"]],
        },
      ],
    }),
    [],
  );

  assert.deepEqual(
    getThreadTaskAgentPubkeys({
      currentPubkey: "human",
      knownAgentPubkeys,
      messages: [
        {
          pubkey: "human",
          tags: [["p", "agent-one"]],
        },
        {
          pubkey: "agent-two",
          tags: [["p", "human"]],
        },
      ],
    }),
    [],
  );
});
