import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "./relayClient.ts";
import { subscribeToAgentObserverFrames } from "./observerRelay.ts";

// Regression guard: the observer subscription MUST request a replay-capable
// limit. With `limit: 0` the relay truncates reconnect replay to zero rows
// (NIP-01: limit 0 = no historical rows), so a turn_started missed during a
// network drop never re-delivers and the active-agents badge never appears.
test("subscribeToAgentObserverFrames requests a replay-capable limit with a since window", () => {
  const calls = [];
  mock.method(relayClient, "subscribeLive", (filter) => {
    calls.push(filter);
    return () => {};
  });

  subscribeToAgentObserverFrames("owner-pubkey", () => {});

  assert.equal(calls.length, 1);
  assert.equal(
    calls[0].limit,
    1000,
    "observer sub must use limit:1000 so reconnect replay can recover missed frames — limit:0 drops the gap",
  );
  assert.ok(
    calls[0].since && calls[0].since > 0,
    "observer sub must carry `since` so launch history stays suppressed — only the reconnect replay window is backfilled",
  );

  mock.reset();
});

// Regression guard: the live subscription lookback MUST be at least 300s so
// session/prompt frames from long-running active turns are captured when the
// desktop subscribes mid-turn. This test mocks Date.now() to a fixed epoch and
// asserts the computed `since` equals exactly (now/1000 - 300). It MUST fail
// if OBSERVER_LIVE_LOOKBACK_SECS is reverted to 0 or less than 300.
test("subscribeToAgentObserverFrames since is at least 300s before now", () => {
  // Pin Date.now() to a known value so the assertion is deterministic.
  const FIXED_NOW_MS = 1_750_000_000_000;
  const expectedSince = Math.floor(FIXED_NOW_MS / 1_000) - 300;

  mock.method(Date, "now", () => FIXED_NOW_MS);

  const calls = [];
  mock.method(relayClient, "subscribeLive", (filter) => {
    calls.push(filter);
    return () => {};
  });

  subscribeToAgentObserverFrames("owner-pubkey", () => {});

  assert.equal(calls.length, 1);
  assert.equal(
    calls[0].since,
    expectedSince,
    `observer since must be now-300s (${expectedSince}) so session/prompt frames from long-running turns are captured — zero lookback recreates the lifecycle-only turn bug`,
  );

  mock.reset();
});
