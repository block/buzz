import assert from "node:assert/strict";
import test from "node:test";

import {
  shouldAttributeFetch,
  buildSwitchPerfLogRecord,
  resolveSettleAction,
  summarizeChannelSwitchTrace,
} from "./channelSwitchPerf.ts";

function trace(overrides = {}) {
  return {
    channelId: "abcdef1234567890",
    startedAt: 1_000,
    routeCommitAt: null,
    windowFetch: null,
    membersFetch: null,
    ...overrides,
  };
}

test("summary reports total and cache-served fetches", () => {
  const summary = summarizeChannelSwitchTrace(trace(), 1_412.4);
  assert.equal(
    summary,
    "[switch-perf] channel=abcdef12 total=412ms commit=? window=cache members=cache",
  );
});

test("summary includes route commit offset and fetch timings", () => {
  const summary = summarizeChannelSwitchTrace(
    trace({
      routeCommitAt: 1_038,
      windowFetch: { durationMs: 180.6, eventCount: 250 },
      membersFetch: { durationMs: 320.2, memberCount: 10_000 },
    }),
    1_912,
  );
  assert.equal(
    summary,
    "[switch-perf] channel=abcdef12 total=912ms commit=+38ms " +
      "window=250 events in 181ms members=10000 members in 320ms",
  );
});

test("log record carries rounded stage timings and fetch attributions", () => {
  const record = buildSwitchPerfLogRecord(
    trace({
      routeCommitAt: 1_038.4,
      windowFetch: { durationMs: 180.6, eventCount: 250 },
      membersFetch: { durationMs: 320.2, memberCount: 10_000 },
    }),
    1_912.3,
  );
  assert.equal(record.channelId, "abcdef1234567890");
  assert.equal(record.totalMs, 912);
  assert.equal(record.commitOffsetMs, 38);
  assert.deepEqual(record.windowFetch, { durationMs: 181, eventCount: 250 });
  assert.deepEqual(record.membersFetch, {
    durationMs: 320,
    memberCount: 10_000,
  });
  assert.equal(typeof record.ts, "string");
});

test("log record marks cache-served fetches and missing commit as null", () => {
  const record = buildSwitchPerfLogRecord(trace(), 1_412);
  assert.equal(record.commitOffsetMs, null);
  assert.equal(record.windowFetch, null);
  assert.equal(record.membersFetch, null);
});

test("settle resolves only the trace for the settled channel", () => {
  const active = trace();
  assert.deepEqual(resolveSettleAction(active, "abcdef1234567890", 2_000), {
    settledTrace: active,
    clearActive: true,
  });
  assert.deepEqual(resolveSettleAction(null, "abcdef1234567890", 2_000), {
    settledTrace: null,
    clearActive: false,
  });
});

test("a mismatched settle never clobbers a newer switch's trace", () => {
  // Channel A settles after the user already clicked channel B: B's trace
  // must survive so B still gets measured.
  const nextSwitch = trace({ channelId: "bbbb0000bbbb0000" });
  assert.deepEqual(resolveSettleAction(nextSwitch, "abcdef1234567890", 2_000), {
    settledTrace: null,
    clearActive: false,
  });
});

test("settle drops a trace that has timed out", () => {
  const stale = trace({ startedAt: 1_000 });
  assert.deepEqual(resolveSettleAction(stale, "abcdef1234567890", 31_001), {
    settledTrace: null,
    clearActive: true,
  });
  assert.deepEqual(
    resolveSettleAction(stale, "abcdef1234567890", 11_000).settledTrace,
    stale,
  );
});

test("fetches attribute only when started after the switch began", () => {
  const active = trace({ channelId: "abcdef1234567890", startedAt: 1_000 });
  // Started before the switch (stale A→B→A leg): not attributable.
  assert.equal(shouldAttributeFetch(active, "abcdef1234567890", 999), false);
  // Started at/after the switch: attributable.
  assert.equal(shouldAttributeFetch(active, "abcdef1234567890", 1_000), true);
  assert.equal(shouldAttributeFetch(active, "abcdef1234567890", 1_500), true);
  // Other channel or no trace: never.
  assert.equal(shouldAttributeFetch(active, "bbbb0000bbbb0000", 1_500), false);
  assert.equal(shouldAttributeFetch(null, "abcdef1234567890", 1_500), false);
});

