import assert from "node:assert/strict";
import test from "node:test";

import {
  mergeManagedAgentRuntimeStatuses,
  replaceManagedAgentRuntimeStatus,
  restartManagedAgentPair,
  stoppedPairMatchesActiveCommunity,
} from "./managedAgentRuntimeHooks.ts";

// ---------------------------------------------------------------------------
// restartManagedAgentPair: discriminating regression tests for the pair
// restart lifecycle boundary (stop → relay-scoped clear → start).
//
// These tests exercise the exact function called by useManagedAgentRuntimeAction's
// mutationFn restart branch, so reverting to the old combined Rust command
// (which cleared only in onSuccess, after the new process was already running)
// would make tests (a) and (c) fail.
// ---------------------------------------------------------------------------

const PUBKEY = "deadbeef".repeat(8);
const RELAY = "wss://relay.example";

/** Returns a resolved-status stub sufficient for the return-type assertion. */
function makeStatus(overrides = {}) {
  return {
    pubkey: PUBKEY,
    relayUrl: RELAY,
    localSetup: true,
    lifecycle: "running",
    ...overrides,
  };
}

test("cache merge retains colliding localhost success and 127 failure rows", () => {
  const liveLocalhost = makeStatus({
    pubkey: PUBKEY.toUpperCase(),
    relayUrl: "ws://127.0.0.1:3000",
    requestedRelayUrl: "ws://localhost:3000",
    lifecycle: "ready",
  });
  const failedIpv4 = makeStatus({
    relayUrl: "ws://127.0.0.1:3000",
    requestedRelayUrl: "ws://127.0.0.1:3000",
    lifecycle: "failed",
    error: "connection-target conflict",
  });
  const baseline = [liveLocalhost];

  const merged = mergeManagedAgentRuntimeStatuses(baseline, baseline, [
    failedIpv4,
  ]);

  assert.equal(merged.length, 2);
  assert.equal(
    merged.find((row) => row.requestedRelayUrl.includes("localhost"))
      ?.lifecycle,
    "ready",
  );
  assert.equal(
    merged.find((row) => row.requestedRelayUrl.includes("127.0.0.1"))
      ?.lifecycle,
    "failed",
  );
});

test("action replacement updates only the authoritative connection target", () => {
  const liveLocalhost = makeStatus({
    relayUrl: "ws://127.0.0.1:3000",
    requestedRelayUrl: "ws://localhost:3000",
    lifecycle: "ready",
  });
  const failedIpv4 = makeStatus({
    relayUrl: "ws://127.0.0.1:3000",
    requestedRelayUrl: "ws://127.0.0.1:3000",
    lifecycle: "failed",
  });
  const stoppedLocalhost = makeStatus({
    pubkey: PUBKEY.toUpperCase(),
    relayUrl: "ws://127.0.0.1:3000",
    requestedRelayUrl: "ws://LOCALHOST:3000/",
    lifecycle: "stopped",
  });

  const replaced = replaceManagedAgentRuntimeStatus(
    [liveLocalhost, failedIpv4],
    stoppedLocalhost,
  );
  assert.equal(replaced.length, 2);
  assert.equal(replaced[0], stoppedLocalhost);
  assert.equal(replaced[1], failedIpv4);
});

test("stop badge clearing never crosses loopback tenants", () => {
  assert.equal(
    stoppedPairMatchesActiveCommunity(
      "ws://localhost:3000",
      "ws://127.0.0.1:3000",
    ),
    false,
  );
  assert.equal(
    stoppedPairMatchesActiveCommunity("ws://LOCALHOST:80/", "ws://localhost"),
    true,
  );
});

test("test_pair_restart_stop_success_start_failure_clear_still_ran", async () => {
  // Stop succeeds, start throws.  The clear must have fired — badge is gone
  // regardless of the start failure.  On the old combined-command approach,
  // a rejected command meant onSuccess never ran and the badge survived.
  let clearFired = false;

  await assert.rejects(
    restartManagedAgentPair(
      PUBKEY,
      RELAY,
      async () => makeStatus(), // stop succeeds
      (_pubkey, _relayUrl) => {
        clearFired = true;
      },
      async () => {
        throw new Error("start failed");
      },
    ),
    /start failed/,
  );

  assert.ok(
    clearFired,
    "clear must fire at stop-success boundary even when start subsequently fails",
  );
});

test("test_pair_restart_stop_failure_neither_clear_nor_start_called", async () => {
  // Stop throws.  Neither clear nor start should run — clearing on a failed
  // stop would remove a badge that is still legitimately active.
  let clearFired = false;
  let startCalled = false;

  await assert.rejects(
    restartManagedAgentPair(
      PUBKEY,
      RELAY,
      async () => {
        throw new Error("stop failed");
      },
      (_pubkey, _relayUrl) => {
        clearFired = true;
      },
      async () => {
        startCalled = true;
        return makeStatus();
      },
    ),
    /stop failed/,
  );

  assert.ok(!clearFired, "clear must NOT fire when stop itself fails");
  assert.ok(!startCalled, "start must NOT be called when stop fails");
});

test("test_pair_restart_strict_stop_clear_start_ordering", async () => {
  // Verify the operations fire in the guaranteed order: stop → clear → start.
  // A clear that fires after start begins can tombstone genuine new turns.
  const events = [];

  await restartManagedAgentPair(
    PUBKEY,
    RELAY,
    async () => {
      events.push("stop");
      return makeStatus();
    },
    (_pubkey, _relayUrl) => {
      events.push("clear");
    },
    async () => {
      events.push("start");
      return makeStatus();
    },
  );

  assert.deepEqual(
    events,
    ["stop", "clear", "start"],
    "operations must fire in stop → clear → start order",
  );
});
