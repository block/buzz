import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  buildReconnectReplayFilter,
  DB_CREATED_AT_FLOOR_SECS,
  FENCE_CLOCK_MARGIN_SECS,
  RELAY_INGEST_FUTURE_TOLERANCE_SECS,
  RECONNECT_REPLAY_CHANNEL_LOOKBACK_SECS,
  replayReconnectHistoryPages,
  PAGE_REPLAY_MAX_ATTEMPTS,
  replayLiveSubscriptions,
  REPLAY_BATCH_SIZE,
  shouldPageReconnectReplay,
} from "./relayReconnectReplay.ts";
import { buildChannelFilter } from "./relayChannelFilters.ts";
import {
  flushEvents,
  markReconnectLiveEose,
  markReconnectRepairDone,
  prepareSubscriptionEvent,
  shouldDispatchSubscriptionEvent,
} from "./relayClosedRecovery.ts";

// ── Fake-timer + Date.now setup for gate tests ────────────────────────────────

let fakeNow = 0;
const pendingTimers = new Map();
let nextTimerId = 1;

function fakeSetTimeout(fn, ms) {
  const id = nextTimerId++;
  pendingTimers.set(id, { fn, fireAt: fakeNow + ms });
  return id;
}

function fakeClearTimeout(id) {
  pendingTimers.delete(id);
}

function tickTo(ms) {
  fakeNow = ms;
  for (const [id, { fn, fireAt }] of Array.from(pendingTimers.entries())) {
    if (fireAt <= fakeNow) {
      pendingTimers.delete(id);
      fn();
    }
  }
}

globalThis.window = {
  setTimeout: fakeSetTimeout,
  clearTimeout: fakeClearTimeout,
};

const origDateNow = Date.now;
function setFakeNow(ms) {
  fakeNow = ms;
  Date.now = () => fakeNow;
}

// Import gate module AFTER window shim so it picks up the fake timers.
const { activateRateLimit, resetRateLimitGate } = await import(
  "./relayRateLimitGate.ts"
);

function resetGate(startMs = 0) {
  pendingTimers.clear();
  nextTimerId = 1;
  setFakeNow(startMs);
  resetRateLimitGate();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function event(id, createdAt) {
  return {
    id,
    pubkey: "pubkey",
    created_at: createdAt,
    kind: 9,
    tags: [],
    content: "",
    sig: "sig",
  };
}

function eventRange(prefix, start, count) {
  return Array.from({ length: count }, (_, index) =>
    event(`${prefix}-${index}`, start + index),
  ).reverse();
}

function replayFilter(filter, since, until) {
  return buildReconnectReplayFilter(filter, since, until);
}

function numericList(source, pattern) {
  const match = source.match(pattern);
  assert.ok(match, `expected source list to match ${pattern}`);
  return [...match[1].matchAll(/\b\d[\d_]*\b/g)].map((value) =>
    Number(value[0].replaceAll("_", "")),
  );
}

test("channel replay lookback stays coupled to relay and DB source constants", async () => {
  const [ingest, fence] = await Promise.all([
    readFile("../crates/buzz-relay/src/handlers/ingest.rs", "utf8"),
    readFile("../crates/buzz-db/src/runtime/replica_fence.rs", "utf8"),
  ]);
  assert.match(
    ingest,
    new RegExp(
      `MAX_TIMESTAMP_DRIFT_SECS: i64 = ${RELAY_INGEST_FUTURE_TOLERANCE_SECS}`,
    ),
  );
  assert.match(
    fence,
    new RegExp(`CREATED_AT_FLOOR_SECS: i64 = ${DB_CREATED_AT_FLOOR_SECS}`),
  );
  assert.match(
    fence,
    new RegExp(`FENCE_CLOCK_MARGIN_SECS: i64 = ${FENCE_CLOCK_MARGIN_SECS}`),
  );
  assert.equal(RECONNECT_REPLAY_CHANNEL_LOOKBACK_SECS, 1_865);
});

test("native and E2E repair kinds stay coupled to the live channel filter", async () => {
  const [rustCommand, e2eBridge] = await Promise.all([
    readFile("src-tauri/src/commands/channel_reconnect_repair.rs", "utf8"),
    readFile("src/testing/e2eBridge.ts", "utf8"),
  ]);
  const expectedKinds = [...buildChannelFilter("channel-1", 50).kinds].sort(
    (left, right) => left - right,
  );
  assert.deepEqual(
    numericList(
      rustCommand,
      /const CHANNEL_REPAIR_KINDS: \[u32; \d+\] = \[([\s\S]*?)\];/,
    ).sort((left, right) => left - right),
    expectedKinds,
  );
  assert.deepEqual(
    numericList(
      e2eBridge,
      /function handleGetChannelReconnectRepair[\s\S]*?const kinds = new Set\(\[([\s\S]*?)\]\);/,
    ).sort((left, right) => left - right),
    expectedKinds,
  );
});

// ── buildReconnectReplayFilter ────────────────────────────────────────────────

test("reconnect replay preserves small steady-state limits when adding since", () => {
  const filter = {
    kinds: [9, 40002],
    "#h": ["channel-1"],
    limit: 50,
  };

  assert.deepEqual(replayFilter(filter, 123), {
    kinds: [9, 40002],
    "#h": ["channel-1"],
    limit: 50,
    since: 123,
  });
});

test("reconnect replay caps large steady-state limits", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 1000,
  };

  assert.deepEqual(replayFilter(filter, 123), {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 500,
    since: 123,
  });
});

