import assert from "node:assert/strict";
import test from "node:test";

import { mergeManagedAgentRuntimeStatuses } from "./managedAgentRuntimeHooks.ts";

const runtime = (overrides = {}) => ({
  pubkey: "aa",
  relayUrl: "wss://relay.example",
  localSetup: true,
  lifecycle: "starting",
  pid: 1,
  error: null,
  logPath: null,
  ...overrides,
});

test("startup reconcile does not clobber a lifecycle update received in flight", () => {
  const starting = runtime();
  const ready = runtime({ lifecycle: "ready" });
  const reconciled = runtime({ requestedRelayUrl: "WSS://RELAY.EXAMPLE/" });

  assert.deepEqual(
    mergeManagedAgentRuntimeStatuses([starting], [ready], [reconciled]),
    [{ ...reconciled, ...ready }],
  );
});

test("startup reconcile replaces an unchanged baseline row with its newer result", () => {
  const stopped = runtime({ lifecycle: "stopped", pid: null });
  const starting = runtime({ lifecycle: "starting", pid: 2 });

  assert.deepEqual(
    mergeManagedAgentRuntimeStatuses([stopped], [stopped], [starting]),
    [starting],
  );
});

test("startup reconcile preserves unrelated runtime rows", () => {
  const existing = runtime({
    relayUrl: "wss://other.example",
    lifecycle: "ready",
  });
  const discovered = runtime();

  assert.deepEqual(
    mergeManagedAgentRuntimeStatuses([], [existing], [discovered]),
    [discovered, existing],
  );
});
