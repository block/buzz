import { strict as assert } from "node:assert";
import test from "node:test";

import { canStartHuddleInChannel } from "./huddleAvailability.ts";

const SELF = "a".repeat(64);
const OTHER = "b".repeat(64);

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

test("canStartHuddleInChannel allows DM participants", () => {
  assert.equal(
    canStartHuddleInChannel({
      channel: channel({
        channelType: "dm",
        visibility: "private",
        participantPubkeys: [OTHER, SELF],
        isMember: false,
      }),
      currentPubkey: SELF.toUpperCase(),
      selfMember: null,
    }),
    true,
  );
});

test("canStartHuddleInChannel allows DM membership when identity is not loaded yet", () => {
  assert.equal(
    canStartHuddleInChannel({
      channel: channel({
        channelType: "dm",
        visibility: "private",
        participantPubkeys: [OTHER, SELF],
        isMember: true,
      }),
      selfMember: null,
    }),
    true,
  );
});

test("canStartHuddleInChannel blocks non-participant DMs", () => {
  assert.equal(
    canStartHuddleInChannel({
      channel: channel({
        channelType: "dm",
        visibility: "private",
        participantPubkeys: [OTHER],
        isMember: false,
      }),
      currentPubkey: SELF,
      selfMember: null,
    }),
    false,
  );
});

test("canStartHuddleInChannel keeps private channels member-gated", () => {
  const privateChannel = channel({ visibility: "private" });

  assert.equal(
    canStartHuddleInChannel({
      channel: privateChannel,
      currentPubkey: SELF,
      selfMember: null,
    }),
    false,
  );

  assert.equal(
    canStartHuddleInChannel({
      channel: privateChannel,
      currentPubkey: SELF,
      selfMember: member(),
    }),
    true,
  );
});

test("canStartHuddleInChannel blocks archived channels and DMs", () => {
  assert.equal(
    canStartHuddleInChannel({
      channel: channel({
        archivedAt: "2026-01-01T00:00:00Z",
        visibility: "open",
      }),
      currentPubkey: SELF,
      selfMember: member(),
    }),
    false,
  );

  assert.equal(
    canStartHuddleInChannel({
      channel: channel({
        archivedAt: "2026-01-01T00:00:00Z",
        channelType: "dm",
        participantPubkeys: [SELF, OTHER],
      }),
      currentPubkey: SELF,
      selfMember: null,
    }),
    false,
  );
});