test("reconnect replay preserves the live-only zero-history contract", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 0,
  };

  assert.deepEqual(replayFilter(filter, 123), {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 0,
    since: 123,
  });
});

test("live-only subscriptions do not page reconnect history", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 0,
  };

  assert.equal(shouldPageReconnectReplay(filter), false);
});

test("reconnect replay keeps the stricter existing since window", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
    since: 200,
  };

  assert.deepEqual(replayFilter(filter, 123), {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
    since: 200,
  });
});

test("reconnect replay applies the stricter until window", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
    until: 300,
  };

  assert.deepEqual(replayFilter(filter, 123, 400), {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
    since: 123,
    until: 300,
  });
});

test("initial subscription replay preserves the original filter", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
  };

  assert.equal(replayFilter(filter, undefined), filter);
});

// ── Batching: REPLAY_BATCH_SIZE cap ──────────────────────────────────────────

test("replay sends all subs in one batch when count equals REPLAY_BATCH_SIZE", async () => {
  resetGate();
  let delayCount = 0;
  const sentIds = [];

  const subscriptions = new Map(
    Array.from({ length: REPLAY_BATCH_SIZE }, (_, i) => [
      `sub-${i}`,
      {
        mode: "live",
        filter: { kinds: [9], "#h": [`ch-${i}`], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ]),
  );

  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async (payload) => {
      sentIds.push(payload[1]);
    },
    requestRepair: async () => [],
    setTimeoutFn: (fn, _ms) => {
      delayCount++;
      fn();
      return 0;
    },
  });

  assert.equal(sentIds.length, REPLAY_BATCH_SIZE);
  assert.equal(delayCount, 0, "no inter-batch delay for exactly one batch");
});

test("replay splits subscriptions into batches of REPLAY_BATCH_SIZE", async () => {
  resetGate();
  let delayCount = 0;
  const sentIds = [];
  const batchBreakpoints = []; // indices where a delay fired

  const subCount = REPLAY_BATCH_SIZE + 3;
  const subscriptions = new Map(
    Array.from({ length: subCount }, (_, i) => [
      `sub-${i}`,
      {
        mode: "live",
        filter: { kinds: [9], "#h": [`ch-${i}`], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ]),
  );

  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async (payload) => {
      sentIds.push(payload[1]);
    },
    requestRepair: async () => [],
    setTimeoutFn: (fn, _ms) => {
      delayCount++;
      batchBreakpoints.push(sentIds.length);
      fn();
      return 0;
    },
  });

  assert.equal(delayCount, 1, "one inter-batch delay between two batches");
  assert.equal(sentIds.length, subCount, "all subs sent");
  // The delay fired after the first batch (REPLAY_BATCH_SIZE subs sent).
  assert.equal(batchBreakpoints[0], REPLAY_BATCH_SIZE);
});

