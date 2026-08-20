import assert from "node:assert/strict";
import test from "node:test";

import { buildHuddleChannelName } from "./huddleChannelName.ts";

const SELF = "a".repeat(64);
const OTHER = "b".repeat(64);
const THIRD = "c".repeat(64);

function channel(overrides = {}) {
  return {
    id: "channel-id",
    name: "general",
    channelType: "stream",
    visibility: "private",
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

function member(overrides = {}) {
  return {
    pubkey: SELF,
    role: "member",
    isAgent: false,
    joinedAt: "2026-01-01T00:00:00Z",
    displayName: null,
    ...overrides,
  };
}

test("buildHuddleChannelName names one-on-one DMs with current user first", () => {
  assert.equal(
    buildHuddleChannelName({
      channel: channel({
        channelType: "dm",
        participants: ["Tyler", "Kenny"],
        participantPubkeys: [OTHER, SELF],
      }),
      currentPubkey: SELF,
      members: [
        member({ pubkey: SELF, displayName: "Kenny Lopez" }),
        member({ pubkey: OTHER, displayName: "Tyler Durden" }),
      ],
    }),
    "Kenny <> Tyler huddle",
  );
});

test("buildHuddleChannelName falls back to channel participant names", () => {
  assert.equal(
    buildHuddleChannelName({
      channel: channel({
        channelType: "dm",
        participants: ["Other Person", "Self User"],
        participantPubkeys: [OTHER, SELF],
      }),
      currentPubkey: SELF,
    }),
    "Self <> Other huddle",
  );
});

test("buildHuddleChannelName keeps group DM participants readable", () => {
  assert.equal(
    buildHuddleChannelName({
      channel: channel({
        channelType: "dm",
        participants: ["Other", "Self", "Third"],
        participantPubkeys: [OTHER, SELF, THIRD],
      }),
      currentPubkey: SELF,
    }),
    "Self <> Other <> Third huddle",
  );
});

test("buildHuddleChannelName names stream huddles after the channel", () => {
  assert.equal(
    buildHuddleChannelName({
      channel: channel({ name: "engineering" }),
      currentPubkey: SELF,
    }),
    "engineering huddle",
  );
});
