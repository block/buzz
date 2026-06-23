import assert from "node:assert/strict";
import test from "node:test";

import {
  backfillAuxForMessages,
  collectAuxEventIdsForDeletionBackfill,
  collectMessageIdsForAuxBackfill,
  mergeAuxEventsWithDeletionBackfill,
} from "./auxBackfill.ts";
import { formatTimelineMessages } from "./formatTimelineMessages.ts";
import { buildChannelReactionAuxFilter } from "@/shared/api/relayChannelFilters.ts";
import { channelMessagesKey } from "./messageQueryKeys.ts";

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

// Regression for the "duplicate: reaction already exists" report: a reaction
// older than the content-kinds history window never rendered, because the old
// single-filter history fetch (all CHANNEL_EVENT_KINDS under one `limit`) let
// reactions/deletions evict older reactions from the window. The fix (#1153)
// fetches history with content kinds only and backfills reactions by `#e`
// reference over the loaded message ids — recency-independent. This test pins
// that path end-to-end: a loaded message whose only reaction is NOT in the
// history window still renders that reaction after the `#e` backfill.
//
// Would fail pre-#1153: the reaction was never fetched, so the message
// rendered with no reactions and re-reacting hit the relay's duplicate guard.
test("old reaction outside the history window is backfilled by #e and renders", async () => {
  const messageId = hex("1");
  const reactionId = hex("2");
  const currentUser = hex("c");

  // The cold-load history window: the message, but NOT its (older) reaction.
  const history = [
    event(messageId, 9, {
      pubkey: hex("a"),
      content: "ship it?",
      created_at: 1_700_001_000,
    }),
  ];

  // Step 1: backfill keys off the loaded content message ids.
  const messageIds = collectMessageIdsForAuxBackfill(history);
  assert.deepEqual(messageIds, [messageId]);

  // Step 2: the reaction aux filter references those ids by `#e`, with no time
  // window — so an old reaction is reachable regardless of when it was created.
  const auxFilter = buildChannelReactionAuxFilter(CHANNEL_ID, messageIds);
  assert.deepEqual(auxFilter["#e"], [messageId]);
  assert.equal("since" in auxFilter, false);
  assert.equal("until" in auxFilter, false);

  // Step 3: the relay returns the old reaction for that `#e` filter. Its
  // created_at predates the loaded message — the exact case the old window
  // dropped.
  const oldReaction = event(reactionId, 7, {
    pubkey: currentUser,
    content: "✅",
    created_at: 1_700_000_000,
    tags: [["e", messageId]],
  });

  const merged = await mergeAuxEventsWithDeletionBackfill({
    channelId: CHANNEL_ID,
    cachedEvents: history,
    fetchedAuxEvents: [oldReaction],
    // No deletions target the reaction.
    fetchAuxEventsForMessages: async () => [],
  });
  assert.deepEqual(
    merged.map((e) => e.id),
    [reactionId],
  );

  // Step 4: the reaction merged into the timeline renders on its target
  // message, attributed to the current user (so the UI shows it as already
  // reacted and won't prompt a duplicate add).
  const timeline = formatTimelineMessages(
    [...history, ...merged],
    null,
    currentUser,
    null,
  );
  const row = timeline.find((m) => m.id === messageId);
  assert.ok(row, "loaded message should render a timeline row");
  assert.deepEqual(
    row.reactions?.map((r) => ({
      emoji: r.emoji,
      count: r.count,
      mine: r.reactedByCurrentUser,
    })),
    [{ emoji: "✅", count: 1, mine: true }],
  );
});

// Minimal in-memory stand-in for the React-Query client: only the two methods
// backfillAuxForMessages touches. `setQueryData` mirrors React-Query's updater
// contract (receives current value, defaulted to [] by the caller).
function makeQueryClientStub() {
  const store = new Map();
  return {
    getQueryData(key) {
      return store.get(JSON.stringify(key));
    },
    setQueryData(key, updater) {
      const k = JSON.stringify(key);
      const next =
        typeof updater === "function" ? updater(store.get(k) ?? []) : updater;
      store.set(k, next);
      return next;
    },
  };
}

function reactionAux(id, messageId, emoji = "✅") {
  return event(id, 7, {
    pubkey: hex("c"),
    content: emoji,
    tags: [["e", messageId]],
  });
}

// The whole point of the kind-split: a slow/failed structural (kind:5/9005/
// 40003) fetch must NOT blank reactions. Reactions are committed first on their
// own REQ; the structural overlay's failure is caught and logged, leaving the
// reactions in cache. Pre-fix, both rode one bundled REQ under one try/catch,
// so a structural timeout dropped every reaction in the view.
test("structural-overlay failure does not strand already-committed reactions", async () => {
  const messageId = hex("1");
  const queryClient = makeQueryClientStub();
  // Seed the content message into cache, as the cold-load history fetch would.
  queryClient.setQueryData(channelMessagesKey(CHANNEL_ID), [
    event(messageId, 9, { pubkey: hex("a"), content: "ship it?" }),
  ]);

  await backfillAuxForMessages(queryClient, CHANNEL_ID, [event(messageId, 9)], {
    fetchReactionAuxEventsForMessages: async () => [
      reactionAux(hex("2"), messageId),
    ],
    // The slow half blows up — exactly the cold-load kind:5 timeout.
    fetchStructuralAuxEventsForMessages: async () => {
      throw new Error("Timed out while loading channel history.");
    },
    fetchAuxDeletionEventsForAuxEvents: async () => [],
  });

  const cached = queryClient.getQueryData(channelMessagesKey(CHANNEL_ID));
  assert.ok(
    cached.some((e) => e.id === hex("2") && e.kind === 7),
    "reaction must survive a structural-overlay fetch failure",
  );
});

// Symmetric guarantee: a reaction-fetch failure must not abort the structural
// overlay. Each half owns its try/catch, so an edit/deletion still applies even
// if reactions couldn't be fetched this pass (they self-heal next backfill).
test("reaction-fetch failure does not block the structural overlay", async () => {
  const messageId = hex("1");
  const editId = hex("3");
  const queryClient = makeQueryClientStub();
  queryClient.setQueryData(channelMessagesKey(CHANNEL_ID), [
    event(messageId, 9, { pubkey: hex("a"), content: "original" }),
  ]);

  await backfillAuxForMessages(queryClient, CHANNEL_ID, [event(messageId, 9)], {
    fetchReactionAuxEventsForMessages: async () => {
      throw new Error("Timed out while loading channel history.");
    },
    fetchStructuralAuxEventsForMessages: async () => [
      event(editId, 40003, { tags: [["e", messageId]] }),
    ],
    fetchAuxDeletionEventsForAuxEvents: async () => [],
  });

  const cached = queryClient.getQueryData(channelMessagesKey(CHANNEL_ID));
  assert.ok(
    cached.some((e) => e.id === editId && e.kind === 40003),
    "edit must apply even when the reaction fetch failed",
  );
});