test("inter-batch disposal cannot resurrect a closed subscription", async () => {
  resetGate();
  const frames = [];
  const secondSubscription = {
    mode: "live",
    filter: { kinds: [9], "#h": ["ch-2"], limit: 50 },
    onEvent: () => {},
    lastSeenCreatedAt: undefined,
  };
  const subscriptions = new Map([
    [
      "sub-1",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ],
    ["sub-2", secondSubscription],
  ]);

  await replayLiveSubscriptions({
    subscriptions,
    replayBatchSize: 1,
    sendRaw: async (payload) => {
      frames.push(`${payload[0]}:${payload[1]}`);
    },
    requestRepair: async () => [],
    setTimeoutFn: (fn, _ms) => {
      assert.equal(subscriptions.delete("sub-2"), true);
      frames.push("CLOSE:sub-2");
      fn();
      return 0;
    },
  });

  assert.deepEqual(frames, ["REQ:sub-1", "CLOSE:sub-2"]);
});

test("in-flight disposal is closed again after the stale REQ settles", async () => {
  resetGate();
  const frames = [];
  let markReqStarted;
  const reqStarted = new Promise((resolve) => {
    markReqStarted = resolve;
  });
  let releaseReq;
  const heldReq = new Promise((resolve) => {
    releaseReq = resolve;
  });
  const subscription = {
    mode: "live",
    filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
    onEvent: () => {},
    lastSeenCreatedAt: undefined,
  };
  const subscriptions = new Map([["sub-1", subscription]]);

  const replayPromise = replayLiveSubscriptions({
    subscriptions,
    sendRaw: async (payload) => {
      frames.push(`${payload[0]}:${payload[1]}`);
      if (payload[0] === "REQ") {
        markReqStarted();
        await heldReq;
      }
    },
    requestRepair: async () => [],
  });

  await reqStarted;
  assert.equal(subscriptions.delete("sub-1"), true);
  // The ordinary disposer emits CLOSE while the websocket REQ invoke is held.
  frames.push("CLOSE:sub-1");
  releaseReq();
  await replayPromise;

  assert.deepEqual(frames, ["REQ:sub-1", "CLOSE:sub-1", "CLOSE:sub-1"]);
});

test("inter-batch replacement cannot be overwritten by a stale REQ", async () => {
  resetGate();
  const sentFilters = [];
  const subscriptions = new Map([
    [
      "sub-1",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ],
    [
      "sub-2",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-old"], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ],
  ]);

  await replayLiveSubscriptions({
    subscriptions,
    replayBatchSize: 1,
    sendRaw: async (payload) => {
      sentFilters.push(payload[2]);
    },
    requestRepair: async () => [],
    setTimeoutFn: (fn, _ms) => {
      subscriptions.set("sub-2", {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-new"], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      });
      fn();
      return 0;
    },
  });

  assert.deepEqual(sentFilters, [{ kinds: [9], "#h": ["ch-1"], limit: 50 }]);
});

// ── Visible-channel priority ──────────────────────────────────────────────────

test("visible channel subscription is sent first", async () => {
  resetGate();
  const sentOrder = [];

  const subscriptions = new Map([
    [
      "other-1",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-other"], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ],
    [
      "visible-sub",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-visible"], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ],
    [
      "other-2",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-other2"], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ],
  ]);

  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async (payload) => {
      sentOrder.push(payload[1]);
    },
    requestRepair: async () => [],
    visibleChannelId: "ch-visible",
  });

  assert.equal(sentOrder[0], "visible-sub", "visible sub sent first");
  assert.equal(sentOrder.length, 3);
});

// ── Rate-limit gate deferral ──────────────────────────────────────────────────

test("replay waits for rate-limit gate before sending REQs", async () => {
  resetGate(0);
  activateRateLimit(5); // gate active for 5 seconds

  const sentIds = [];

  const replayPromise = replayLiveSubscriptions({
    subscriptions: new Map([
      [
        "sub-1",
        {
          mode: "live",
          filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
          onEvent: () => {},
          lastSeenCreatedAt: undefined,
        },
      ],
    ]),
    sendRaw: async (payload) => {
      sentIds.push(payload[1]);
    },
    requestRepair: async () => [],
    setTimeoutFn: (fn, _ms) => {
      fn();
      return 0;
    },
  });

  // Gate expires — replay should proceed now.
  tickTo(5_001);

  await replayPromise;

  assert.equal(sentIds.length, 1, "REQ sent after gate expired");
});

// ── Connection-generation guard ───────────────────────────────────────────────

test("stale replay sends no REQs when generation advances while gate was active", async () => {
  resetGate(0);
  activateRateLimit(5); // gate active for 5 seconds

  let generationActive = true; // true = current, false = stale
  const sentIds = [];

  const replayPromise = replayLiveSubscriptions({
    subscriptions: new Map([
      [
        "sub-1",
        {
          mode: "live",
          filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
          onEvent: () => {},
          lastSeenCreatedAt: undefined,
        },
      ],
    ]),
    sendRaw: async (payload) => {
      sentIds.push(payload[1]);
    },
    requestRepair: async () => [],
    isActive: () => generationActive,
  });

  // Advance the generation (simulate new connection) before the gate resolves.
  generationActive = false;

  // Fire the gate timer.
  tickTo(5_001);

  await replayPromise;

  assert.equal(sentIds.length, 0, "no REQs sent for a stale replay");
});

// ── Paged replay (existing behaviour) ────────────────────────────────────────

test("channel reconnect replay pages the missed window until a short page", async () => {
  resetGate();
  const delivered = [];
  const historyFilters = [];
  const sentPayloads = [];
  const pages = [
    eventRange("newest", 1501, 500),
    eventRange("middle", 1002, 500),
    eventRange("oldest", 995, 8),
  ];
  const filter = buildChannelFilter("channel-1", 50);
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter,
        onEvent: (event) => delivered.push(event),
        lastSeenCreatedAt: 1000,
      },
    ],
  ]);

  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async (payload) => {
      sentPayloads.push(payload);
    },
    requestRepair: async (request) => {
      historyFilters.push(request);
      return pages.shift() ?? [];
    },
  });

  assert.deepEqual(sentPayloads, [
    [
      "REQ",
      "live-1",
      {
        kinds: filter.kinds,
        "#h": ["channel-1"],
        limit: 50,
      },
    ],
  ]);
  assert.deepEqual(historyFilters, [
    {
      channelId: "channel-1",
      limit: 500,
      since: 0,
      until: undefined,
      beforeId: undefined,
    },
    {
      channelId: "channel-1",
      limit: 500,
      since: 0,
      until: 1501,
      beforeId: "newest-0",
    },
    {
      channelId: "channel-1",
      limit: 500,
      since: 0,
      until: 1002,
      beforeId: "middle-0",
    },
  ]);
  assert.equal(delivered.length, 1008);
});

