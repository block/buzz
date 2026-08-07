import assert from "node:assert/strict";
import test from "node:test";

import {
  buildReconnectReplayFilter,
  PAGE_REPLAY_MAX_ATTEMPTS,
  replayLiveSubscriptions,
  REPLAY_BATCH_SIZE,
  shouldPageReconnectReplay,
} from "./relayReconnectReplay.ts";
import { buildChannelFilter } from "./relayChannelFilters.ts";
import { prepareSubscriptionEvent } from "./relayClosedRecovery.ts";

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
    now: 2000,
    sendRaw: async () => {},
    requestHistory: async () => {
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
    now: 2000,
    sendRaw: async () => {},
    requestHistory: async () => {
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
    now: 2000,
    sendRaw: async () => {},
    requestHistory: async () => {
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
    now: 2000,
    sendRaw: async () => {},
    requestHistory: async () => {
      throw new Error("rate-limited: quota exceeded; retry in 4s");
    },
  });
  assert.equal(
    subscription.pendingReplaySince,
    995,
    "exhausted backfill must pin the unresolved window's lower bound",
  );

  // A live event arrives through the normal cursor path.
  prepareSubscriptionEvent(subscription, event("live-newer", 2100));
  assert.equal(subscription.lastSeenCreatedAt, 2100);

  // Reconnect 2: backfill now succeeds. It must request the ORIGINAL window.
  const historyFilters = [];
  await replayLiveSubscriptions({
    subscriptions,
    now: 2200,
    sendRaw: async () => {},
    requestHistory: async (filter) => {
      historyFilters.push(filter);
      return [];
    },
  });

  assert.equal(historyFilters.length, 1);
  assert.equal(
    historyFilters[0].since,
    995,
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
    now: 2300,
    sendRaw: async () => {},
    requestHistory: async (filter) => {
      laterFilters.push(filter);
      return [];
    },
  });
  assert.equal(
    laterFilters[0].since,
    2095,
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
    now: 2000,
    sendRaw: async () => {},
    isActive: () => generationActive,
    requestHistory: async () => {
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
    995,
    "a stale-generation abort must not clear the floor the new connection needs",
  );
});

// ── Teardown ──────────────────────────────────────────────────────────────────

test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
