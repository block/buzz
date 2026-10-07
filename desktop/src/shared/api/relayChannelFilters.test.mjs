import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChannelAuxDeletionFilter,
  buildChannelAuxFilter,
  buildChannelLiveFilter,
  buildChannelReactionAuxFilter,
  buildChannelStructuralAuxFilter,
  buildHuddleTtsLiveFilter,
} from "./relayChannelFilters.ts";
import {
  CHANNEL_EVENT_KINDS,
  KIND_CHANNEL_THREAD_SUMMARY,
} from "../constants/kinds.ts";
import {
  RECONNECT_REPLAY_CHANNEL_LOOKBACK_SECS,
  shouldPageReconnectReplay,
} from "./relayReconnectReplay.ts";
import { handleRelayClosed } from "./relayClosedRecovery.ts";

const CHANNEL = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const IDS = [
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
];

test("huddle TTS filter includes a bounded startup replay for both message kinds", () => {
  assert.deepEqual(buildHuddleTtsLiveFilter(CHANNEL, 1_725_100_000), {
    kinds: [9, 40002],
    "#h": [CHANNEL],
    since: 1_725_100_000,
    limit: 50,
  });
});

// Regression: reaction (kind:7) and reaction-removal (kind:5) events carry only
// an `e` tag, no channel `h` tag. An `#h`-scoped aux query never matches them,
// so removed historical reactions reappear. The aux filters must key on `#e`
// only.
test("buildChannelAuxFilter keys on #e only, no #h", () => {
  const filter = buildChannelAuxFilter(CHANNEL, IDS);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal("#h" in filter, false);
});

test("buildChannelAuxDeletionFilter keys on #e only, no #h", () => {
  const filter = buildChannelAuxDeletionFilter(CHANNEL, IDS);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal("#h" in filter, false);
});

test("buildChannelReactionAuxFilter fetches only kind:7 by #e", () => {
  const filter = buildChannelReactionAuxFilter(CHANNEL, IDS);
  assert.deepEqual(filter.kinds, [7]);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal("#h" in filter, false);
});

test("buildChannelStructuralAuxFilter excludes reactions", () => {
  const filter = buildChannelStructuralAuxFilter(CHANNEL, IDS);
  assert.deepEqual(filter.kinds, [5, 9005, 40003]);
  assert.deepEqual(filter["#e"], IDS);
  assert.equal("#h" in filter, false);
});

test("buildChannelLiveFilter is replay-bounded without a reader-clock since", () => {
  const filter = buildChannelLiveFilter(CHANNEL);

  assert.equal("since" in filter, false);
  assert.ok(filter.limit > 0);
  assert.equal(shouldPageReconnectReplay(filter), true);
  assert.deepEqual(filter["#h"], [CHANNEL]);
  assert.deepEqual(filter.kinds, [
    ...CHANNEL_EVENT_KINDS,
    KIND_CHANNEL_THREAD_SUMMARY,
  ]);
});

test("CLOSED retry pages a gap larger than the live limit from last-seen author time", async () => {
  const originalWindow = globalThis.window;
  let retry;
  globalThis.window = {
    setTimeout: (callback) => {
      retry = callback;
      return 1;
    },
    clearTimeout: () => {},
  };

  try {
    const liveFilter = buildChannelLiveFilter(CHANNEL);
    const recoveredEvents = Array.from(
      { length: liveFilter.limit + 1 },
      (_, index) => ({
        id: String(index).padStart(64, "0"),
        pubkey: "a".repeat(64),
        created_at: 10_001 + index,
        kind: 9,
        tags: [["h", CHANNEL]],
        content: `recovered ${index}`,
        sig: "b".repeat(128),
      }),
    );
    const delivered = [];
    const sentFilters = [];
    let flushes = 0;
    let repairRequest;
    let resolveReplay;
    const replayed = new Promise((resolve) => {
      resolveReplay = resolve;
    });
    const subscription = {
      mode: "live",
      filter: liveFilter,
      onEvent: (event) => delivered.push(event),
      onFlush: () => {
        flushes += 1;
      },
      lastSeenCreatedAt: 10_000,
    };
    const subscriptions = new Map([["live-closed", subscription]]);

    handleRelayClosed({
      subscriptions,
      subId: "live-closed",
      message: "error: temporary relay failure",
      sendReq: async (_subId, filter) => {
        sentFilters.push(filter);
      },
      requestRepair: async (request) => {
        repairRequest = request;
        resolveReplay();
        // The restored live REQ may already have delivered the newest row.
        return [recoveredEvents.at(-1), ...recoveredEvents];
      },
      generation: 7,
    });

    assert.equal(typeof retry, "function");
    retry();
    await replayed;
    await new Promise((resolve) => setImmediate(resolve));

    assert.equal(sentFilters.length, 1);
    assert.equal(sentFilters[0], liveFilter);
    assert.equal(repairRequest.channelId, CHANNEL);
    assert.equal(
      repairRequest.since,
      10_000 - RECONNECT_REPLAY_CHANNEL_LOOKBACK_SECS,
    );
    // No renderer-clock upper bound.
    assert.equal(repairRequest.until, undefined);
    assert.equal(repairRequest.limit, 500);
    assert.equal(delivered.length, liveFilter.limit + 1);
    assert.equal(flushes, 1);
    assert.equal(subscription.pendingReplaySince, undefined);
  } finally {
    globalThis.window = originalWindow;
  }
});