test("native repair preserves an explicit subscription since boundary", async () => {
  resetGate();
  const repairRequests = [];
  const filter = {
    ...buildChannelFilter("channel-1", 1000),
    since: 10_000,
  };
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter,
        onEvent: () => {},
        // The repair lookback would otherwise cross the from-now boundary.
        lastSeenCreatedAt: 11_000,
      },
    ],
  ]);

  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async () => {},
    requestRepair: async (request) => {
      repairRequests.push(request);
      return [];
    },
  });

  assert.equal(11_000 - RECONNECT_REPLAY_CHANNEL_LOOKBACK_SECS, 9_135);
  assert.deepEqual(repairRequests, [
    {
      channelId: "channel-1",
      limit: 500,
      since: 10_000,
      until: undefined,
      beforeId: undefined,
    },
  ]);
});

test("reconnect replay starts live REQs in parallel and preserves per-sub page order", async () => {
  resetGate();
  const sentPayloads = [];
  const sendResolvers = [];
  const historyFiltersByChannel = {
    "channel-1": [],
    "channel-2": [],
  };
  const pagesByChannel = {
    "channel-1": [
      eventRange("c1-full", 1501, 500),
      eventRange("c1-short", 1490, 2),
    ],
    "channel-2": [
      eventRange("c2-full", 1701, 500),
      eventRange("c2-short", 1690, 2),
    ],
  };
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter: buildChannelFilter("channel-1", 50),
        onEvent: () => {},
        lastSeenCreatedAt: 1000,
      },
    ],
    [
      "live-2",
      {
        mode: "live",
        filter: buildChannelFilter("channel-2", 50),
        onEvent: () => {},
        lastSeenCreatedAt: 1000,
      },
    ],
  ]);

  const replayPromise = replayLiveSubscriptions({
    subscriptions,
    pageReplayConcurrency: 2,
    sendRaw: (payload) => {
      sentPayloads.push(payload);
      return new Promise((resolve) => {
        sendResolvers.push(resolve);
      });
    },
    requestRepair: async (request) => {
      const channelId = request.channelId;
      historyFiltersByChannel[channelId].push(request.until);
      return pagesByChannel[channelId].shift() ?? [];
    },
  });

  await Promise.resolve();

  assert.deepEqual(
    sentPayloads.map((payload) => payload[1]),
    ["live-1", "live-2"],
  );
  assert.equal(sendResolvers.length, 2);
  assert.deepEqual(historyFiltersByChannel, {
    "channel-1": [],
    "channel-2": [],
  });

  for (const resolve of sendResolvers) {
    resolve();
  }
  await replayPromise;

  assert.deepEqual(historyFiltersByChannel, {
    "channel-1": [undefined, 1501],
    "channel-2": [undefined, 1701],
  });
});

