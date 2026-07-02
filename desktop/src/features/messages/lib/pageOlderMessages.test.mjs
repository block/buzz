import assert from "node:assert/strict";
import test from "node:test";

import {
  maxEventIdAtSecond,
  pageOlderMessagesUntilRowFloor,
} from "./pageOlderMessages.ts";
import {
  channelMessagesKey,
  mergeNonContiguousTimelineMessages,
  mergeTimelineHistoryMessages,
  oldestContiguousHistoryTimestamp,
  sortMessages,
} from "./messageQueryKeys.ts";
import { relayClient } from "@/shared/api/relayClient";

const PUBKEY = "a".repeat(64);

function event({ id, kind = 9, createdAt, channelId, tags }) {
  return {
    id,
    pubkey: PUBKEY,
    created_at: createdAt,
    kind,
    tags: tags ?? [["h", channelId]],
    content: "",
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

function id(prefix, index) {
  return `${prefix}${String(index).padStart(64 - prefix.length, "0")}`;
}

function makeQueryClientStub(queryKey, initialEvents) {
  const store = new Map([[JSON.stringify(queryKey), initialEvents]]);
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

/**
 * Relay double: serves WS-style `until` pages over a fixed ascending dataset,
 * newest `limit` events strictly older than `before`.
 */
function serveHistoryFrom(dataset) {
  return async (_channelId, before, limit) =>
    dataset.filter((e) => e.created_at < before).slice(-limit);
}

test("ancestor island does not poison the older-history cursor (June 14 → June 9 skip)", async (t) => {
  const channelId = "island-cursor-regression";
  // Relay holds a contiguous channel history: 400 events spanning the "gap
  // days" (t=5000..5399), then the newest window (t=10000..10059).
  const gapDays = [];
  for (let index = 0; index < 400; index += 1) {
    gapDays.push(
      event({ id: id("gap", index), createdAt: 5_000 + index, channelId }),
    );
  }
  const newestWindow = [];
  for (let index = 0; index < 60; index += 1) {
    newestWindow.push(
      event({ id: id("new", index), createdAt: 10_000 + index, channelId }),
    );
  }
  const islandRoot = event({ id: id("old", 0), createdAt: 1_000, channelId });
  const relayDataset = sortMessages([...gapDays, ...newestWindow, islandRoot]);

  // Cache shape at the moment of the bug: contiguous newest window plus one
  // much older thread root injected out-of-band by useLoadMissingAncestors.
  const cache = mergeNonContiguousTimelineMessages(newestWindow, [islandRoot]);
  const queryKey = channelMessagesKey(channelId);
  const queryClient = makeQueryClientStub(queryKey, cache);

  const originalFetch = relayClient.fetchChannelHistoryBefore;
  relayClient.fetchChannelHistoryBefore = serveHistoryFrom(relayDataset);
  t.after(() => {
    relayClient.fetchChannelHistoryBefore = originalFetch;
  });

  await pageOlderMessagesUntilRowFloor(queryClient, channelId, () => true);

  const merged = queryClient.getQueryData(queryKey);
  const gapLoaded = merged.filter((e) => e.id.startsWith("gap")).length;
  // The pager must page from the contiguous frontier (t=10000) into the gap
  // days — not from the island (t=1000), which skips them forever.
  assert.ok(
    gapLoaded > 0,
    "pager anchored on the out-of-band island and skipped the gap days",
  );
});

test("contiguous paging heals the island: re-fetched copy loses the mark and anchors later passes", async (t) => {
  const channelId = "island-heal-regression";
  const older = [];
  for (let index = 0; index < 10; index += 1) {
    older.push(
      event({ id: id("old", index), createdAt: 1_000 + index, channelId }),
    );
  }
  const newestWindow = [];
  for (let index = 0; index < 60; index += 1) {
    newestWindow.push(
      event({ id: id("new", index), createdAt: 10_000 + index, channelId }),
    );
  }
  const relayDataset = sortMessages([...older, ...newestWindow]);

  // older[5] was injected out-of-band; the rest of `older` is unloaded.
  const cache = mergeNonContiguousTimelineMessages(newestWindow, [older[5]]);
  const queryKey = channelMessagesKey(channelId);
  const queryClient = makeQueryClientStub(queryKey, cache);

  const originalFetch = relayClient.fetchChannelHistoryBefore;
  relayClient.fetchChannelHistoryBefore = serveHistoryFrom(relayDataset);
  t.after(() => {
    relayClient.fetchChannelHistoryBefore = originalFetch;
  });

  await pageOlderMessagesUntilRowFloor(queryClient, channelId, () => true);

  const merged = queryClient.getQueryData(queryKey);
  assert.equal(merged.filter((e) => e.id.startsWith("old")).length, 10);
  const healed = merged.find((e) => e.id === older[5].id);
  assert.ok(
    !healed.nonContiguous,
    "contiguous re-fetch must clear the nonContiguous mark (last-copy-wins)",
  );
  // With the island healed, the frontier is now the true oldest event.
  assert.equal(oldestContiguousHistoryTimestamp(merged), 1_000);
});

test("frontier falls back to the oldest cached event when nothing is contiguous", async () => {
  const events = [
    {
      ...event({ id: id("iso", 0), createdAt: 500, channelId: "x" }),
      nonContiguous: true,
    },
  ];
  assert.equal(oldestContiguousHistoryTimestamp(events), null);
  // mergeTimelineHistoryMessages keeps the incoming (unflagged) copy on collision.
  const healed = mergeTimelineHistoryMessages(events, [
    event({ id: id("iso", 0), createdAt: 500, channelId: "x" }),
  ]);
  assert.equal(oldestContiguousHistoryTimestamp(healed), 500);
});

test("late island response never downgrades a contiguous copy (downgrade race)", () => {
  const channelId = "island-downgrade-race";
  // The event was missing when the ancestor fetch started, but a contiguous
  // history page fetched it (unmarked) before the island response landed.
  const contiguousCopy = event({
    id: id("race", 0),
    createdAt: 1_000,
    channelId,
  });
  const cache = mergeTimelineHistoryMessages(
    [],
    [contiguousCopy, event({ id: id("new", 0), createdAt: 10_000, channelId })],
  );

  // Late ancestor/thread response for the same id must be a no-op — marking
  // it here would re-poison the frontier that contiguous paging just fixed.
  const merged = mergeNonContiguousTimelineMessages(cache, [
    { ...contiguousCopy },
  ]);

  const kept = merged.find((e) => e.id === contiguousCopy.id);
  assert.ok(!kept.nonContiguous, "island merge downgraded a contiguous copy");
  assert.equal(oldestContiguousHistoryTimestamp(merged), 1_000);
  // And the reverse direction: thread replies already contiguous stay put.
  assert.equal(merged.length, cache.length);
});

test("dense-second seed ignores aux and island ids at the boundary second", () => {
  const channelId = "dense-second-seed";
  const second = 7_000;
  // Two content events already held at the stalled second...
  const contentA = event({ id: id("caa", 0), createdAt: second, channelId });
  const contentB = event({ id: id("cbb", 0), createdAt: second, channelId });
  // ...an unmarked reaction whose id sorts after both (aux kinds are absent
  // from the bridge keyset pages, so its id proves nothing about content)...
  const auxLater = event({
    id: id("zaux", 0),
    kind: 7,
    createdAt: second,
    channelId,
    tags: [
      ["h", channelId],
      ["e", contentA.id],
    ],
  });
  // ...and a marked island whose id sorts after everything.
  const islandLater = {
    ...event({ id: id("zisl", 0), createdAt: second, channelId }),
    nonContiguous: true,
  };

  const seed = maxEventIdAtSecond(
    [contentA, contentB, auxLater, islandLater],
    second,
  );

  // Seeding from the aux/island id would tell the relay "everything up to
  // this id is held" and skip unseen content rows in the same second.
  assert.equal(seed, contentB.id);
  // No contiguous content at the second -> no fabricated cursor.
  assert.equal(maxEventIdAtSecond([auxLater, islandLater], second), null);
});
