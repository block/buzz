import assert from "node:assert/strict";
import test from "node:test";

import {
  parseProfilePanelView,
  personaManagedAgentUpdate,
  profilePanelViewFromSearch,
} from "./UserProfilePanelUtils.ts";

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
    systemPrompt: "Old prompt",
    avatarUrl: "app-avatar://old",
    model: "old-model",
    mcpToolsets: null,
    envVars: { OLD_KEY: "1" },
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
    displayName: "Fizz Prime",
    avatarUrl: null,
    systemPrompt: "New prompt",
    runtime: "goose",
    model: "new-model",
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    envVars: { NEW_KEY: "2" },
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function runtime(overrides = {}) {
  return {
    id: "claude",
    label: "Claude Code",
    avatarUrl: "app-avatar://claude",
    availability: "available",
    command: "claude",
    binaryPath: "/usr/local/bin/claude",
    defaultArgs: ["mcp", "serve"],
    mcpCommand: "claude-mcp",
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
    ...overrides,
  };
}

test("personaManagedAgentUpdate syncs edited persona identity to linked agent", () => {
  assert.deepEqual(personaManagedAgentUpdate(agent(), persona()), {
    pubkey: "deadbeef".repeat(8),
    name: "Fizz Prime",
    avatarUrl: null,
    systemPrompt: "New prompt",
    model: "new-model",
    envVars: { NEW_KEY: "2" },
  });
});

test("personaManagedAgentUpdate skips unrelated or unchanged agents", () => {
  assert.equal(
    personaManagedAgentUpdate(agent({ personaId: "persona-2" }), persona()),
    null,
  );
  assert.equal(
    personaManagedAgentUpdate(
      agent({
        name: "Fizz Prime",
        avatarUrl: null,
        systemPrompt: "New prompt",
        model: "new-model",
        envVars: { NEW_KEY: "2" },
      }),
      persona(),
    ),
    null,
  );
});

test("personaManagedAgentUpdate maps changed persona runtime to linked agent commands", () => {
  assert.deepEqual(
    personaManagedAgentUpdate(agent(), persona({ runtime: "claude" }), {
      previousPersona: persona({ runtime: "goose" }),
      runtimes: [runtime()],
    }),
    {
      pubkey: "deadbeef".repeat(8),
      name: "Fizz Prime",
      avatarUrl: null,
      systemPrompt: "New prompt",
      model: "new-model",
      envVars: { NEW_KEY: "2" },
      agentCommand: "claude",
      agentArgs: ["mcp", "serve"],
      mcpCommand: "claude-mcp",
    },
  );
});

test("personaManagedAgentUpdate leaves runtime fields alone when runtime is unchanged", () => {
  assert.equal(
    personaManagedAgentUpdate(
      agent({
        name: "Fizz Prime",
        avatarUrl: null,
        systemPrompt: "New prompt",
        model: "new-model",
        envVars: { NEW_KEY: "2" },
        agentArgs: ["custom"],
      }),
      persona({ runtime: "goose" }),
      {
        previousPersona: persona({ runtime: "goose" }),
        runtimes: [runtime({ id: "goose", command: "goose" })],
      },
    ),
    null,
  );
});

test("parseProfilePanelView accepts all profile panel subviews", () => {
  for (const view of [
    "summary",
    "info",
    "configuration",
    "diagnostics",
    "memories",
    "channels",
    "logs",
  ]) {
    assert.equal(parseProfilePanelView(view), view);
  }
});

test("parseProfilePanelView maps legacy agent config subviews to configuration", () => {
  for (const view of ["instructions", "model", "settings"]) {
    assert.equal(parseProfilePanelView(view), "configuration");
  }
});

test("profilePanelViewFromSearch falls back to summary for invalid values", () => {
  assert.equal(parseProfilePanelView("missing"), null);
  assert.equal(profilePanelViewFromSearch("missing"), "summary");
  assert.equal(profilePanelViewFromSearch(null), "summary");
});