// ── Per-batch gate re-check (F2 fix) ─────────────────────────────────────────

test("batch-1 arms gate mid-replay: batch-2 is withheld until gate expires", async () => {
  // Gate is inactive at the start of replay. Batch 1 fires and (simulating the
  // relay's admission control) activates the gate. Batch 2 must wait until the
  // gate clears before its REQs are sent.
  resetGate(0);

  const BATCH = REPLAY_BATCH_SIZE;
  const sentAtMs = []; // record the fakeNow when each REQ fires

  // Build BATCH+1 subscriptions so there are exactly two batches.
  const subscriptions = new Map(
    Array.from({ length: BATCH + 1 }, (_, i) => [
      `sub-${i}`,
      {
        mode: "live",
        filter: { kinds: [9], "#h": [`ch-${i}`], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ]),
  );

  let _batchCount = 0;
  const replayPromise = replayLiveSubscriptions({
    subscriptions,
    sendRaw: async (payload) => {
      sentAtMs.push({ id: payload[1], ms: fakeNow });
      // After the first full batch is sent, arm the gate for 5 s.
      // This simulates the relay responding to batch-1 traffic with back-pressure.
      if (sentAtMs.length === BATCH) {
        _batchCount += 1;
        activateRateLimit(5);
      }
    },
    requestRepair: async () => [],
    setTimeoutFn: (fn, _ms) => {
      fn();
      return 0;
    },
  });

  // Advance time to expire the gate while the replay is suspended in the
  // per-batch gate await. This unblocks the second batch.
  tickTo(5_001);

  await replayPromise;

  const batch1Ids = sentAtMs.filter((r) => r.ms < 5_001).map((r) => r.id);
  const batch2Ids = sentAtMs.filter((r) => r.ms >= 5_001).map((r) => r.id);

  assert.equal(
    batch1Ids.length,
    BATCH,
    "batch 1 must send exactly REPLAY_BATCH_SIZE REQs",
  );
  assert.equal(
    batch2Ids.length,
    1,
    "batch 2 must send the remaining sub after the gate expires",
  );
});

test("per-batch gate re-arm skips a subscription disposed while waiting", async () => {
  resetGate(0);
  const sentIds = [];
  let gateArmed;
  const gateArmedPromise = new Promise((resolve) => {
    gateArmed = resolve;
  });
  const subscriptions = new Map([
    [
      "sub-1",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-1"], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ],
    [
      "sub-2",
      {
        mode: "live",
        filter: { kinds: [9], "#h": ["ch-2"], limit: 50 },
        onEvent: () => {},
        lastSeenCreatedAt: undefined,
      },
    ],
  ]);

  const replayPromise = replayLiveSubscriptions({
    subscriptions,
    replayBatchSize: 1,
    sendRaw: async (payload) => {
      sentIds.push(payload[1]);
      if (payload[1] === "sub-1") {
        activateRateLimit(5);
        gateArmed();
      }
    },
    requestRepair: async () => [],
    setTimeoutFn: (fn, _ms) => {
      fn();
      return 0;
    },
  });

  await gateArmedPromise;
  assert.equal(subscriptions.delete("sub-2"), true);
  tickTo(5_001);
  await replayPromise;

  assert.deepEqual(sentIds, ["sub-1"]);
});

test("disposed subscription receives no repair events after an in-flight page", async () => {
  resetGate();
  const delivered = [];
  const subscription = {
    mode: "live",
    filter: buildChannelFilter("channel-1", 50),
    onEvent: (value) => delivered.push(value.id),
    lastSeenCreatedAt: 1000,
  };
  const subscriptions = new Map([["live-1", subscription]]);

  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async () => {},
    requestRepair: async () => {
      assert.equal(subscriptions.delete("live-1"), true);
      return [event("disposed", 1001)];
    },
  });

  assert.deepEqual(delivered, []);
});

test("stale repair cannot clear the successor generation dedupe state", async () => {
  resetGate();
  const duplicate = event("successor-seen", 1001);
  const subscription = {
    mode: "live",
    filter: buildChannelFilter("channel-1", 50),
    onEvent: () => {},
    lastSeenCreatedAt: 1000,
  };
  const subscriptions = new Map([["live-1", subscription]]);
  let generationActive = true;
  let startRepair;
  const repairStarted = new Promise((resolve) => {
    startRepair = resolve;
  });
  let finishRepair;
  const repairPage = new Promise((resolve) => {
    finishRepair = resolve;
  });

  const replayPromise = replayLiveSubscriptions({
    subscriptions,
    generation: 10,
    isActive: () => generationActive,
    sendRaw: async () => {},
    requestRepair: async () => {
      startRepair();
      return repairPage;
    },
  });

  await repairStarted;
  generationActive = false;
  subscription.reconnectReplay = {
    generation: 11,
    seenEventIds: new Set([duplicate.id]),
    liveEose: false,
    repairDone: false,
  };
  finishRepair([duplicate]);
  await replayPromise;

  assert.equal(subscription.reconnectReplay.generation, 11);
  assert.deepEqual(
    [...subscription.reconnectReplay.seenEventIds],
    [duplicate.id],
  );
  assert.equal(shouldDispatchSubscriptionEvent(subscription, duplicate), false);
});

// ── Backfill failure containment ─────────────────────────────────────────────

test("history backfill rejection never escapes replayLiveSubscriptions", async () => {
  resetGate(0);
  const filter = buildChannelFilter("channel-1", 50);
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter,
        onEvent: () => {},
        lastSeenCreatedAt: 1000,
      },
    ],
  ]);

  let historyCalls = 0;
  // Must resolve — a rejection here is the socket-killing flap regression.
  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async () => {},
    requestRepair: async () => {
      historyCalls++;
      throw new Error("rate-limited: quota exceeded; retry in 4s");
    },
  });

  assert.equal(
    historyCalls,
    PAGE_REPLAY_MAX_ATTEMPTS,
    "backfill must retry a bounded number of times, then degrade",
  );
});

