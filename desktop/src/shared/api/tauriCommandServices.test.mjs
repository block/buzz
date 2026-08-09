import assert from "node:assert/strict";
import test from "node:test";

const calls = [];
let response;
globalThis.window = globalThis;
globalThis.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    calls.push({ command, args });
    return response;
  },
  transformCallback: () => 1,
};

const { getCommandKnowledgeStatus } = await import("./tauriCommandServices.ts");

const validStatus = {
  kind: "command-knowledge-status",
  version: 1,
  classification: "OFFICIAL",
  observedAt: "2026-07-24T00:00:00Z",
  memory: {
    status: "not_configured",
    serverIdentity: null,
    nodeId: null,
    homeNodeId: null,
    revisionCount: 0,
    conflictCount: 0,
    replicationCursor: null,
    homeReplicationCursor: null,
    lastSuccessfulSync: null,
    freshness: "unknown",
    validation: "unknown",
    toolAllowlist: [],
    error: null,
  },
  rag: {
    status: "not_configured",
    serverIdentity: null,
    activeSnapshotId: null,
    signatureFingerprint: null,
    snapshotTime: null,
    lastSuccessfulActivation: null,
    freshness: "unknown",
    validation: "unknown",
    toolAllowlist: [],
    error: null,
  },
  appleInputs: [],
  degradedSections: [],
};

test("getCommandKnowledgeStatus invokes the trusted metadata-only command", async () => {
  calls.length = 0;
  response = structuredClone(validStatus);

  const result = await getCommandKnowledgeStatus();

  assert.deepEqual(calls, [
    { command: "get_command_knowledge_status", args: {} },
  ]);
  assert.deepEqual(result, validStatus);
});

test("getCommandKnowledgeStatus does not reinterpret the native payload", async () => {
  response = { ...structuredClone(validStatus), bearerToken: "secret" };

  assert.deepEqual(await getCommandKnowledgeStatus(), response);
});
