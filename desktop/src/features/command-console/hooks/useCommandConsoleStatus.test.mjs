import assert from "node:assert/strict";
import test from "node:test";

import { createCommandConsoleStatusViewModel } from "./useCommandConsoleStatus.ts";

function meshStatus({ state, health = { status: "ok" }, modelName = null }) {
  return {
    apiBaseUrl: null,
    consoleUrl: null,
    health,
    mode: state === "off" ? null : "serve",
    modelId: modelName,
    modelName,
    state,
  };
}

test("reports connected only after successful relay and local-compute probes", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    localCompute: {
      error: null,
      status: meshStatus({
        state: "running",
        modelName: "Qwen local",
      }),
    },
    relayConnection: "connected",
  });

  assert.deepEqual(viewModel.liveServices, [
    {
      detail: "Authenticated relay connection is active.",
      id: "relay",
      label: "Buzz relay",
      state: "connected",
      statusLabel: "Connected",
    },
    {
      detail: "Qwen local is running on this Mac.",
      id: "local-compute",
      label: "Local compute",
      state: "connected",
      statusLabel: "Connected",
    },
  ]);
});

test("reports degraded relay and local-compute health without calling either healthy", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    localCompute: {
      error: null,
      status: meshStatus({
        state: "running",
        health: {
          reason: "Worker heartbeat is delayed.",
          status: "degraded",
        },
      }),
    },
    relayConnection: "reconnecting",
  });

  assert.equal(viewModel.liveServices[0].state, "degraded");
  assert.equal(viewModel.liveServices[0].statusLabel, "Degraded");
  assert.match(viewModel.liveServices[0].detail, /reconnect/i);
  assert.equal(viewModel.liveServices[1].state, "degraded");
  assert.equal(viewModel.liveServices[1].statusLabel, "Degraded");
  assert.equal(
    viewModel.liveServices[1].detail,
    "Worker heartbeat is delayed.",
  );
});

test("reports unavailable when the current probe fails despite a previous status", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    localCompute: {
      error: "mesh_node_status is unavailable",
      status: meshStatus({ state: "running" }),
    },
    relayConnection: "idle",
  });

  assert.equal(viewModel.liveServices[0].state, "unavailable");
  assert.equal(viewModel.liveServices[0].statusLabel, "Unavailable");
  assert.match(viewModel.liveServices[0].detail, /not been established/i);
  assert.equal(viewModel.liveServices[1].state, "unavailable");
  assert.equal(viewModel.liveServices[1].statusLabel, "Unavailable");
  assert.equal(
    viewModel.liveServices[1].detail,
    "Status probe failed: mesh_node_status is unavailable",
  );
});

test("reports explicit offline states after successful terminal probes", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    localCompute: {
      error: null,
      status: meshStatus({ state: "off" }),
    },
    relayConnection: "disconnected",
  });

  assert.equal(viewModel.liveServices[0].state, "offline");
  assert.equal(viewModel.liveServices[0].statusLabel, "Offline");
  assert.equal(viewModel.liveServices[1].state, "offline");
  assert.equal(viewModel.liveServices[1].statusLabel, "Offline");
});

test("marks later-phase capabilities as not configured instead of probing or simulating them", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    localCompute: { error: null, status: null },
    relayConnection: "connecting",
  });

  assert.deepEqual(
    viewModel.laterCapabilities.map(({ label, state, statusLabel }) => ({
      label,
      state,
      statusLabel,
    })),
    [
      {
        label: "LM Studio",
        state: "not_configured",
        statusLabel: "Not configured",
      },
      {
        label: "Memory",
        state: "not_configured",
        statusLabel: "Not configured",
      },
      {
        label: "RAG",
        state: "not_configured",
        statusLabel: "Not configured",
      },
      {
        label: "Apple inputs",
        state: "not_configured",
        statusLabel: "Not configured",
      },
    ],
  );
});