test("backfill retry waits out the armed gate, then succeeds", async () => {
  resetGate(0);
  const delivered = [];
  const filter = buildChannelFilter("channel-1", 50);
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter,
        onEvent: (event) => delivered.push(event),
        lastSeenCreatedAt: 1000,
      },
    ],
  ]);

  const attemptAtMs = [];
  let armGate;
  const gateArmed = new Promise((resolve) => {
    armGate = resolve;
  });
  const replayPromise = replayLiveSubscriptions({
    subscriptions,
    sendRaw: async () => {},
    requestRepair: async () => {
      attemptAtMs.push(fakeNow);
      if (attemptAtMs.length === 1) {
        // Mirror relayClosedRecovery: the CLOSED handler arms the gate
        // before rejecting the history promise.
        activateRateLimit(4);
        armGate();
        throw new Error("rate-limited: quota exceeded; retry in 4s");
      }
      return [event("recovered", 1500)];
    },
  });

  // Wait until the gate is actually armed, then expire it. The retry loop is
  // (or will be) suspended in waitForRateLimit; expiring the gate releases it.
  await gateArmed;
  tickTo(4_001);
  await replayPromise;

  assert.equal(attemptAtMs.length, 2, "one failure, one retry");
  assert.ok(
    attemptAtMs[1] >= 4_001,
    "retry must not fire before the rate-limit gate expires",
  );
  assert.deepEqual(
    delivered.map((e) => e.id),
    ["recovered"],
    "the retried backfill must deliver its events",
  );
});

test("backfill retry aborts when the subscription was replaced", async () => {
  resetGate(0);
  const filter = buildChannelFilter("channel-1", 50);
  const subscription = {
    mode: "live",
    filter,
    onEvent: () => {},
    lastSeenCreatedAt: 1000,
  };
  const subscriptions = new Map([["live-1", subscription]]);

  let historyCalls = 0;
  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async () => {},
    requestRepair: async () => {
      historyCalls++;
      // Simulate the subscription being torn down while the REQ is in flight.
      subscriptions.delete("live-1");
      throw new Error("rate-limited: quota exceeded; retry in 4s");
    },
  });

  assert.equal(
    historyCalls,
    1,
    "no retry may target a subscription that no longer exists",
  );
});

