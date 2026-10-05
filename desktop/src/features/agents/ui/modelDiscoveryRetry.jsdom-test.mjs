import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { usePersonaModelDiscovery } from "./usePersonaModelDiscovery.ts";
import { ModelDiscoveryStatusLine } from "./ModelDiscoveryStatusLine.tsx";
import { MODEL_DISCOVERY_SLOW_MS } from "./personaModelDiscoveryStatus.ts";

const envVars = {};
const selectedRuntime = {
  command: "codex",
  availability: "available",
  label: "Codex",
};
const response = (models = [{ id: "gpt-test", name: "Test model" }]) => ({
  agentName: "Codex",
  agentVersion: "1",
  models,
  supportsSwitching: true,
  agentDefaultModel: null,
  selectedModel: null,
});
let container,
  root,
  requests,
  timers,
  sequence,
  originalSetTimeout,
  originalClearTimeout;
function Harness({ provider = "", open = true }) {
  const state = usePersonaModelDiscovery({
    envVars,
    selectedRuntime,
    provider,
    open,
    modelFieldVisible: true,
    isCustomProviderEditing: false,
  });
  return React.createElement(
    "div",
    null,
    React.createElement(
      "output",
      null,
      state.modelDiscoveryLoading
        ? "Loading models…"
        : JSON.stringify(state.discoveredModelOptions),
    ),
    React.createElement(ModelDiscoveryStatusLine, {
      loading: state.modelDiscoveryLoading,
      loadingMessage: state.modelDiscoveryLoadingMessage,
      status: state.modelDiscoveryStatus,
      onRetry: state.retryModelDiscovery,
    }),
  );
}
const render = (props = {}) =>
  act(async () => {
    root.render(React.createElement(Harness, props));
  });
const settle = (index, value = response(), error = false) =>
  act(async () => {
    requests[index][error ? "reject" : "resolve"](value);
  });
const runTimers = (delay) =>
  act(async () => {
    for (const [id, timer] of [...timers])
      if (timer.delay === delay) {
        timers.delete(id);
        timer.callback();
      }
  });
beforeEach(() => {
  requests = [];
  timers = new Map();
  sequence = 0;
  originalSetTimeout = window.setTimeout;
  originalClearTimeout = window.clearTimeout;
  window.setTimeout = (callback, delay) => {
    timers.set(++sequence, { callback, delay });
    return sequence;
  };
  window.clearTimeout = (id) => timers.delete(id);
  window.__TAURI_INTERNALS__ = {
    invoke: (command, args) => {
      assert.equal(command, "discover_agent_models");
      return new Promise((resolve, reject) =>
        requests.push({ args, resolve, reject }),
      );
    },
  };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  if (root) await act(async () => root.unmount());
  container.remove();
  window.setTimeout = originalSetTimeout;
  window.clearTimeout = originalClearTimeout;
  delete window.__TAURI_INTERNALS__;
});
test("rendered timeout Retry repeats the same discovery and clears error on success", async () => {
  await render();
  assert.equal(requests.length, 1);
  assert.equal(
    container.querySelector('[data-testid="model-discovery-status"]'),
    null,
  );
  await settle(0, new Error("agent timed out (45s)"), true);
  const retry = container.querySelector("button");
  assert.equal(retry?.textContent, "Retry");
  await act(async () => retry.click());
  assert.equal(requests.length, 2);
  assert.deepEqual(requests[1].args, requests[0].args);
  assert.equal(container.querySelector("button"), null);
  await settle(1);
  assert.match(container.textContent, /Test model/);
  assert.doesNotMatch(container.textContent, /timed out|Still loading/);
  assert.equal(timers.size, 0);
});
test("slow note starts after ten seconds and restarts on retry", async () => {
  await render();
  assert.doesNotMatch(container.textContent, /Still loading/);
  assert.equal(
    [...timers.values()].filter((t) => t.delay === MODEL_DISCOVERY_SLOW_MS)
      .length,
    1,
  );
  await runTimers(MODEL_DISCOVERY_SLOW_MS);
  assert.match(container.textContent, /Still loading/);
  await settle(0, new Error("agent timed out"), true);
  await act(async () => container.querySelector("button").click());
  assert.doesNotMatch(container.textContent, /Still loading/);
  await runTimers(MODEL_DISCOVERY_SLOW_MS);
  assert.match(container.textContent, /Still loading/);
  await settle(1);
  assert.equal(timers.size, 0);
});
test("provider switch resets timer and ignores the prior response", async () => {
  await render();
  await runTimers(MODEL_DISCOVERY_SLOW_MS);
  await render({ provider: "anthropic" });
  assert.doesNotMatch(container.textContent, /Still loading/);
  await runTimers(250);
  assert.equal(requests.length, 2);
  await settle(0);
  assert.match(container.textContent, /Loading models/);
  assert.doesNotMatch(container.textContent, /Test model/);
  await settle(1, response([{ id: "claude-test", name: "Current model" }]));
  assert.match(container.textContent, /Current model/);
  assert.equal(timers.size, 0);
});
test("closing fences non-debounced discovery and reopening probes again", async () => {
  await render();
  await render({ open: false });
  assert.equal(timers.size, 0);
  await settle(0);
  await render();
  assert.equal(requests.length, 2);
  assert.doesNotMatch(container.textContent, /Test model/);
  await settle(1);
});
test("unmount clears the slow timer while IPC is pending", async () => {
  await render();
  assert.equal(timers.size, 1);
  await act(async () => root.unmount());
  root = null;
  assert.equal(timers.size, 0);
  await settle(0);
  assert.equal(container.textContent, "");
});
test("empty response offers a real retry without caching the empty result", async () => {
  await render();
  await settle(0, response([]));
  assert.match(container.textContent, /reported no models/);
  await act(async () => container.querySelector("button").click());
  assert.equal(requests.length, 2);
  await settle(1);
  assert.match(container.textContent, /Test model/);
});
test("Databricks sign-in timeout retains auth guidance without generic retry", async () => {
  await render({ provider: "databricks_v2" });
  await runTimers(250);
  await settle(0, new Error("Databricks sign-in timed out"), true);
  assert.match(container.textContent, /sign-in didn't complete/);
  assert.equal(container.querySelector("button"), null);
});
