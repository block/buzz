import assert from "node:assert/strict";
import test from "node:test";

import {
  canonicalCommunityRelays,
  classifyReconcileResult,
  pendingReconcileRelays,
  reconcileRetryDelayMs,
} from "./managedAgentReconciliationPlan.ts";
import { canonicalRelayUrl } from "./managedAgentRuntimeStatus.ts";

test("reconcileRetryDelayMs walks a capped backoff then gives up", () => {
  assert.equal(reconcileRetryDelayMs(1), 5_000);
  assert.equal(reconcileRetryDelayMs(2), 30_000);
  assert.equal(reconcileRetryDelayMs(3), 120_000);
  assert.equal(reconcileRetryDelayMs(4), null);
  assert.equal(reconcileRetryDelayMs(0), null);
});

test("canonicalCommunityRelays dedupes by canonical form, keeps stored spelling", () => {
  const relays = canonicalCommunityRelays(
    [
      { relayUrl: "ws://localhost:3000" },
      // Same relay, different spelling — folds onto the first entry.
      { relayUrl: "ws://127.0.0.1:3000" },
      { relayUrl: "wss://relay.example" },
      // Unparsable entries are dropped rather than reconciled.
      { relayUrl: "not a url" },
    ],
    canonicalRelayUrl,
  );
  assert.deepEqual(
    [...relays.entries()],
    [
      ["ws://127.0.0.1:3000", "ws://localhost:3000"],
      ["wss://relay.example", "wss://relay.example"],
    ],
  );
});

test("pendingReconcileRelays skips reconciled and in-flight relays", () => {
  const canonicalToRequested = new Map([
    ["ws://127.0.0.1:3000", "ws://localhost:3000"],
    ["wss://a.example", "wss://a.example"],
    ["wss://b.example", "wss://b.example"],
  ]);
  const pending = pendingReconcileRelays(
    canonicalToRequested,
    new Set(["wss://a.example"]),
    new Set(["ws://127.0.0.1:3000"]),
  );
  assert.deepEqual(pending, ["wss://b.example"]);
});

test("classifyReconcileResult marks the whole batch failed when the call throws", () => {
  const attempted = ["wss://a.example", "wss://b.example"];
  assert.deepEqual(
    classifyReconcileResult(attempted, null, canonicalRelayUrl),
    {
      succeeded: [],
      failed: attempted,
    },
  );
});

test("classifyReconcileResult splits by Failed rows, matching on requested URL", () => {
  const attempted = ["ws://127.0.0.1:3000", "wss://b.example"];
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
    // Failed on b.example — stays failing so it is retried.
    {
      pubkey: "aa",
      relayUrl: "wss://b.example",
      requestedRelayUrl: "wss://b.example",
      localSetup: true,
      lifecycle: "failed",
      pid: null,
      error: "relay access probe timed out",
      logPath: null,
    },
  ];
  assert.deepEqual(
    classifyReconcileResult(attempted, rows, canonicalRelayUrl),
    {
      succeeded: ["ws://127.0.0.1:3000"],
      failed: ["wss://b.example"],
    },
  );
});

test("classifyReconcileResult treats a relay with no rows as reconciled", () => {
  // A community with no eligible auto-start agents produces no rows; it must
  // still count as reconciled so the hook stops retrying it.
  assert.deepEqual(
    classifyReconcileResult(["wss://a.example"], [], canonicalRelayUrl),
    { succeeded: ["wss://a.example"], failed: [] },
  );
});
