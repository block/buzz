import assert from "node:assert/strict";
import test from "node:test";

import {
  buildReconnectReplayFilter,
  replayLiveSubscriptions,
  REPLAY_BATCH_SIZE,
} from "./relayReconnectReplay.ts";
import { buildChannelFilter } from "./relayChannelFilters.ts";

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
  );
}

function replayFilter(filter, since, until) {
  return buildReconnectReplayFilter(filter, since, until);
}

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
    requestHistory: async () => [],
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
    requestHistory: async () => [],
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
    requestHistory: async () => [],
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
    requestHistory: async () => [],
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
    requestHistory: async () => [],
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
    now: 2000,
    sendRaw: async (payload) => {
      sentPayloads.push(payload);
    },
    requestHistory: async (filter) => {
      historyFilters.push(filter);
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
      kinds: filter.kinds,
      "#h": ["channel-1"],
      limit: 500,
      since: 995,
      until: 2000,
    },
    {
      kinds: filter.kinds,
      "#h": ["channel-1"],
      limit: 500,
      since: 995,
      until: 1501,
    },
    {
      kinds: filter.kinds,
      "#h": ["channel-1"],
      limit: 500,
      since: 995,
      until: 1002,
    },
  ]);
  assert.equal(delivered.length, 1008);
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
    now: 2000,
    pageReplayConcurrency: 2,
    sendRaw: (payload) => {
      sentPayloads.push(payload);
      return new Promise((resolve) => {
        sendResolvers.push(resolve);
      });
    },
    requestHistory: async (filter) => {
      const channelId = filter["#h"]?.[0];
      historyFiltersByChannel[channelId].push(filter.until);
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
    "channel-1": [2000, 1501],
    "channel-2": [2000, 1701],
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
    requestHistory: async () => [],
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

// ── Teardown ──────────────────────────────────────────────────────────────────

test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
