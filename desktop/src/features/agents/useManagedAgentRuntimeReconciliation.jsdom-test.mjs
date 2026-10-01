import assert from "node:assert/strict";
import test from "node:test";

const calls = [];
let resolveFirstReconcile;
const tauriMock = {
  invoke(command, args) {
    calls.push(command);
    if (command === "reconcile_managed_agent_runtimes") {
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
