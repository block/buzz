import assert from "node:assert/strict";
import test from "node:test";

import { buildAgentHealthSnapshot } from "./agentHealth.ts";

const agent = {
  pubkey: "a".repeat(64),
  name: "Hermes",
  personaId: null,
  relayUrl: "wss://buzz.example",
  acpCommand: "buzz-acp",
  agentCommand: "codex-acp",
  agentCommandOverride: null,
  agentArgs: [],
  mcpCommand: "",
  turnTimeoutSeconds: 320,
  idleTimeoutSeconds: null,
  maxTurnDurationSeconds: null,
  parallelism: 1,
  systemPrompt: "Ship carefully.",
  avatarUrl: "https://example.com/hermes.jpg",
  model: "gpt-5",
  modelSource: "definition",
  provider: "openai",
  personaOutOfDate: false,
  personaOrphaned: false,
  needsRestart: false,
  envVars: {},
  status: "running",
  pid: 42,
  createdAt: "2026-07-26T10:00:00Z",
  updatedAt: "2026-07-26T11:00:00Z",
  lastStartedAt: "2026-07-26T12:00:00Z",
  lastStoppedAt: null,
  lastExitCode: null,
  lastError: null,
  lastErrorCode: null,
  logPath: "/tmp/hermes.log",
  startOnAppLaunch: true,
  autoRestartOnConfigChange: true,
  backend: { type: "local" },
  backendAgentId: null,
  respondTo: "owner-only",
  respondToAllowlist: [],
};

test("buildAgentHealthSnapshot exposes known fields and honest contract gaps", () => {
  const snapshot = buildAgentHealthSnapshot({
    agent,
    channels: [{ id: "system", name: "System" }],
    channelsError: false,
    channelsLoading: false,
    presenceLoaded: true,
    presenceStatus: "online",
  });

  assert.equal(
    snapshot.fields.find((field) => field.key === "runtime")?.value,
    "codex-acp",
  );
  assert.equal(
    snapshot.fields.find((field) => field.key === "channels")?.value,
    "#System",
  );
  assert.equal(
    snapshot.fields.find((field) => field.key === "configuration-version")
      ?.availability,
    "unavailable",
  );
  assert.match(
    snapshot.fields.find((field) => field.key === "last-successful-mention")
      ?.detail ?? "",
    /does not currently persist/,
  );
});

test("buildAgentHealthSnapshot distinguishes loading, unavailable, and warnings", () => {
  const snapshot = buildAgentHealthSnapshot({
    agent: {
      ...agent,
      avatarUrl: null,
      model: null,
      provider: null,
      personaOrphaned: true,
      needsRestart: true,
      lastError: "Authentication failed",
      lastStartedAt: null,
    },
    channels: [],
    channelsError: true,
    channelsLoading: false,
    presenceLoaded: false,
    presenceStatus: undefined,
  });

  assert.equal(
    snapshot.fields.find((field) => field.key === "channels")?.availability,
    "unavailable",
  );
  assert.equal(
    snapshot.fields.find((field) => field.key === "model")?.availability,
    "unknown",
  );
  assert.deepEqual(
    snapshot.warnings.map((warning) => warning.key),
    ["persona-orphaned", "restart", "last-error"],
  );
});
