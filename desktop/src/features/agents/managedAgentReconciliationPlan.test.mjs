import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyReconcileResult,
  connectionTargetCommunityRelays,
  pendingReconcileRelays,
  reconcileRetryDelayMs,
} from "./managedAgentReconciliationPlan.ts";
import { connectionTargetUrl } from "./managedAgentRuntimeStatus.ts";

test("reconcileRetryDelayMs walks a capped backoff then gives up", () => {
  assert.equal(reconcileRetryDelayMs(1), 5_000);
  assert.equal(reconcileRetryDelayMs(2), 30_000);
  assert.equal(reconcileRetryDelayMs(3), 120_000);
  assert.equal(reconcileRetryDelayMs(4), null);
  assert.equal(reconcileRetryDelayMs(0), null);
});

test("connectionTargetCommunityRelays attempts localhost and 127 separately", () => {
  const relays = connectionTargetCommunityRelays(
    [
      { relayUrl: "ws://localhost:3000" },
      // Connection-equivalent formatting dedupes onto the first entry.
      { relayUrl: "ws://LOCALHOST:3000/" },
      // Canonical runtime-key aliases are still distinct tenant attempts.
      { relayUrl: "ws://127.0.0.1:3000" },
      { relayUrl: "wss://relay.example" },
      // Unparsable entries are dropped rather than reconciled.
      { relayUrl: "not a url" },
    ],
    connectionTargetUrl,
  );
  assert.deepEqual(
    [...relays.entries()],
    [
      ["ws://localhost:3000", "ws://localhost:3000"],
      ["ws://127.0.0.1:3000", "ws://127.0.0.1:3000"],
      ["wss://relay.example", "wss://relay.example"],
    ],
  );
});

test("pendingReconcileRelays skips reconciled and in-flight relays", () => {
  const targetToRequested = new Map([
    ["ws://localhost:3000", "ws://localhost:3000"],
    ["wss://a.example", "wss://a.example"],
    ["wss://b.example", "wss://b.example"],
  ]);
  const pending = pendingReconcileRelays(
    targetToRequested,
    new Set(["wss://a.example"]),
    new Set(["ws://localhost:3000"]),
  );
  assert.deepEqual(pending, ["wss://b.example"]);
});

test("classifyReconcileResult marks the whole batch failed when the call throws", () => {
  const attempted = ["wss://a.example", "wss://b.example"];
  assert.deepEqual(
    classifyReconcileResult(attempted, null, connectionTargetUrl),
    {
      succeeded: [],
      failed: attempted,
    },
  );
});

test("classifyReconcileResult keeps colliding loopback targets separate", () => {
  const attempted = ["ws://localhost:3000", "ws://127.0.0.1:3000"];
  const rows = [
    // Started cleanly on the loopback relay — reconciled.
    {
      pubkey: "aa",
      relayUrl: "ws://127.0.0.1:3000",
      requestedRelayUrl: "ws://localhost:3000",
      localSetup: true,
      lifecycle: "starting",
      pid: 1,
      error: null,
      logPath: null,
    },
    // The canonical-key collision for 127 is explicit and retried separately.
    {
      pubkey: "aa",
      relayUrl: "ws://127.0.0.1:3000",
      requestedRelayUrl: "ws://127.0.0.1:3000",
      localSetup: true,
      lifecycle: "failed",
      pid: null,
      error: "connection-target conflict",
      logPath: null,
    },
  ];
  assert.deepEqual(
    classifyReconcileResult(attempted, rows, connectionTargetUrl),
    {
      succeeded: ["ws://localhost:3000"],
      failed: ["ws://127.0.0.1:3000"],
    },
  );
});

test("classifyReconcileResult treats a relay with no rows as reconciled", () => {
  // A community with no eligible auto-start agents produces no rows; it must
  // still count as reconciled so the hook stops retrying it.
  assert.deepEqual(
    classifyReconcileResult(["wss://a.example"], [], connectionTargetUrl),
    { succeeded: ["wss://a.example"], failed: [] },
  );
});
