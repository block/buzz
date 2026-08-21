import assert from "node:assert/strict";
import test from "node:test";

import {
  candidateFromEvent,
  candidatesFromInboxItems,
  collectChannelCandidates,
  mapWithConcurrency,
  mergeCandidates,
  PER_CHANNEL_CAP,
  sortOldestFirst,
  triageableChannels,
} from "./collectCandidates.ts";

const SELF = "aa".repeat(32);
const OTHER = "bb".repeat(32);

function event(overrides = {}) {
  return {
    id: "event-1",
    pubkey: OTHER,
    created_at: 1_000,
    kind: 40002,
    tags: [["h", "channel-1"]],
    content: "hello",
    sig: "sig",
    ...overrides,
  };
}

function channel(overrides = {}) {
  return {
    id: "channel-1",
    name: "general",
    channelType: "stream",
    isMember: true,
    archivedAt: null,
    ...overrides,
  };
}

test("candidateFromEvent flags a mention of the current user", () => {
  const candidate = candidateFromEvent(
    event({
      tags: [
        ["h", "channel-1"],
        ["p", SELF],
      ],
    }),
    channel(),
    { currentPubkey: SELF },
  );

  assert.equal(candidate.isMention, true);
  assert.equal(candidate.source, "channel");
  assert.equal(candidate.channelName, "general");
  assert.equal(candidate.isSelf, false);
});

test("candidateFromEvent marks the current user's own messages", () => {
  const candidate = candidateFromEvent(
    event({ pubkey: SELF, content: "I will look into it" }),
    channel(),
    { currentPubkey: SELF },
  );

  assert.equal(candidate.isSelf, true);
  assert.equal(candidate.authorPubkey, SELF);
});

test("candidateFromEvent does not read your own author tag as a mention", () => {
  const candidate = candidateFromEvent(
    event({
      pubkey: SELF,
      tags: [
        ["h", "channel-1"],
        ["p", SELF],
      ],
    }),
    channel(),
    { currentPubkey: SELF },
  );

  assert.equal(candidate.isSelf, true);
  assert.equal(candidate.isMention, false);
});