// --- Settle lifecycle: rapid switches and community resets ----------------

async function withSettleHarness(run) {
  const frames = [];
  const originalWindow = globalThis.window;
  const originalDocument = globalThis.document;
  globalThis.window = {
    requestAnimationFrame: (cb) => frames.push(cb) && frames.length,
    cancelAnimationFrame: () => {},
  };
  globalThis.document = { querySelector: () => null };
  const {
    abandonChannelSwitchTrace,
    beginChannelSwitchTrace,
    settleChannelSwitchTrace,
    resetChannelSwitchTrace,
    CHANNEL_SWITCH_MEASURE,
  } = await import("./channelSwitchPerf.ts");
  performance.clearMeasures?.(CHANNEL_SWITCH_MEASURE);
  const flush = () => {
    // Drain chained rAFs until quiescent.
    for (let i = 0; i < 20 && frames.length > 0; i += 1) {
      for (const cb of frames.splice(0, frames.length)) cb();
    }
  };
  const measures = () =>
    performance
      .getEntriesByName(CHANNEL_SWITCH_MEASURE)
      .map((entry) => entry.detail?.channelId);
  try {
    await run({
      abandon: abandonChannelSwitchTrace,
      begin: beginChannelSwitchTrace,
      settle: settleChannelSwitchTrace,
      reset: resetChannelSwitchTrace,
      flush,
      measures,
    });
  } finally {
    resetChannelSwitchTrace();
    performance.clearMeasures?.(CHANNEL_SWITCH_MEASURE);
    if (originalWindow === undefined) delete globalThis.window;
    else globalThis.window = originalWindow;
    if (originalDocument === undefined) delete globalThis.document;
    else globalThis.document = originalDocument;
  }
}

test("a switch begun during A's deferred wait drops A's record (no clock theft)", async () => {
  await withSettleHarness(async ({ begin, settle, flush, measures }) => {
    begin("aaaa1111aaaa1111");
    settle("aaaa1111aaaa1111"); // A's deferred-paint wait is now queued
    begin("bbbb2222bbbb2222"); // rapid follow-up switch replaces the trace
    flush();
    // A must NOT be recorded: its settledAt would be sampled from B's
    // timeline, charging B's delay to A.
    assert.deepEqual(measures(), []);
    settle("bbbb2222bbbb2222");
    flush();
    assert.deepEqual(measures(), ["bbbb2222bbbb2222"]);
  });
});

test("a community reset during the deferred wait drops the record", async () => {
  await withSettleHarness(async ({ begin, settle, reset, flush, measures }) => {
    begin("aaaa1111aaaa1111");
    settle("aaaa1111aaaa1111");
    reset();
    flush();
    assert.deepEqual(measures(), []);
  });
});

test("an undisturbed settle records exactly one measure", async () => {
  await withSettleHarness(async ({ begin, settle, flush, measures }) => {
    begin("aaaa1111aaaa1111");
    settle("aaaa1111aaaa1111");
    flush();
    assert.deepEqual(measures(), ["aaaa1111aaaa1111"]);
  });
});

test("leaving the channel surface abandons the trace; history-back records nothing", async () => {
  await withSettleHarness(
    async ({ abandon, begin, settle, flush, measures }) => {
      begin("aaaa1111aaaa1111");
      // Route exit (Projects/Home): the channel screen unmounts before the
      // trace settled and abandons it.
      abandon("aaaa1111aaaa1111");
      // History-back re-enters the channel without goChannel; its settle must
      // find no trace — otherwise the time spent away would be recorded as
      // switch latency.
      settle("aaaa1111aaaa1111");
      flush();
      assert.deepEqual(measures(), []);
    },
  );
});
