import assert from "node:assert/strict";
import { mock } from "node:test";
import test from "node:test";

import { installHookTestDom } from "./hookTestDom.mjs";

installHookTestDom();

const React = await import("react");
const { act } = React;
const { createRoot } = await import("react-dom/client");
const { relayClient } = await import("@/shared/api/relayClient");
const { useCommandConsoleStatus, useFreshCommandConsoleLocalCompute } =
  await import("./useCommandConsoleStatus.ts");

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
      await act(async () => {
        root.render(React.createElement(Harness));
      });
    },
    async unmount() {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

function runningServeStatus() {
  return {
    apiBaseUrl: null,
    consoleUrl: null,
    health: { status: "ok" },
    mode: "serve",
    modelId: "qwen",
    modelName: "Qwen",
    state: "running",
  };
}

test("Command Console exposes reconnecting and stalled transitions without the generic debounce", async () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  const emitter = relayClient.connectionStateEmitter;
  emitter.set("connected");
  const hook = renderHook(() => useCommandConsoleStatus());

  try {
    await hook.mount();
    assert.equal(hook.value.liveServices[0].state, "connected");

    await act(async () => {
      emitter.set("reconnecting");
    });
    assert.equal(hook.value.liveServices[0].state, "degraded");

    await act(async () => {
      emitter.set("stalled");
    });
    assert.equal(hook.value.liveServices[0].state, "degraded");
  } finally {
    await hook.unmount();
    emitter.set("idle");
    mock.timers.reset();
  }
});

test("a last successful local-compute probe expires at the freshness deadline", async () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  const probe = {
    error: null,
    status: runningServeStatus(),
  };
  const hook = renderHook(() =>
    useFreshCommandConsoleLocalCompute(probe, { freshnessMs: 5_000 }),
  );

  try {
    await hook.mount();
    assert.equal(hook.value.status?.state, "running");
    assert.equal(hook.value.error, null);

    await act(async () => {
      mock.timers.tick(5_001);
    });

    assert.equal(hook.value.status, null);
    assert.match(hook.value.error, /stale|deadline/i);
  } finally {
    await hook.unmount();
    mock.timers.reset();
  }
});
