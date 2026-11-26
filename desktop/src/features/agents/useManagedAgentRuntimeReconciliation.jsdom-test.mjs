import assert from "node:assert/strict";
import test from "node:test";

const calls = [];
let resolveFirstReconcile;
let reconcileHandler = null;
const tauriMock = {
  invoke(command, args) {
    calls.push(command);
    if (command === "reconcile_managed_agent_runtimes") {
      if (reconcileHandler) {
        return reconcileHandler(args);
      }
      if (!resolveFirstReconcile) {
        return new Promise((resolve) => {
          resolveFirstReconcile = resolve;
        });
      }
      return Promise.resolve([]);
    }
    if (command === "stop_managed_agent_runtime") {
      return Promise.resolve({ ...args, lifecycle: "stopped" });
    }
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback() {
    return Math.random();
  },
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
globalThis.window.__TAURI_INTERNALS__ = tauriMock;

const React = (await import("react")).default;
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
  "@tanstack/react-query"
);
const { useManagedAgentRuntimeReconciliation } = await import(
  "./useManagedAgentRuntimeReconciliation.ts"
);
const { markRelayRemoved } = await import("./managedAgentRelayCleanup.ts");

const RELAY = "wss://removed.example";

function Harness({ communities }) {
  useManagedAgentRuntimeReconciliation(communities);
  return null;
}

test("a reconcile that finishes after its community was removed does not block re-adding it", async () => {
  const queryClient = new QueryClient();
  const root = createRoot(document.createElement("div"));
  const render = (communities) =>
    act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Harness, { communities }),
        ),
      );
    });
  const reconciles = () =>
    calls.filter((command) => command === "reconcile_managed_agent_runtimes")
      .length;

  await render([{ relayUrl: RELAY }]);
  assert.equal(reconciles(), 1);

  // The community is removed while its reconcile is still in flight.
  markRelayRemoved(RELAY);
  await render([]);

  // The late reconcile finishes with a pair on the removed relay.
  await act(async () => {
    resolveFirstReconcile([
      { pubkey: "a".repeat(64), relayUrl: RELAY, lifecycle: "ready" },
    ]);
  });
  assert.ok(calls.includes("stop_managed_agent_runtime"));

  // Re-adding the community must reconcile it again.
  await render([{ relayUrl: RELAY }]);
  assert.equal(reconciles(), 2);

  await act(async () => root.unmount());
});

test("network online event immediately retries failing relays", async () => {
  const queryClient = new QueryClient();
  const root = createRoot(document.createElement("div"));
  const render = (communities) =>
    act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Harness, { communities }),
        ),
      );
    });

  const FAILING_RELAY = "wss://failing.example";
  let fail = true;
  let reconcileCount = 0;
  reconcileHandler = async () => {
    reconcileCount += 1;
    if (fail) {
      return [
        {
          pubkey: "b".repeat(64),
          relayUrl: FAILING_RELAY,
          lifecycle: "failed",
          error: "relay unreachable",
        },
      ];
    }
    return [
      {
        pubkey: "b".repeat(64),
        relayUrl: FAILING_RELAY,
        lifecycle: "ready",
      },
    ];
  };

  await render([{ relayUrl: FAILING_RELAY }]);
  assert.equal(reconcileCount, 1);

  // Still failing, timer is set for backoff.
  // Dispatch "online" event on window — must trigger immediate retry.
  fail = false;
  await act(async () => {
    window.dispatchEvent(new Event("online"));
  });

  assert.equal(reconcileCount, 2);

  await act(async () => root.unmount());
  reconcileHandler = null;
});
