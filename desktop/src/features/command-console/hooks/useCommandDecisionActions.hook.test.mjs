import assert from "node:assert/strict";
import { mock } from "node:test";
import test from "node:test";

import { installHookTestDom } from "./hookTestDom.mjs";

installHookTestDom();

const React = await import("react");
const { act } = React;
const { createRoot } = await import("react-dom/client");
const { useCommandDecisionActions } = await import(
  "./useCommandDecisionActions.ts"
);

const decision = {
  key: "run-1:action-1",
  runId: "run-1",
  actionId: "action-1",
  adviser: "operations",
  coaA: "Complete the readiness review today.",
  coaB: "Defer the review until tomorrow.",
};

function renderHook(useValue) {
  let value;
  const root = createRoot(document.createElement("div"));
  function Harness() {
    value = useValue();
    return null;
  }
  return {
    get value() {
      return value;
    },
    async mount() {
      await act(async () => root.render(React.createElement(Harness)));
    },
    async unmount() {
      await act(async () => root.unmount());
    },
  };
}

test("dispatches direction and tracks active, complete, and persisted state", async () => {
  let now = 1000;
  let liveListener;
  let watchdog;
  let active = false;
  const stored = new Map();
  const send = mock.fn(async () => {});
  const deps = {
    openChief: mock.fn(async () => ({
      pubkey: "a".repeat(64),
      channelId: "00000000-0000-4000-8000-000000000001",
    })),
    send,
    navigate: mock.fn(async () => {}),
    subscribe: mock.fn(async (_channelId, listener) => {
      liveListener = listener;
      return async () => {};
    }),
    hasActiveTurn: () => active,
    now: () => now,
    storage: {
      getItem: (key) => stored.get(key) ?? null,
      setItem: (key, value) => stored.set(key, value),
    },
    setInterval: (callback) => {
      watchdog = callback;
      return 1;
    },
    clearInterval: () => {},
  };
  const hook = renderHook(() => useCommandDecisionActions(deps));

  try {
    await hook.mount();
    await act(async () => {
      await hook.value.issue(decision, decision.coaA, "coa_a");
    });

    assert.equal(send.mock.callCount(), 1);
    assert.equal(hook.value.executions[0].status, "queued");
    assert.match(stored.values().next().value, /run-1:action-1/);

    active = true;
    now = 2000;
    await act(async () => watchdog());
    assert.equal(hook.value.executions[0].status, "in_progress");

    await act(async () => {
      liveListener({
        pubkey: "a".repeat(64),
        content: "CO DIRECTION run-1:action-1 — COMPLETE\nChecklist published.",
      });
    });
    assert.equal(hook.value.executions[0].status, "completed");
    assert.equal(hook.value.executions[0].statusText, "Checklist published.");
  } finally {
    await hook.unmount();
  }
});

test("marks silent queued work stalled and supports retry", async () => {
  let now = 1000;
  let watchdog;
  const send = mock.fn(async () => {});
  const deps = {
    openChief: async () => ({
      pubkey: "a".repeat(64),
      channelId: "00000000-0000-4000-8000-000000000001",
    }),
    send,
    navigate: async () => {},
    subscribe: async () => async () => {},
    hasActiveTurn: () => false,
    now: () => now,
    storage: { getItem: () => null, setItem: () => {} },
    setInterval: (callback) => {
      watchdog = callback;
      return 1;
    },
    clearInterval: () => {},
  };
  const hook = renderHook(() => useCommandDecisionActions(deps));

  try {
    await hook.mount();
    await act(async () => {
      await hook.value.issue(decision, decision.coaB, "coa_b");
    });
    now += 5 * 60_000;
    await act(async () => watchdog());
    assert.equal(hook.value.executions[0].status, "stalled");

    await act(async () => {
      await hook.value.retry(decision, hook.value.executions[0]);
    });
    assert.equal(send.mock.callCount(), 2);
    assert.equal(hook.value.executions[0].status, "queued");
  } finally {
    await hook.unmount();
  }
});
