import assert from "node:assert/strict";
import { after, afterEach, beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node,
  getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
  localStorage: dom.window.localStorage,
  IS_REACT_ACT_ENVIRONMENT: true,
});

// Drive the real polling hooks deterministically, without services or sleeps.
const intervals = new Map();
let timerId = 0;
dom.window.setInterval = (callback, delay) => {
  const id = ++timerId;
  intervals.set(id, { callback, delay });
  return id;
};
dom.window.clearInterval = (id) => intervals.delete(id);

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function nodeStatus(state, mode = state === "off" ? null : "serve") {
  return {
    state,
    mode,
    health: { status: "ok" },
    modelId: mode === "serve" ? "Qwen3-8B-Q4_K_M" : null,
    modelName: null,
    apiBaseUrl: null,
    consoleUrl: null,
  };
}
let status, start, stop, commands;
dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
dom.window.__TAURI_INTERNALS__ = {
  transformCallback: () => 1,
  invoke: async (command) => {
    commands.push(command);
    switch (command) {
      case "mesh_node_status":
        return status;
      case "mesh_start_node":
        return start.promise;
      case "mesh_stop_node":
        return stop.promise;
      case "mesh_installed_models":
        return [];
      case "mesh_model_catalog":
        return { entries: [], recommended: "Qwen3-8B-Q4_K_M" };
      case "mesh_snapshot":
        return { contributorMemberCount: 0 };
      case "mesh_serving_usage":
        return null;
      case "plugin:event|listen":
        return 1;
      case "plugin:event|unlisten":
        return null;
      default:
        throw new Error(`Unexpected IPC: ${command}`);
    }
  },
};

const React = await import("react");
const { act, cleanup, fireEvent, render } = await import(
  "@testing-library/react"
);
const { CommunitiesProvider } = await import(
  "@/features/communities/useCommunities"
);
const { MeshComputeSettingsCard } = await import(
  "./MeshComputeSettingsCard.tsx"
);

beforeEach(() => {
  status = nodeStatus("off");
  start = deferred();
  stop = deferred();
  commands = [];
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  assert.equal(intervals.size, 0);
});
after(() => dom.window.close());

async function mount() {
  let view;
  await act(async () => {
    view = render(
      React.createElement(
        CommunitiesProvider,
        null,
        React.createElement(MeshComputeSettingsCard),
      ),
    );
  });
  return { view, toggle: view.getByTestId("mesh-share-compute-toggle") };
}
async function click(toggle) {
  await act(async () => fireEvent.click(toggle));
}
async function poll(next) {
  status = next;
  await act(async () => {
    // Snapshot polling is intentionally slower and must not gate the toggle.
    for (const { callback, delay } of [...intervals.values()]) {
      if (delay <= 4000) callback();
    }
  });
}

for (const lateResult of ["success", "failure"]) {
  test(`ready unlocks stop without remount; late start ${lateResult} cannot clear stopping`, async () => {
    const { view, toggle } = await mount();
    assert.equal(toggle.disabled, false);
    await click(toggle);
    assert.equal(toggle.disabled, true);
    await poll(nodeStatus("starting"));
    assert.equal(toggle.getAttribute("aria-checked"), "true");
    assert.equal(toggle.disabled, true);
    const snapshotsBeforeReady = commands.filter(
      (c) => c === "mesh_snapshot",
    ).length;
    await poll(nodeStatus("running"));
    assert.equal(view.getByTestId("mesh-share-compute-toggle"), toggle);
    assert.equal(
      toggle.disabled,
      false,
      "live readiness must retire the pending start",
    );
    assert.ok(view.getByText("You’re sharing compute"));
    assert.ok(
      commands.filter((c) => c === "mesh_snapshot").length >
        snapshotsBeforeReady,
    );

    await click(toggle);
    assert.equal(commands.filter((c) => c === "mesh_stop_node").length, 1);
    assert.equal(toggle.disabled, true);
    assert.ok(view.getByText("Stopping…"));
    await poll(nodeStatus("stopping"));
    await act(async () => {
      if (lateResult === "success") start.resolve(nodeStatus("running"));
      else start.reject(new Error("obsolete start failure"));
    });
    assert.equal(
      toggle.disabled,
      true,
      "old finally must not unlock the new action",
    );
    assert.equal(toggle.getAttribute("aria-checked"), "false");
    assert.equal(view.queryByText("obsolete start failure"), null);
    // Off can be visible before stop finishes persisting the disabled config.
    await poll(nodeStatus("off"));
    assert.equal(toggle.disabled, true);
    await act(async () => stop.resolve(status));
    assert.equal(toggle.disabled, false);
    start = deferred();
    await click(toggle);
    assert.equal(commands.filter((c) => c === "mesh_start_node").length, 2);
    assert.equal(toggle.disabled, true);
    await poll(nodeStatus("running"));
    assert.equal(toggle.disabled, false);
  });
}

test("a failed start can be stopped and a rejected stop can be retried", async () => {
  const { view, toggle } = await mount();
  await click(toggle);
  await poll({
    ...nodeStatus("failed"),
    health: { status: "failed", reason: "model failed" },
  });
  assert.equal(toggle.disabled, false);
  assert.ok(view.getByText("model failed"));
  await click(toggle);
  await act(async () => stop.reject(new Error("stop failed")));
  assert.equal(toggle.disabled, false);
  assert.ok(view.getByText("stop failed"));
  stop = deferred();
  await click(toggle);
  assert.equal(commands.filter((c) => c === "mesh_stop_node").length, 2);
  assert.equal(view.queryByText("stop failed"), null);
  status = nodeStatus("off");
  await act(async () => stop.resolve(status));
  assert.equal(toggle.disabled, false);
});

test("rejected start unlocks retry and polled stopping blocks a new start", async () => {
  const { view, toggle } = await mount();
  await click(toggle);
  await act(async () => start.reject(new Error("start failed")));
  assert.equal(toggle.disabled, false);
  assert.ok(view.getByText("start failed"));
  await poll(nodeStatus("stopping"));
  assert.equal(toggle.disabled, true);
  await poll(nodeStatus("off"));
  start = deferred();
  await click(toggle);
  assert.equal(commands.filter((c) => c === "mesh_start_node").length, 2);
  assert.equal(view.queryByText("start failed"), null);
});
