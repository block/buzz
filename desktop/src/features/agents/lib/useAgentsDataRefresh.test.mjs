import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { KIND_MANAGED_AGENT } from "@/shared/constants/kinds";
import { startRelayAgentPolicyRefresh } from "./useAgentsDataRefresh.ts";

const coordinates = [
  { ownerPubkey: "owner-a", agentPubkey: "agent-a" },
  { ownerPubkey: "owner-b", agentPubkey: "agent-b" },
];

function event(pubkey, dTag) {
  return {
    id: "id",
    pubkey,
    created_at: 1,
    kind: KIND_MANAGED_AGENT,
    tags: dTag ? [["d", dTag]] : [],
    content: "{}",
    sig: "sig",
  };
}

test("remote managed policy refresh accepts only exact authenticated coordinates", async () => {
  let onEvent;
  let filter;
  let unsubscribeCalls = 0;
  mock.method(relayClient, "subscribeLive", (nextFilter, listener) => {
    filter = nextFilter;
    onEvent = listener;
    return Promise.resolve(() => {
      unsubscribeCalls += 1;
      return Promise.resolve();
    });
  });

  let refreshes = 0;
  const stop = startRelayAgentPolicyRefresh(coordinates, () => {
    refreshes += 1;
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(filter, {
    kinds: [KIND_MANAGED_AGENT],
    authors: ["owner-a", "owner-b"],
    "#d": ["agent-a", "agent-b"],
    limit: 0,
  });
  onEvent(event("owner-a", "agent-a"));
  assert.equal(refreshes, 1);

  for (const irrelevant of [
    event("owner-x", "agent-a"),
    event("owner-a", "agent-x"),
    event("owner-a", "agent-b"), // authors×d cross-product
    event("owner-a", null),
  ]) {
    onEvent(irrelevant);
  }
  assert.equal(refreshes, 1, "irrelevant coordinates must not refresh");

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

  const stop = startRelayAgentPolicyRefresh(coordinates, () => {});
  stop();
  resolveSubscription(() => {
    unsubscribeCalls += 1;
    return Promise.resolve();
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(unsubscribeCalls, 1);
  mock.reset();
});

test("no authenticated coordinates creates no global subscription", () => {
  let subscriptions = 0;
  mock.method(relayClient, "subscribeLive", () => {
    subscriptions += 1;
    return Promise.resolve(() => Promise.resolve());
  });
  startRelayAgentPolicyRefresh([], () => {})();
  assert.equal(subscriptions, 0);
  mock.reset();
});
