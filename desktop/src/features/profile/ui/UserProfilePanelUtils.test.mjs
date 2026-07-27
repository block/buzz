import assert from "node:assert/strict";
import test from "node:test";

import {
  parseProfilePanelTab,
  parseProfilePanelView,
  personaManagedAgentUpdate,
  profilePanelTabFromSearch,
  profilePanelViewFromSearch,
  resolveProfileEditTarget,
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
  for (const view of ["model", "settings"]) {
    assert.equal(parseProfilePanelView(view), "configuration");
  }
});

test("profilePanelViewFromSearch falls back to summary for invalid values", () => {
  assert.equal(parseProfilePanelView("missing"), null);
  assert.equal(profilePanelViewFromSearch("missing"), "summary");
  assert.equal(profilePanelViewFromSearch(null), "summary");
});

test("parseProfilePanelTab accepts profile summary tabs", () => {
  for (const tab of ["info", "runtime", "channels", "memories"]) {
    assert.equal(parseProfilePanelTab(tab), tab);
  }
});

test("profilePanelTabFromSearch falls back to info for invalid values", () => {
  assert.equal(parseProfilePanelTab("missing"), null);
  assert.equal(profilePanelTabFromSearch("missing"), "info");
  assert.equal(profilePanelTabFromSearch(null), "info");
});

// ── Profile Edit routing: displayed policy must be the enforced policy ───────
//
// Regression: a persona-linked agent routed Edit to the DEFINITION editor, so
// the dialog showed the definition's inbound-author policy while the running
// agent still enforced the instance's own policy. A definition's behavior
// group is copied onto an instance only at mint time, so an owner who granted
// (or revoked) access there changed nothing about the live agent.

test("resolveProfileEditTarget: an instance-backed profile edits the instance", () => {
  assert.equal(
    resolveProfileEditTarget({
      hasManagedInstance: true,
      hasDefinition: true,
    }),
    "instance",
    "a persona-linked instance must still edit the instance it displays",
  );
  assert.equal(
    resolveProfileEditTarget({
      hasManagedInstance: true,
      hasDefinition: false,
    }),
    "instance",
  );
});

test("resolveProfileEditTarget: a definition-only profile edits the definition", () => {
  assert.equal(
    resolveProfileEditTarget({
      hasManagedInstance: false,
      hasDefinition: true,
    }),
    "definition",
    "with no minted instance the definition is the only editable record",
  );
});

test("resolveProfileEditTarget: no instance and no definition falls back to instance", () => {
  // Preserves the pre-existing fallback: the caller renders the instance
  // dialog only when a managed agent exists, so this is inert rather than a
  // route into a dialog that cannot edit anything.
  assert.equal(
    resolveProfileEditTarget({
      hasManagedInstance: false,
      hasDefinition: false,
    }),
    "instance",
  );
});
