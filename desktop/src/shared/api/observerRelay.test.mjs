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