test("candidateFromEvent uses profile display names for authorLabel", () => {
  const candidate = candidateFromEvent(event(), channel(), {
    currentPubkey: SELF,
    profiles: {
      [OTHER]: {
        displayName: "Blake",
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
    },
  });

  assert.equal(candidate.authorLabel, "Blake");
});

test("candidateFromEvent does not flag a mention of somebody else", () => {
  const candidate = candidateFromEvent(
    event({
      tags: [
        ["h", "channel-1"],
        ["p", OTHER],
      ],
    }),
    channel(),
    { currentPubkey: SELF },
  );

  assert.equal(candidate.isMention, false);
});

test("candidateFromEvent derives the thread root and reply flag", () => {
  const candidate = candidateFromEvent(
    event({
      tags: [
        ["h", "channel-1"],
        ["e", "root-1", "", "root"],
        ["e", "parent-1", "", "reply"],
      ],
    }),
    channel(),
    { currentPubkey: SELF },
  );

  assert.equal(candidate.threadRootId, "root-1");
  assert.equal(candidate.isReply, true);
});

test("candidateFromEvent falls back to the h tag when no channel is known", () => {
  const candidate = candidateFromEvent(event(), undefined, {
    currentPubkey: SELF,
  });

  assert.equal(candidate.channelId, "channel-1");
  assert.equal(candidate.channelName, null);
});

test("candidatesFromInboxItems marks feed mentions and keeps the inbox source", () => {
  const candidates = candidatesFromInboxItems(
    [
      {
        conversationId: "root-9",
        id: "event-9",
        categories: ["mention"],
        item: {
          id: "event-9",
          kind: 40002,
          pubkey: OTHER,
          content: "ping",
          createdAt: 2_000,
          channelId: "channel-2",
          channelName: "design",
          channelType: "stream",
          tags: [],
        },
      },
    ],
    { currentPubkey: SELF },
  );

  assert.equal(candidates.length, 1);
  assert.equal(candidates[0].isMention, true);
  assert.equal(candidates[0].source, "inbox");
  assert.equal(candidates[0].threadRootId, "root-9");
  assert.equal(candidates[0].createdAt, 2_000);
});

test("candidatesFromInboxItems does not read your own message as a mention", () => {
  const candidates = candidatesFromInboxItems(
    [
      {
        conversationId: "root-9",
        id: "event-10",
        categories: ["mention"],
        item: {
          id: "event-10",
          kind: 40002,
          pubkey: SELF,
          content: "I will take this",
          createdAt: 2_000,
          channelId: "channel-2",
          channelName: "design",
          channelType: "stream",
          tags: [["p", SELF]],
        },
      },
    ],
    { currentPubkey: SELF },
  );

  assert.equal(candidates[0].isSelf, true);
  assert.equal(candidates[0].isMention, false);
});

test("mergeCandidates prefers the inbox copy of a duplicated event", () => {
  const merged = mergeCandidates(
    [
      {
        eventId: "dupe",
        createdAt: 5,
        source: "inbox",
        channelName: "general",
      },
    ],
    [{ eventId: "dupe", createdAt: 5, source: "channel", channelName: null }],
  );

  assert.equal(merged.length, 1);
  assert.equal(merged[0].source, "inbox");
  assert.equal(merged[0].channelName, "general");
});

test("mergeCandidates sorts newest first", () => {
  const merged = mergeCandidates(
    [{ eventId: "old", createdAt: 1, source: "inbox" }],
    [
      { eventId: "new", createdAt: 9, source: "channel" },
      { eventId: "mid", createdAt: 5, source: "channel" },
    ],
  );

  assert.deepEqual(
    merged.map((candidate) => candidate.eventId),
    ["new", "mid", "old"],
  );
});

test("sortOldestFirst is chronological", () => {
  const sorted = sortOldestFirst([
    { eventId: "new", createdAt: 9, source: "channel" },
    { eventId: "old", createdAt: 1, source: "inbox" },
    { eventId: "mid", createdAt: 5, source: "channel" },
  ]);
  assert.deepEqual(
    sorted.map((candidate) => candidate.eventId),
    ["old", "mid", "new"],
  );
});

test("triageableChannels drops non-member and archived channels", () => {
  const kept = triageableChannels([
    channel({ id: "keep" }),
    channel({ id: "not-member", isMember: false }),
    channel({ id: "archived", archivedAt: "2026-01-01T00:00:00Z" }),
  ]);

  assert.deepEqual(
    kept.map((entry) => entry.id),
    ["keep"],
  );
});

test("mapWithConcurrency preserves input order", async () => {
  const results = await mapWithConcurrency([3, 1, 2], 2, async (value) => {
    await new Promise((resolve) => setTimeout(resolve, value));
    return value * 10;
  });

  assert.deepEqual(results, [30, 10, 20]);
});

test("collectChannelCandidates includes self-authored messages and ignores read state", async () => {
  const filters = [];
  const candidates = await collectChannelCandidates({
    channels: [channel()],
    context: { currentPubkey: SELF },
    kindsForChannel: () => [40002],
    fetchEvents: async (filter) => {
      filters.push(filter);
      return [
        event({ id: "noise", created_at: 900, content: "lol" }),
        event({ id: "read", created_at: 500 }),
        event({
          id: "mine",
          created_at: 900,
          pubkey: SELF,
          content: "I will look into it",
        }),
        event({ id: "blank", created_at: 900, content: "   " }),
      ];
    },
  });

  assert.deepEqual(filters, [
    { kinds: [40002], "#h": ["channel-1"], since: 0, limit: PER_CHANNEL_CAP },
  ]);
  assert.deepEqual(candidates.map((candidate) => candidate.eventId).sort(), [
    "mine",
    "noise",
    "read",
  ]);
  assert.equal(
    candidates.find((candidate) => candidate.eventId === "mine")?.isSelf,
    true,
  );
});

test("collectChannelCandidates always queries from the beginning of history", async () => {
  const filters = [];
  await collectChannelCandidates({
    channels: [channel()],
    context: { currentPubkey: SELF },
    kindsForChannel: () => [40002],
    fetchEvents: async (filter) => {
      filters.push(filter);
      return [];
    },
  });

  assert.equal(filters[0].since, 0);
});

test("collectChannelCandidates isolates a failing channel", async () => {
  const candidates = await collectChannelCandidates({
    channels: [channel({ id: "broken" }), channel({ id: "channel-1" })],
    context: { currentPubkey: SELF },
    kindsForChannel: () => [40002],
    fetchEvents: async (filter) => {
      if (filter["#h"][0] === "broken") throw new Error("relay refused");
      return [event({ id: "survivor", created_at: 900 })];
    },
  });

  assert.deepEqual(
    candidates.map((candidate) => candidate.eventId),
    ["survivor"],
  );
});
