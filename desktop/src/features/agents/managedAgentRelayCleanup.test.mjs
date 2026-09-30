import assert from "node:assert/strict";
import test, { mock } from "node:test";

import {
  reconcileConfiguredManagedAgentRuntimes,
  stopManagedAgentPairsOnRelay,
} from "./managedAgentRelayCleanup.ts";

function pair(pubkey, relayUrl, lifecycle = "ready") {
  return { pubkey, relayUrl, lifecycle };
}

test("stops every live pair on the removed relay and no others", async () => {
  const stopped = [];
  await stopManagedAgentPairsOnRelay("wss://Dead.example/", {
    list: async () => [
      pair("a", "wss://dead.example"),
      pair("b", "wss://dead.example", "failed"),
      pair("c", "wss://dead.example", "stopped"),
      pair("d", "wss://alive.example"),
    ],
    stop: async (pubkey, relayUrl) => stopped.push([pubkey, relayUrl]),
  });
  assert.deepEqual(stopped, [
    ["a", "wss://dead.example"],
    ["b", "wss://dead.example"],
  ]);
});

test("a failed stop is logged and does not throw", async () => {
  const warn = mock.method(console, "warn", () => {});
  const stopped = [];
  await stopManagedAgentPairsOnRelay("wss://dead.example", {
    list: async () => [
      pair("a", "wss://dead.example"),
      pair("b", "wss://dead.example"),
    ],
    stop: async (pubkey) => {
      if (pubkey === "a") throw new Error("boom");
      stopped.push(pubkey);
    },
  });
  assert.deepEqual(stopped, ["b"]);
  assert.equal(warn.mock.callCount(), 1);
  warn.mock.restore();
});

test("a failed runtime listing is logged and does not throw", async () => {
  const warn = mock.method(console, "warn", () => {});
  await stopManagedAgentPairsOnRelay("wss://dead.example", {
    list: async () => {
      throw new Error("ipc down");
    },
    stop: async () => assert.fail("nothing to stop"),
  });
  assert.equal(warn.mock.callCount(), 1);
  warn.mock.restore();
});

test("reconcile stops pairs it started on a relay removed mid-flight", async () => {
  const stopped = [];
  const runtimes = await reconcileConfiguredManagedAgentRuntimes(
    [{ relayUrl: "wss://dead.example" }, { relayUrl: "wss://alive.example" }],
    {
      reconcile: async () => [
        pair("a", "wss://dead.example"),
        pair("b", "wss://alive.example"),
      ],
      // The removal saved the list before the reconcile resolved.
      configuredRelayUrls: () => ["wss://Alive.example/"],
      stop: async (pubkey, relayUrl) => stopped.push([pubkey, relayUrl]),
    },
  );
  assert.deepEqual(stopped, [["a", "wss://dead.example"]]);
  assert.equal(runtimes.length, 2);
});
