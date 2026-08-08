import assert from "node:assert/strict";
import test from "node:test";

import { restartManagedAgentPair } from "./managedAgentRuntimeHooks.ts";

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
function makeStatus() {
  return {
    pubkey: PUBKEY,
    relayUrl: RELAY,
    localSetup: true,
    lifecycle: "running",
  };
}

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
