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

function meshClientStatus() {
  return {
    ...meshStatus({ state: "running" }),
    mode: "client",
  };
}

function knowledgeStatus(overrides = {}) {
  return {
    kind: "command-knowledge-status",
    version: 1,
    classification: "OFFICIAL",
    observedAt: "2026-07-24T04:30:00Z",
    memory: {
      status: "ready",
      serverIdentity: "memory",
      nodeId: "node:command",
      homeNodeId: "node:home-command",
      revisionCount: 42,
      conflictCount: 2,
      replicationCursor: 41,
      homeReplicationCursor: 73,
      lastSuccessfulSync: "2026-07-24T04:20:00Z",
      freshness: "fresh",
      validation: "verified",
      toolAllowlist: ["get_entity", "recall_for_entity", "search_events"],
      error: null,
    },
    rag: {
      status: "ready",
      serverIdentity: "rag",
      activeSnapshotId: "f".repeat(64),
      signatureFingerprint: "e".repeat(64),
      snapshotTime: "2026-07-24T03:30:00Z",
      lastSuccessfulActivation: "2026-07-24T04:00:00Z",
      freshness: "fresh",
      validation: "verified",
      toolAllowlist: [
        "get_document",
        "get_snapshot_status",
        "list_collections",
        "search_knowledge_base",
      ],
      error: null,
    },
    appleInputs: [
      {
        source: "calendar",
        permission: "authorized",
        observedAt: "2026-07-24T04:30:00Z",
        recordCount: 0,
        truncated: false,
        error: null,
      },
      {
        source: "reminders",
        permission: "denied",
        observedAt: "2026-07-24T04:30:00Z",
        recordCount: 0,
        truncated: false,
        error: "permission_denied",
      },
      {
        source: "notes",
        permission: "authorized",
        observedAt: "2026-07-24T04:30:00Z",
        recordCount: 0,
        truncated: false,
        error: null,
      },
      {
        source: "files",
        permission: "authorized",
        observedAt: "2026-07-24T04:30:00Z",
        recordCount: 0,
        truncated: false,
        error: null,
      },
    ],
    degradedSections: ["apple-reminders", "memory-conflicts"],
    ...overrides,
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
      label: "Command workspace",
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
  assert.equal(
    viewModel.liveServices[1].detail,
    "Local compute is not running.",
  );
  assert.doesNotMatch(viewModel.liveServices[1].detail, /installed/i);
});

test("does not claim a healthy mesh client is serving local compute", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    localCompute: {
      error: null,
      status: meshClientStatus(),
    },
    relayConnection: "connected",
  });

  assert.equal(viewModel.liveServices[1].state, "unavailable");
  assert.equal(viewModel.liveServices[1].statusLabel, "Unavailable");
  assert.match(viewModel.liveServices[1].detail, /client/i);
  assert.doesNotMatch(viewModel.liveServices[1].detail, /running on this Mac/i);
});

test("does not treat daemon reachability without serve mode as local compute", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    localCompute: {
      error: null,
      status: {
        ...meshStatus({ state: "running" }),
        mode: null,
      },
    },
    relayConnection: "connected",
  });

  assert.equal(viewModel.liveServices[1].state, "unavailable");
  assert.equal(viewModel.liveServices[1].statusLabel, "Unavailable");
  assert.match(viewModel.liveServices[1].detail, /did not verify/i);
});

test("reports live verified Memory, RAG, and Apple status without content or secrets", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    knowledge: { error: null, status: knowledgeStatus() },
    lmStudio: { error: null, status: null },
    localCompute: { error: null, status: null },
    relayConnection: "connecting",
  });

  const memory = viewModel.liveServices.find(({ id }) => id === "memory");
  assert.equal(memory?.state, "degraded");
  assert.match(memory?.detail ?? "", /2 unresolved conflicts/i);
  assert.deepEqual(memory?.facts, [
    { label: "Node", value: "node:command" },
    { label: "Home node", value: "node:home-command" },
    { label: "Replication cursor", value: "41" },
    { label: "Home replication cursor", value: "73" },
    {
      label: "Last successful sync",
      value: "2026-07-24T04:20:00Z",
    },
    { label: "Revisions", value: "42" },
    { label: "Conflicts", value: "2" },
    { label: "Freshness", value: "Fresh" },
    { label: "Validation", value: "Verified" },
    {
      label: "Permissions",
      value: "get_entity, recall_for_entity, search_events",
    },
  ]);

  const rag = viewModel.liveServices.find(({ id }) => id === "rag");
  assert.equal(rag?.state, "connected");
  assert.match(rag?.detail ?? "", /signed active snapshot/i);
  assert.ok(
    rag?.facts?.some(
      ({ label, value }) =>
        label === "Active snapshot" && value === "f".repeat(64),
    ),
  );
  assert.ok(
    rag?.facts?.some(
      ({ label, value }) => label === "Validation" && value === "Verified",
    ),
  );

  const apple = viewModel.liveServices.find(({ id }) => id === "apple-inputs");
  assert.equal(apple?.state, "degraded");
  assert.deepEqual(apple?.facts, [
    { label: "Calendar", value: "Authorized" },
    { label: "Reminders", value: "Denied" },
    { label: "Notes", value: "Authorized" },
    { label: "Files", value: "Authorized" },
  ]);
  assert.deepEqual(viewModel.degradedSections, [
    "apple-reminders",
    "memory-conflicts",
  ]);

  const rendered = JSON.stringify(viewModel);
  assert.doesNotMatch(rendered, /fixture-token|private|quoted_text|records/i);
});

