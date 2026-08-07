import assert from "node:assert/strict";
import test from "node:test";

import { restartManagedAgentPair } from "./managedAgentRuntimeHooks.ts";

// ---------------------------------------------------------------------------
// restartManagedAgentPair: discriminating regression tests for the pair
// restart lifecycle boundary (backend-atomic restart → relay-scoped clear).
//
// These tests exercise the exact function called by useManagedAgentRuntimeAction's
// mutationFn restart branch, so reverting to split stop/start IPC calls makes
// the ordering test fail.
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

test("test_pair_restart_success_clears_and_returns_runtime", async () => {
  let clearFired = false;
  const status = makeStatus();
  const result = await restartManagedAgentPair(
    PUBKEY,
    RELAY,
    async () => status,
    (_pubkey, _relayUrl) => {
      clearFired = true;
    },
  );
  assert.equal(result, status);
  assert.ok(clearFired, "clear must fire after a successful atomic restart");
});

test("test_pair_restart_failure_does_not_clear", async () => {
  let clearFired = false;

  await assert.rejects(
    restartManagedAgentPair(
      PUBKEY,
      RELAY,
      async () => {
        throw new Error("restart failed");
      },
      (_pubkey, _relayUrl) => {
        clearFired = true;
      },
    ),
    /restart failed/,
  );

  assert.ok(!clearFired, "clear must NOT fire when atomic restart fails");
});

test("test_pair_restart_strict_atomic_restart_then_clear_ordering", async () => {
  const events = [];

  await restartManagedAgentPair(
    PUBKEY,
    RELAY,
    async () => {
      events.push("atomic-restart");
      return makeStatus();
    },
    (_pubkey, _relayUrl) => {
      events.push("clear");
    },
  );

  assert.deepEqual(
    events,
    ["atomic-restart", "clear"],
    "clear must follow the one atomic backend restart",
  );
});