test("exhausted backfill pins the floor: next replay still requests the original window after live events advance the cursor", async () => {
  // The blocking review scenario on PR #4990: cursor=1000, all backfill
  // attempts fail, a live event at 2100 then advances lastSeenCreatedAt via
  // prepareSubscriptionEvent. Without the pinned floor, the next reconnect
  // would start near 2095 and silently skip 1001..1999.
  resetGate(0);
  const filter = buildChannelFilter("channel-1", 50);
  const subscription = {
    mode: "live",
    filter,
    onEvent: () => {},
    lastSeenCreatedAt: 1000,
  };
  const subscriptions = new Map([["live-1", subscription]]);

  // Reconnect 1: every backfill attempt is rate-limited.
  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async () => {},
    requestRepair: async () => {
      throw new Error("rate-limited: quota exceeded; retry in 4s");
    },
  });
  assert.equal(
    subscription.pendingReplaySince,
    0,
    "exhausted backfill must pin the unresolved window's lower bound",
  );

  // A live event arrives through the normal cursor path.
  prepareSubscriptionEvent(subscription, event("live-newer", 2100));
  assert.equal(subscription.lastSeenCreatedAt, 2100);

  // Reconnect 2: backfill now succeeds. It must request the ORIGINAL window.
  const historyFilters = [];
  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async () => {},
    requestRepair: async (request) => {
      historyFilters.push(request);
      return [];
    },
  });

  assert.equal(historyFilters.length, 1);
  assert.equal(
    historyFilters[0].since,
    0,
    "replay must start from the pinned floor, not the advanced cursor",
  );
  assert.equal(
    subscription.pendingReplaySince,
    undefined,
    "a completed backfill must clear the pinned floor",
  );

  // Reconnect 3: with the floor cleared, replay returns to the cursor.
  const laterFilters = [];
  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async () => {},
    requestRepair: async (request) => {
      laterFilters.push(request);
      return [];
    },
  });
  assert.equal(
    laterFilters[0].since,
    235,
    "after recovery the cursor governs again",
  );
});

test("in-flight stale abort keeps the pinned floor for the superseding connection", async () => {
  // Race from re-review of b70a6716d/c493d378b: production supersession bumps
  // the connection GENERATION while the same subscription key and object
  // survive in the map. The identity guard alone stays true, so only the
  // combined guard (outer isActive && identity) aborts the stale pass. That
  // abort must NOT count as completion — the pinned floor belongs to the
  // superseding connection's replay.
  resetGate(0);
  const filter = buildChannelFilter("channel-1", 50);
  const subscription = {
    mode: "live",
    filter,
    onEvent: () => {},
    lastSeenCreatedAt: 1000,
  };
  const subscriptions = new Map([["live-1", subscription]]);

  let generationActive = true;
  let historyCalls = 0;
  await replayLiveSubscriptions({
    subscriptions,
    sendRaw: async () => {},
    isActive: () => generationActive,
    requestRepair: async () => {
      historyCalls++;
      // Connection A is superseded while the REQ is in flight: the generation
      // advances, but the subscription keeps its key AND object identity —
      // exactly what production supersession does.
      generationActive = false;
      // A full page would otherwise continue paging — the post-await
      // combined guard must abort instead.
      return eventRange("full", 1001, 500);
    },
  });

  assert.equal(historyCalls, 1, "stale generation must stop paging");
  assert.equal(
    subscriptions.get("live-1"),
    subscription,
    "precondition: key and object survive supersession untouched",
  );
  assert.equal(
    subscription.pendingReplaySince,
    0,
    "a stale-generation abort must not clear the floor the new connection needs",
  );
});

