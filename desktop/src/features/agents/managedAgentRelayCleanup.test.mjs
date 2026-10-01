import assert from "node:assert/strict";
import test, { mock } from "node:test";

import {
  markRelayRemoved,
  readmitRelay,
  reconcileConfiguredManagedAgentRuntimes,
  refuseRelayAdmission,
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

test("reconcile stops live pairs on a relay removed mid-flight and reports it", async () => {
  const stopped = [];
  const { runtimes, removedRelays } =
    await reconcileConfiguredManagedAgentRuntimes(
      [
        { relayUrl: "wss://Dead.example/" },
        { relayUrl: "wss://alive.example" },
      ],
      {
        reconcile: async () => {
          // The community is removed while the reconcile is in flight.
          markRelayRemoved("wss://dead.example");
          return [
            pair("a", "wss://dead.example"),
            pair("b", "wss://dead.example", "failed"),
            pair("c", "wss://alive.example"),
          ];
        },
        stop: async (pubkey, relayUrl) => stopped.push([pubkey, relayUrl]),
      },
    );
  // The failed row has no live child, so it is left alone.
  assert.deepEqual(stopped, [["a", "wss://dead.example"]]);
  assert.deepEqual([...removedRelays], ["wss://dead.example"]);
  assert.equal(runtimes.length, 3);
});

test("reconcile stops nothing without a removal, whatever storage holds", async () => {
  // A storage failure used to look like "every relay removed". The fence now
  // needs a removal recorded during the call, so it stops nothing here, even
  // for a relay removed before the call started.
  markRelayRemoved("wss://earlier.example");
  const stopped = [];
  const { removedRelays } = await reconcileConfiguredManagedAgentRuntimes(
    [
      { relayUrl: "wss://earlier.example" },
      { relayUrl: "wss://alive.example" },
    ],
    {
      reconcile: async () => [
        pair("a", "wss://earlier.example"),
        pair("b", "wss://alive.example"),
      ],
      stop: async (pubkey) => stopped.push(pubkey),
    },
  );
  assert.deepEqual(stopped, []);
  assert.equal(removedRelays.size, 0);
});

test("removing a 127.* hostname does not fence the real loopback relay", async () => {
  const stopped = [];
  const { removedRelays } = await reconcileConfiguredManagedAgentRuntimes(
    [
      { relayUrl: "wss://127.preview.example" },
      { relayUrl: "ws://127.0.0.1:3000" },
    ],
    {
      reconcile: async () => {
        markRelayRemoved("wss://127.preview.example");
        return [
          pair("a", "wss://127.preview.example"),
          pair("b", "ws://127.0.0.1:3000"),
        ];
      },
      stop: async (pubkey) => stopped.push(pubkey),
    },
  );
  assert.deepEqual(stopped, ["a"]);
  assert.deepEqual([...removedRelays], ["wss://127.preview.example"]);
});

test("a failed admission write rejects for its caller and the queue keeps running", async () => {
  // No Tauri runtime here, so every native admission write fails.
  await assert.rejects(refuseRelayAdmission("wss://write-fails.example"));
  await assert.rejects(readmitRelay("wss://write-fails.example"));
  // The queue recovered: a reconcile waiting on it still runs.
  const { runtimes } = await reconcileConfiguredManagedAgentRuntimes([], {
    list: async () => [],
    reconcile: async () => [],
    stop: async () => {},
  });
  assert.deepEqual(runtimes, []);
});
