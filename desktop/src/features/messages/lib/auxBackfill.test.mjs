import assert from "node:assert/strict";
import test from "node:test";

import {
  collectAuxEventIdsForDeletionBackfill,
  collectMessageIdsForAuxBackfill,
  mergeAuxEventsWithDeletionBackfill,
} from "./auxBackfill.ts";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";

function event(id, kind, overrides = {}) {
  return {
    id,
    pubkey: "a".repeat(64),
    kind,
    created_at: 1_700_000_000,
    content: "",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
    ...overrides,
  };
}

function hex(char) {
  return char.repeat(64);
}

test("collects content-kind message ids (stream, v2, diff, system, jobs)", () => {
  const events = [
    event(hex("1"), 9), // stream message
    event(hex("2"), 40002), // v2 stream message
    event(hex("3"), 40008), // diff (own row)
    event(hex("4"), 40099), // system message
    event(hex("5"), 43001), // job request
  ];
  assert.deepEqual(collectMessageIdsForAuxBackfill(events), [
    hex("1"),
    hex("2"),
    hex("3"),
    hex("4"),
    hex("5"),
  ]);
});

test("excludes auxiliary kinds (reactions, edits, deletions)", () => {
  const events = [
    event(hex("1"), 9), // message — kept
    event(hex("2"), 7), // reaction — excluded
    event(hex("3"), 40003), // edit — excluded
    event(hex("4"), 5), // NIP-09 deletion — excluded
    event(hex("5"), 9005), // Buzz-native deletion — excluded
  ];
  assert.deepEqual(collectMessageIdsForAuxBackfill(events), [hex("1")]);
});

test("returns empty for a window of only auxiliary events", () => {
  const events = [event(hex("2"), 7), event(hex("3"), 40003)];
  assert.deepEqual(collectMessageIdsForAuxBackfill(events), []);
});

test("collects reaction and edit ids for deletion-marker backfill", () => {
  const events = [
    event(hex("1"), 9),
    event(hex("2"), 7),
    event(hex("3"), 40003),
    event(hex("4"), 5),
    event(hex("5"), 9005),
  ];

  assert.deepEqual(collectAuxEventIdsForDeletionBackfill(events), [
    hex("2"),
    hex("3"),
  ]);
});

test("merges deletion markers that target cached or fetched auxiliary event ids", async () => {
  const messageId = hex("1");
  const cachedReactionId = hex("2");
  const fetchedReactionId = hex("3");
  const cachedReactionDeletionId = hex("4");
  const fetchedReactionDeletionId = hex("5");
  const cachedReaction = event(cachedReactionId, 7, {
    content: "+",
    tags: [
      ["h", CHANNEL_ID],
      ["e", messageId],
    ],
  });
  const fetchedReaction = event(fetchedReactionId, 7, {
    content: "-",
    tags: [
      ["h", CHANNEL_ID],
      ["e", messageId],
    ],
  });
  const cachedReactionDeletion = event(cachedReactionDeletionId, 5, {
    tags: [
      ["h", CHANNEL_ID],
      ["e", cachedReactionId],
    ],
  });
  const fetchedReactionDeletion = event(fetchedReactionDeletionId, 5, {
    tags: [
      ["h", CHANNEL_ID],
      ["e", fetchedReactionId],
    ],
  });
  const calls = [];

  const merged = await mergeAuxEventsWithDeletionBackfill({
    channelId: CHANNEL_ID,
    cachedEvents: [cachedReaction],
    fetchedAuxEvents: [fetchedReaction],
    fetchAuxEventsForMessages: async (channelId, ids) => {
      calls.push({ channelId, ids });
      return [cachedReactionDeletion, fetchedReactionDeletion];
    },
  });

  assert.deepEqual(calls, [
    { channelId: CHANNEL_ID, ids: [cachedReactionId, fetchedReactionId] },
  ]);
  assert.deepEqual(
    merged.map((cachedEvent) => cachedEvent.id),
    [fetchedReactionId, cachedReactionDeletionId, fetchedReactionDeletionId],
  );
});