test("fails closed when native knowledge status is unavailable", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    knowledge: {
      error: "Bearer secret-token leaked by transport",
      status: knowledgeStatus(),
    },
    localCompute: { error: null, status: null },
    relayConnection: "connected",
  });

  for (const id of ["memory", "rag", "apple-inputs"]) {
    const service = viewModel.liveServices.find((item) => item.id === id);
    assert.equal(service?.state, "unavailable");
    assert.match(
      service?.detail ?? "",
      /native knowledge status probe failed/i,
    );
    assert.doesNotMatch(service?.detail ?? "", /secret-token|bearer/i);
  }
  assert.deepEqual(viewModel.degradedSections, ["knowledge-status"]);
});

test("reports a ready LM Studio route as connected, never mesh", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    lmStudio: {
      error: null,
      status: {
        bindExposure: "unknown",
        configuredModel: "qwen/qwen3.6-27b",
        detail: "Loaded model is ready; authentication is not enabled.",
        loadedModels: ["qwen/qwen3.6-27b"],
        securityWarnings: [
          "LM Studio API authentication is not enabled.",
          "LM Studio listener exposure is unverified.",
        ],
        status: "ready",
      },
    },
    localCompute: {
      error: "mesh_node_status is unavailable",
      status: null,
    },
    relayConnection: "connected",
  });

  const lmStudio = viewModel.liveServices.find(
    (service) => service.id === "lm-studio",
  );
  assert.deepEqual(lmStudio, {
    detail: "Loaded model is ready; authentication is not enabled.",
    id: "lm-studio",
    label: "LM Studio",
    state: "connected",
    statusLabel: "Connected",
  });
});

test("treats reachable trusted LAN knowledge as connected without requiring mesh compute", () => {
  const status = knowledgeStatus({
    version: 2,
    sourceMode: "trusted_lan",
    modelRoute: "local_litellm_openai",
    evidenceAssurance: "trusted_lan_observed",
    memory: {
      ...knowledgeStatus().memory,
      nodeId: null,
      homeNodeId: null,
      revisionCount: 0,
      conflictCount: 0,
      replicationCursor: null,
      homeReplicationCursor: null,
      lastSuccessfulSync: null,
      freshness: "observed",
      validation: "trusted_lan_observed",
      toolAllowlist: ["search_events"],
    },
    rag: {
      ...knowledgeStatus().rag,
      signatureFingerprint: null,
      freshness: "observed",
      validation: "trusted_lan_observed",
      toolAllowlist: ["list_collections", "search_knowledge_base"],
    },
    degradedSections: [],
  });
  const viewModel = createCommandConsoleStatusViewModel({
    knowledge: { error: null, status },
    lmStudio: { error: null, status: null },
    relayConnection: "connected",
  });

  assert.equal(
    viewModel.liveServices.some(({ id }) => id === "local-compute"),
    false,
  );
  assert.equal(
    viewModel.liveServices.find(({ id }) => id === "memory")?.state,
    "connected",
  );
  assert.equal(
    viewModel.liveServices.find(({ id }) => id === "rag")?.state,
    "connected",
  );
  assert.deepEqual(viewModel.degradedSections, []);
});

test("does not turn an accepted listener warning into a model outage", () => {
  const viewModel = createCommandConsoleStatusViewModel({
    lmStudio: {
      error: null,
      status: {
        bindExposure: "unknown",
        configuredModel: "qwen/qwen3.6-27b",
        detail: "Loaded LM Studio model is ready.",
        loadedModels: ["qwen/qwen3.6-27b"],
        securityWarnings: [],
        status: "ready",
      },
    },
    localCompute: { error: null, status: null },
    relayConnection: "connected",
  });

  const lmStudio = viewModel.liveServices.find(
    (service) => service.id === "lm-studio",
  );
  assert.equal(lmStudio?.state, "connected");
  assert.equal(lmStudio?.statusLabel, "Connected");
  assert.doesNotMatch(lmStudio?.detail ?? "", /listener exposure/i);
});