test("future-dated cursor repairs backdated edits and deletions", async () => {
  resetGate();
  const delivered = [];
  const recovered = [
    { ...event("5".repeat(64), 10_010), kind: 5 },
    { ...event("6".repeat(64), 10_011), kind: 9005 },
    { ...event("7".repeat(64), 10_012), kind: 40003 },
  ];
  const subscription = {
    mode: "live",
    filter: buildChannelFilter("channel-1", 50),
    onEvent: (value) => delivered.push(value),
    // Relay accepted this event 900s in the future before disconnect.
    lastSeenCreatedAt: 10_900,
  };
  const requests = [];
  await replayLiveSubscriptions({
    subscriptions: new Map([["live-1", subscription]]),
    generation: 11,
    sendRaw: async () => {},
    requestRepair: async (request) => {
      requests.push(request);
      return recovered;
    },
  });
  assert.equal(requests[0].since, 9_035);
  assert.equal(requests[0].until, undefined);
  assert.deepEqual(
    delivered.map((value) => value.kind),
    [5, 9005, 40003],
  );
});

test("dense-second repair advances by event id without gaps", async () => {
  resetGate();
  const delivered = [];
  const requests = [];
  const subscription = {
    mode: "live",
    filter: buildChannelFilter("channel-1", 50),
    onEvent: (value) => delivered.push(value.id),
    lastSeenCreatedAt: 2000,
  };
  const dense = Array.from({ length: 1_205 }, (_, index) =>
    event(index.toString().padStart(64, "0"), 2000),
  );
  await replayLiveSubscriptions({
    subscriptions: new Map([["live-1", subscription]]),
    generation: 7,
    sendRaw: async () => {},
    requestRepair: async (request) => {
      requests.push(request);
      const start = request.beforeId
        ? dense.findIndex((value) => value.id === request.beforeId) + 1
        : 0;
      return dense.slice(start, start + request.limit);
    },
  });
  assert.equal(delivered.length, dense.length);
  assert.equal(new Set(delivered).size, dense.length);
  assert.deepEqual(
    requests.map((request) => request.beforeId),
    [undefined, dense[499].id, dense[999].id],
  );
});

test("replay dedupe clears only after both completions and never from stale generation", () => {
  for (const repairFirst of [true, false]) {
    const subscription = {
      mode: "live",
      filter: buildChannelFilter("channel-1", 50),
      onEvent: () => {},
      reconnectReplay: {
        generation: 12,
        seenEventIds: new Set(["seen"]),
        liveEose: false,
        repairDone: false,
      },
    };
    markReconnectRepairDone(subscription, 11);
    markReconnectLiveEose(subscription, 11);
    assert.equal(subscription.reconnectReplay.generation, 12);
    if (repairFirst) {
      markReconnectRepairDone(subscription, 12);
      assert.ok(subscription.reconnectReplay);
      markReconnectLiveEose(subscription, 12);
    } else {
      markReconnectLiveEose(subscription, 12);
      assert.ok(subscription.reconnectReplay);
      markReconnectRepairDone(subscription, 12);
    }
    assert.equal(subscription.reconnectReplay, undefined);
  }
});

test("buffer flush drops stale generations and removed subscriptions", () => {
  const delivered = [];
  const subscription = {
    mode: "live",
    filter: buildChannelFilter("channel-1", 50),
    onEvent: (value) => delivered.push(value.id),
  };
  flushEvents(
    [
      { subId: "live-1", event: event("stale", 100), generation: 6 },
      { subId: "removed", event: event("removed", 101), generation: 7 },
      { subId: "live-1", event: event("current", 102), generation: 7 },
    ],
    new Map([["live-1", subscription]]),
    7,
  );
  assert.deepEqual(delivered, ["current"]);
});

test("live-before-repair and repair-before-live dispatch once", async () => {
  const shared = event("a".repeat(64), 1000);
  for (const liveFirst of [true, false]) {
    const delivered = [];
    const subscription = {
      mode: "live",
      filter: buildChannelFilter("channel-1", 50),
      onEvent: (value) => delivered.push(value.id),
      reconnectReplay: {
        generation: 4,
        seenEventIds: new Set(),
        liveEose: false,
        repairDone: false,
      },
    };
    if (liveFirst) {
      assert.equal(shouldDispatchSubscriptionEvent(subscription, shared), true);
      subscription.onEvent(shared);
    }
    await replayReconnectHistoryPages({
      subscription,
      channelId: "channel-1",
      since: 0,
      until: 1000,
      isActive: () => true,
      requestRepair: async () => [shared],
    });
    if (!liveFirst && shouldDispatchSubscriptionEvent(subscription, shared)) {
      subscription.onEvent(shared);
    }
    assert.deepEqual(delivered, [shared.id]);
  }
});

// ── Teardown ──────────────────────────────────────────────────────────────────

test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
