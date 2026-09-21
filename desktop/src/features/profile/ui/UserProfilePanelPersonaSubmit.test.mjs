import assert from "node:assert/strict";
import test from "node:test";

import {
  profileCreateResultKind,
  submitProfilePersonaDialog,
  validateLinkedAgentRuntimeEdit,
} from "./UserProfilePanelPersonaSubmit.ts";

function agent(overrides = {}) {
  return {
    pubkey: "deadbeef".repeat(8),
    name: "Fizz",
    personaId: "persona-1",
    relayUrl: "ws://localhost:3000",
    acpCommand: "buzz-acp",
    agentCommand: "goose",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 320,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: "Prompt",
    avatarUrl: null,
    model: null,
    envVars: {},
    status: "stopped",
    pid: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    logPath: null,
    startOnAppLaunch: true,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

function persona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Fizz",
    avatarUrl: null,
    description: null,
    systemPrompt: "Prompt",
    runtime: "goose",
    model: null,
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    envVars: {},
    respondTo: "owner-only",
    respondToAllowlist: [],
    parallelism: 3,
    sessionPolicy: "thread",
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

test("submitProfilePersonaDialog saves a linked definition through one backend command", async () => {
  const personaUpdates = [];
  let done = false;

  await submitProfilePersonaDialog({
    createManagedAgentForPersona: async () => {
      throw new Error("not used");
    },
    createPersona: async () => {
      throw new Error("not used");
    },
    input: updateInput({
      runtime: "goose",
      behavior: { parallelism: 3 },
    }),
    managedAgent: agent({ parallelism: 10 }),
    onDone: () => {
      done = true;
    },
    previousPersona: persona({ parallelism: 10 }),
    updatePersona: async (input) => {
      personaUpdates.push(input);
      return persona({ parallelism: 3 });
    },
  });

  assert.equal(personaUpdates.length, 1);
  assert.equal(done, true);
});

test("a reused identity with pending policy sync is never classified as success", () => {
  const created = {
    agent: agent({ status: "running" }),
    privateKeyNsec: "nsec1test",
    spawnError: null,
    profileSyncError: "Managed policy sync is still pending",
  };

  assert.equal(profileCreateResultKind(created), "pending");
  assert.equal(
    profileCreateResultKind({ ...created, profileSyncError: null }),
    "success",
  );
  assert.equal(
    profileCreateResultKind({ ...created, spawnError: "not running" }),
    "error",
  );
});

function updateInput(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Fizz",
    avatarUrl: undefined,
    systemPrompt: "Prompt",
    runtime: "claude",
    model: undefined,
    provider: undefined,
    namePool: [],
    ...overrides,
  };
}

function runtime(overrides = {}) {
  return {
    id: "claude",
    label: "Claude Code",
    avatarUrl: "",
    availability: "available",
    command: "claude",
    binaryPath: "/usr/local/bin/claude",
    defaultArgs: [],
    mcpCommand: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
    ...overrides,
  };
}

test("validateLinkedAgentRuntimeEdit allows available runtime changes", () => {
  assert.equal(
    validateLinkedAgentRuntimeEdit({
      input: updateInput({ runtime: "claude" }),
      managedAgent: agent(),
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [runtime()],
    }),
    null,
  );
});

test("validateLinkedAgentRuntimeEdit rejects unavailable linked-agent runtime changes", () => {
  assert.equal(
    validateLinkedAgentRuntimeEdit({
      input: updateInput({ runtime: "claude" }),
      managedAgent: agent(),
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [runtime({ availability: "cli_missing", command: null })],
    }),
    "Claude Code is not available. Install it before saving this linked agent.",
  );
});

test("validateLinkedAgentRuntimeEdit allows unchanged or unlinked runtime preferences", () => {
  assert.equal(
    validateLinkedAgentRuntimeEdit({
      input: updateInput({ runtime: "goose" }),
      managedAgent: agent(),
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [],
    }),
    null,
  );

  assert.equal(
    validateLinkedAgentRuntimeEdit({
      input: updateInput({ runtime: "claude" }),
      managedAgent: undefined,
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [],
    }),
    null,
  );
});
