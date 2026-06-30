import assert from "node:assert/strict";
import test from "node:test";

import {
  canOpenAgentConversationInChannel,
  getDmAutoRouteAgentPubkeys,
  getThreadAutoRouteAgentPubkeys,
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

function message(overrides = {}) {
  return {
    id: "message",
    createdAt: 1,
    pubkey: "human",
    author: "Human",
    avatarUrl: null,
    role: undefined,
    personaDisplayName: undefined,
    time: "1:00 PM",
    body: "Body",
    parentId: null,
    rootId: null,
    depth: 0,
    accent: false,
    pending: undefined,
    edited: false,
    kind: 9,
    tags: [],
    reactions: undefined,
    ...overrides,
  };
}

test("DM composer auto-routes only when exactly one other participant is an agent", () => {
  const knownAgentPubkeys = new Set(["agent-one", "agent-two"]);

  assert.deepEqual(
    getDmAutoRouteAgentPubkeys({
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
    getDmAutoRouteAgentPubkeys({
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
    getDmAutoRouteAgentPubkeys({
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
    getDmAutoRouteAgentPubkeys({
      channel: channel({
        participantPubkeys: ["human", "agent-one"],
      }),
      currentPubkey: "human",
      knownAgentPubkeys,
    }),
    [],
  );
});

test("thread composer auto-routes only for one human and one known agent", () => {
  const knownAgentPubkeys = new Set(["agent-one", "agent-two"]);

  assert.deepEqual(
    getThreadAutoRouteAgentPubkeys({
      knownAgentPubkeys,
      messages: [
        message({ id: "root", tags: [["p", "agent-one"]] }),
        message({ id: "agent-reply", pubkey: "agent-one" }),
      ],
    }),
    ["agent-one"],
  );

  assert.deepEqual(
    getThreadAutoRouteAgentPubkeys({
      knownAgentPubkeys,
      messages: [
        message({
          id: "root",
          pubkey: "human-one",
          tags: [
            ["p", "human-one"],
            ["p", "agent-one"],
          ],
        }),
        message({
          id: "human-two-reply",
          pubkey: "human-two",
          tags: [
            ["p", "human-two"],
            ["p", "agent-one"],
          ],
        }),
        message({ id: "agent-reply", pubkey: "agent-one" }),
      ],
    }),
    [],
  );

  assert.deepEqual(
    getThreadAutoRouteAgentPubkeys({
      knownAgentPubkeys,
      messages: [
        message({ id: "agent-one-reply", pubkey: "agent-one" }),
        message({ id: "agent-two-reply", pubkey: "agent-two" }),
      ],
    }),
    [],
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
