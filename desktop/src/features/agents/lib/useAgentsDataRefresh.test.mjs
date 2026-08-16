import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { KIND_MANAGED_AGENT } from "@/shared/constants/kinds";
import { startRelayAgentPolicyRefresh } from "./useAgentsDataRefresh.ts";

test("remote managed policy events refresh the relay agent directory", async () => {
  const calls = [];
  let onEvent;
  let unsubscribeCalls = 0;
  mock.method(relayClient, "subscribeLive", (filter, listener) => {
    calls.push(filter);
    onEvent = listener;
    return Promise.resolve(() => {
      unsubscribeCalls += 1;
      return Promise.resolve();
    });
  });

  let refreshes = 0;
  const stop = startRelayAgentPolicyRefresh(() => {
    refreshes += 1;
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(calls, [{ kinds: [KIND_MANAGED_AGENT], limit: 0 }]);
  onEvent({ kind: KIND_MANAGED_AGENT });
  assert.equal(refreshes, 1);

  stop();
  assert.equal(unsubscribeCalls, 1);
  mock.reset();
});

test("stopping before subscription readiness still closes the live query", async () => {
  let resolveSubscription;
  let unsubscribeCalls = 0;
  mock.method(
    relayClient,
    "subscribeLive",
    () =>
      new Promise((resolve) => {
        resolveSubscription = resolve;
      }),
  );

  const stop = startRelayAgentPolicyRefresh(() => {});
  stop();
  resolveSubscription(() => {
    unsubscribeCalls += 1;
    return Promise.resolve();
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(unsubscribeCalls, 1);
  mock.reset();
});
