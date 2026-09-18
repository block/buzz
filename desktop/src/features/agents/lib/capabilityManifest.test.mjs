import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAgentCapabilityManifest,
  findManifestRuntime,
} from "./capabilityManifest.ts";
import {
  getAgentObserverSnapshot,
  injectObserverEventsForE2E,
  resetAgentObserverStore,
} from "../observerRelayStore.ts";

const AGENT_PUBKEY = "ab".repeat(32);

function agent(overrides = {}) {
  return {
    pubkey: AGENT_PUBKEY,
    name: "TARS",
    personaId: null,
    teamId: null,
    relayUrl: "wss://relay.example.test",
    acpCommand: "buzz-acp",
    agentCommand: "codex-acp",
    agentCommandOverride: null,
    agentArgs: [],
    mcpCommand: "buzz-dev-mcp",
    turnTimeoutSeconds: 900,
    idleTimeoutSeconds: 900,
    maxTurnDurationSeconds: 7200,
    parallelism: 1,
    systemPrompt: null,
    avatarUrl: null,
    model: "configured-model",
    provider: "configured-provider",
    personaOutOfDate: false,
    personaOrphaned: false,
    needsRestart: false,
    envVars: {},
    status: "running",
    pid: 123,
    createdAt: "2026-07-25T23:00:00.000Z",
    updatedAt: "2026-07-26T00:00:00.000Z",
    lastStartedAt: "2026-07-26T01:00:00.000Z",
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    lastErrorCode: null,
    logPath: "/private/logs/agent.log",
    startOnAppLaunch: true,
    autoRestartOnConfigChange: true,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    credentialPersistence: "keyring_verified",
    ...overrides,
  };
}

function runtime(overrides = {}) {
  return {
    id: "codex",
    label: "Codex",
    avatarUrl: "",
    availability: "available",
    command: "codex-acp",
    binaryPath: "/private/bin/codex-acp",
    defaultArgs: [],
    mcpCommand: "buzz-dev-mcp",
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: true,
    requiresExternalCli: true,
    underlyingCliPath: "/private/bin/codex",
    nodeRequired: false,
    authStatus: { status: "logged_in" },
    loginHint: null,
    supportsAcpNativeConfig: false,
    supportsAcpModelSwitching: false,
    mcpHooks: false,
    ...overrides,
  };
}

function runtimeStatus(overrides = {}) {
  return {
    pubkey: AGENT_PUBKEY,
    relayUrl: "wss://relay.example.test",
    localSetup: true,
    lifecycle: "ready",
    pid: 123,
    error: null,
    logPath: null,
    ...overrides,
  };
}

function observerEvent(seq, timestamp, kind, payload, sessionId = null) {
  return {
    seq,
    timestamp,
    kind,
    agentIndex: 0,
    channelId: null,
    sessionId,
    turnId: null,
    payload,
  };
}

function manifest(overrides = {}) {
  return buildAgentCapabilityManifest({
    agent: agent(),
    runtime: runtime(),
    runtimeStatus: runtimeStatus(),
    presenceStatus: "online",
    observer: {
      connectionState: "open",
      events: [
        observerEvent(1, "2026-07-26T01:01:00.000Z", "agent_initialized", {
          initializeResult: {
            protocolVersion: 2,
            agentInfo: { name: "Codex ACP", version: "1.2.3" },
            agentCapabilities: {
              promptCapabilities: {
                image: true,
                audio: false,
              },
              tools: [
                {
                  name: "read_file",
                  source: "filesystem",
                  riskClass: "read",
                  available: true,
                  arguments: "argument-canary",
                },
                {
                  name: "/private/tool",
                  source: "filesystem",
                  riskClass: "execute",
                },
                {
                  name: "mystery_tool",
                  source: "runtime",
                  riskClass: "super-safe",
                },
              ],
            },
          },
        }),
        observerEvent(
          2,
          "2026-07-26T01:02:00.000Z",
          "session_config_captured",
          {
            models: {
              currentModelId: "observed-model",
              availableModels: [],
            },
            configOptions: {
              secret: "config-canary",
            },
            capabilityManifest: {
              modelApplication: {
                requested: "observed-model",
                applied: true,
              },
              toolSources: [
                { name: "github", kind: "mcp" },
                { name: "/private/mcp", kind: "mcp" },
              ],
              permissionMode: {
                requested: "bypassPermissions",
                effective: "perToolAutoDecision",
                source: "buzzHarness",
              },
            },
          },
        ),
        observerEvent(3, "2026-07-26T01:03:00.000Z", "acp_read", {
          method: "session/update",
          params: {
            update: {
              sessionUpdate: "available_commands_update",
              availableCommands: [
                { name: "create_plan", description: "description-canary" },
                { name: "/private/command" },
              ],
            },
          },
        }),
      ],
    },
    catalogObservedAt: "2026-07-26T01:04:00.000Z",
    runtimeObservedAt: "2026-07-26T01:04:30.000Z",
    ...overrides,
  });
}

test("builds a ready manifest from separately sourced live evidence", () => {
  const result = manifest();

  assert.equal(result.overallStatus, "ready");
  assert.equal(result.freshness, "fresh");
  assert.deepEqual(result.runtime, {
    id: "codex",
    label: "Codex ACP",
    version: "1.2.3",
  });
  assert.equal(result.protocolVersion, "2");
  assert.deepEqual(result.model, {
    value: "observed-model",
    source: "applied",
    requested: "observed-model",
    matchesRequested: true,
  });
  assert.deepEqual(result.provider, {
    value: "configured-provider",
    source: "configured",
  });
  assert.equal(
    result.features.find((feature) => feature.id === "image-input")?.state,
    "reported",
  );
  assert.equal(
    result.features.find((feature) => feature.id === "audio-input")?.state,
    "unavailable",
  );
  assert.equal(
    result.features.find((feature) => feature.id === "embedded-context")?.state,
    "unknown",
  );
  assert.deepEqual(result.commands, ["create_plan"]);
  assert.deepEqual(result.toolSources, ["github"]);
  assert.deepEqual(result.tools, [
    {
      name: "read_file",
      source: "filesystem",
      riskClass: "read",
      availability: "reported",
    },
    {
      name: "mystery_tool",
      source: "runtime",
      riskClass: "unknown",
      availability: "unknown",
    },
  ]);
  assert.deepEqual(result.permissionMode, {
    requested: "bypassPermissions",
    effective: "perToolAutoDecision",
    source: "buzzHarness",
  });
  assert.deepEqual(result.sessionEvidence, {
    sessionId: null,
    channelId: null,
  });
  assert.equal(
    result.limitations.includes(
      "This runtime depends on a separately installed vendor CLI.",
    ),
    true,
  );

  const serialized = JSON.stringify(result);
  for (const secret of [
    "argument-canary",
    "config-canary",
    "description-canary",
    "/private/mcp",
    "/private/command",
    "/private/tool",
  ]) {
    assert.equal(serialized.includes(secret), false);
  }
});

test("a stopped agent retains evidence but never claims fresh or ready", () => {
  const result = manifest({ agent: agent({ status: "stopped", pid: null }) });

  assert.equal(result.overallStatus, "stopped");
  assert.equal(result.freshness, "stale");
  assert.equal(
    result.readiness.find((check) => check.id === "process")?.status,
    "attention",
  );
});

test("an offline agent never claims ready for delegation", () => {
  const result = manifest({ presenceStatus: "offline" });

  assert.equal(result.overallStatus, "attention");
  assert.equal(
    result.readiness.find((check) => check.id === "presence")?.status,
    "attention",
  );
});

test("a failed current runtime makes cached capability evidence stale", () => {
  const result = manifest({
    runtimeStatus: runtimeStatus({ lifecycle: "failed", pid: null }),
  });

  assert.equal(result.overallStatus, "attention");
  assert.equal(result.freshness, "stale");
});

test("missing metadata remains unknown while explicit false is unavailable", () => {
  const result = buildAgentCapabilityManifest({
    agent: agent(),
    runtime: undefined,
    runtimeStatus: undefined,
    presenceStatus: undefined,
    observer: { connectionState: "idle", events: [] },
  });

  assert.equal(result.overallStatus, "unknown");
  assert.equal(result.freshness, "unknown");
  assert.equal(
    result.features.find((feature) => feature.id === "image-input")?.state,
    "unknown",
  );
  assert.equal(
    result.features.find((feature) => feature.id === "native-config")?.state,
    "unknown",
  );
  assert.equal(result.commandsState, "unknown");
  assert.equal(result.toolSourcesState, "unknown");
  assert.equal(result.toolsState, "unknown");

  const olderBackendResult = manifest({
    runtime: runtime({
      supportsAcpNativeConfig: null,
      supportsAcpModelSwitching: null,
      mcpHooks: null,
    }),
  });
  assert.equal(
    olderBackendResult.features.find(
      (feature) => feature.id === "native-config",
    )?.state,
    "unknown",
  );
});

test("malformed capability arrays remain unknown and absent tool sources stay absent", () => {
  const malformed = manifest({
    observer: {
      connectionState: "open",
      events: [
        observerEvent(1, "2026-07-26T01:01:00.000Z", "agent_initialized", {
          initializeResult: {
            agentCapabilities: { tools: [{ name: "/private/tool" }] },
          },
        }),
        observerEvent(
          2,
          "2026-07-26T01:02:00.000Z",
          "session_config_captured",
          {
            capabilityManifest: {
              toolSources: [{ name: "/private/source" }],
            },
          },
        ),
        observerEvent(3, "2026-07-26T01:03:00.000Z", "acp_read", {
          params: {
            update: {
              sessionUpdate: "available_commands_update",
              availableCommands: [{ name: "/private/command" }],
            },
          },
        }),
      ],
    },
  });
  assert.equal(malformed.commandsState, "unknown");
  assert.equal(malformed.toolSourcesState, "unknown");
  assert.equal(malformed.toolsState, "unknown");

  const sourceAbsent = manifest({
    observer: {
      connectionState: "open",
      events: [
        observerEvent(1, "2026-07-26T01:01:00.000Z", "agent_initialized", {
          initializeResult: {
            agentCapabilities: { tools: [{ name: "safe_tool" }] },
          },
        }),
      ],
    },
  });
  assert.equal(sourceAbsent.toolsState, "reported");
  assert.equal(sourceAbsent.tools[0]?.source, null);
});

test("newer initialize and command observations replace older reports", () => {
  const newerInitialize = observerEvent(
    9,
    "2026-07-26T02:00:00.000Z",
    "agent_initialized",
    {
      initializeResult: {
        protocolVersion: 3,
        agentInfo: { name: "New runtime", version: "2.0.0" },
        agentCapabilities: {
          promptCapabilities: { image: false, embeddedContext: true },
          tools: [],
        },
      },
    },
  );
  const newerCommands = observerEvent(
    10,
    "2026-07-26T02:01:00.000Z",
    "acp_read",
    {
      params: {
        update: {
          sessionUpdate: "available_commands_update",
          availableCommands: [],
        },
      },
    },
  );
  const result = manifest({
    observer: {
      connectionState: "open",
      events: [
        observerEvent(1, "2026-07-26T01:01:00.000Z", "agent_initialized", {
          initializeResult: {
            protocolVersion: 2,
            agentInfo: { name: "Old runtime", version: "1.0.0" },
            agentCapabilities: { promptCapabilities: { image: true } },
          },
        }),
        observerEvent(2, "2026-07-26T01:02:00.000Z", "acp_read", {
          params: {
            update: {
              sessionUpdate: "available_commands_update",
              availableCommands: [{ name: "old_command" }],
            },
          },
        }),
        newerInitialize,
        newerCommands,
      ],
    },
  });

  assert.equal(result.runtime.label, "New runtime");
  assert.equal(result.protocolVersion, "3");
  assert.equal(
    result.features.find((feature) => feature.id === "image-input")?.state,
    "unavailable",
  );
  assert.equal(
    result.features.find((feature) => feature.id === "embedded-context")?.state,
    "reported",
  );
  assert.deepEqual(result.commands, []);
  assert.equal(result.commandsState, "unavailable");
  assert.equal(result.toolsState, "unavailable");
});

test("a new initialize invalidates session evidence from the previous process", () => {
  const result = manifest({
    observer: {
      connectionState: "open",
      events: [
        observerEvent(
          1,
          "2026-07-26T01:00:00.000Z",
          "session_config_captured",
          {
            models: { currentModelId: "stale-model" },
            capabilityManifest: {
              toolSources: [{ name: "stale-source", kind: "mcp" }],
              permissionMode: {
                requested: "bypassPermissions",
                effective: "perToolAutoDecision",
                source: "buzzHarness",
              },
            },
          },
        ),
        observerEvent(2, "2026-07-26T01:01:00.000Z", "acp_read", {
          params: {
            update: {
              sessionUpdate: "available_commands_update",
              availableCommands: [{ name: "stale-command" }],
            },
          },
        }),
        observerEvent(3, "2026-07-26T02:00:00.000Z", "agent_initialized", {
          initializeResult: {
            protocolVersion: 3,
            agentInfo: { name: "Fresh runtime", version: "2.0.0" },
            agentCapabilities: {},
          },
        }),
      ],
    },
  });

  assert.deepEqual(result.model, {
    value: "configured-model",
    source: "configured",
    requested: "configured-model",
    matchesRequested: true,
  });
  assert.deepEqual(result.toolSources, []);
  assert.equal(result.toolSourcesState, "unknown");
  assert.deepEqual(result.commands, []);
  assert.equal(result.commandsState, "unknown");
  assert.equal(result.permissionMode.effective, null);
});

test("manifest output never falls back to executable paths or raw lifecycle errors", () => {
  const result = buildAgentCapabilityManifest({
    agent: agent({ agentCommand: "/private/bin/secret-runtime" }),
    runtime: undefined,
    runtimeStatus: runtimeStatus({
      lifecycle: "failed",
      error: "/private/path credential-canary",
    }),
    presenceStatus: "offline",
    observer: { connectionState: "error", events: [] },
  });
  const serialized = JSON.stringify(result);

  assert.equal(result.runtime.label, "Unknown runtime");
  assert.equal(serialized.includes("/private/bin/secret-runtime"), false);
  assert.equal(serialized.includes("/private/path"), false);
  assert.equal(serialized.includes("credential-canary"), false);
});

test("runtime matching uses catalog facts without runtime-id render checks", () => {
  const match = findManifestRuntime(
    agent({ agentCommand: "/managed/bin/codex-acp" }),
    [runtime()],
  );
  assert.equal(match?.id, "codex");
});

test("failed model application reports the runtime model and requested mismatch", () => {
  const result = manifest({
    observer: {
      connectionState: "open",
      events: [
        observerEvent(1, "2026-07-26T01:01:00.000Z", "agent_initialized", {
          initializeResult: {
            protocolVersion: 2,
            agentInfo: { name: "Codex ACP", version: "1.2.3" },
            agentCapabilities: {},
          },
        }),
        observerEvent(
          2,
          "2026-07-26T01:02:00.000Z",
          "session_config_captured",
          {
            models: { currentModelId: "runtime-default" },
            capabilityManifest: {
              modelApplication: {
                requested: "configured-model",
                applied: false,
              },
            },
          },
        ),
      ],
    },
  });

  assert.deepEqual(result.model, {
    value: "runtime-default",
    source: "reported",
    requested: "configured-model",
    matchesRequested: false,
  });
});

test("a new session config invalidates commands from the prior session", () => {
  const result = manifest({
    observer: {
      connectionState: "open",
      events: [
        observerEvent(1, "2026-07-26T01:00:00.000Z", "agent_initialized", {
          initializeResult: { agentCapabilities: {} },
        }),
        observerEvent(
          2,
          "2026-07-26T01:01:00.000Z",
          "session_config_captured",
          { capabilityManifest: {} },
          "session-1",
        ),
        observerEvent(
          3,
          "2026-07-26T02:01:00.000Z",
          "acp_read",
          {
            params: {
              update: {
                sessionUpdate: "available_commands_update",
                availableCommands: [{ name: "prior_session_command" }],
              },
            },
          },
          "session-1",
        ),
        observerEvent(
          4,
          "2026-07-26T02:00:00.000Z",
          "session_config_captured",
          { capabilityManifest: {} },
          "session-2",
        ),
      ],
    },
  });

  assert.deepEqual(result.commands, []);
  assert.equal(result.commandsState, "unknown");
});

test("observer reduction retains capability evidence after raw events are trimmed", () => {
  resetAgentObserverStore();
  injectObserverEventsForE2E(AGENT_PUBKEY, [
    observerEvent(1, "2026-07-26T01:01:00.000Z", "agent_initialized", {
      initializeResult: {
        protocolVersion: 2,
        agentInfo: { name: "Codex ACP", version: "1.0.0" },
        agentCapabilities: { promptCapabilities: { image: true } },
      },
    }),
    observerEvent(2, "2026-07-26T01:02:00.000Z", "session_config_captured", {
      capabilityManifest: {
        toolSources: [{ name: "github", kind: "mcp" }],
      },
    }),
    observerEvent(3, "2026-07-26T01:03:00.000Z", "acp_read", {
      params: {
        update: {
          sessionUpdate: "available_commands_update",
          availableCommands: [{ name: "create_plan" }],
        },
      },
    }),
  ]);
  injectObserverEventsForE2E(
    AGENT_PUBKEY,
    Array.from({ length: 3_001 }, (_, index) =>
      observerEvent(
        index + 4,
        new Date(
          Date.parse("2026-07-26T01:04:00.000Z") + index * 1_000,
        ).toISOString(),
        "agent_message_chunk",
        { text: `irrelevant-${index}` },
      ),
    ),
  );

  const snapshot = getAgentObserverSnapshot(AGENT_PUBKEY, true);
  // When events exceed MAX_OBSERVER_EVENTS (3000), the store trims to
  // OBSERVER_EVENTS_LOW_WATER (2700) to amortize eviction across refills.
  assert.equal(snapshot.events.length, 2_700);
  assert.equal(
    snapshot.events.some((event) => event.kind === "agent_initialized"),
    false,
  );
  const result = buildAgentCapabilityManifest({
    agent: agent(),
    runtime: runtime(),
    runtimeStatus: runtimeStatus(),
    presenceStatus: "online",
    observer: snapshot,
  });
  assert.equal(result.runtime.version, "1.0.0");
  assert.deepEqual(result.commands, ["create_plan"]);
  assert.deepEqual(result.toolSources, ["github"]);
});

test("observer reset clears live manifest evidence for a community switch", () => {
  resetAgentObserverStore();
  injectObserverEventsForE2E(AGENT_PUBKEY, [
    observerEvent(1, "2026-07-26T02:00:00.000Z", "agent_initialized", {
      initializeResult: {
        protocolVersion: 2,
        agentInfo: { name: "Codex ACP", version: "1.0.0" },
        agentCapabilities: { promptCapabilities: { image: true } },
      },
    }),
  ]);
  const beforeReset = buildAgentCapabilityManifest({
    agent: agent(),
    runtime: runtime(),
    runtimeStatus: runtimeStatus(),
    presenceStatus: "online",
    observer: getAgentObserverSnapshot(AGENT_PUBKEY, true),
  });
  assert.equal(beforeReset.runtime.version, "1.0.0");

  resetAgentObserverStore();
  const afterReset = buildAgentCapabilityManifest({
    agent: agent(),
    runtime: runtime(),
    runtimeStatus: runtimeStatus(),
    presenceStatus: "online",
    observer: getAgentObserverSnapshot(AGENT_PUBKEY, true),
  });
  assert.equal(afterReset.runtime.version, null);
  assert.equal(afterReset.freshness, "unknown");
  assert.equal(
    afterReset.features.find((feature) => feature.id === "image-input")?.state,
    "unknown",
  );
});

test("credential persistence readiness check reflects keyring_verified as ready", () => {
  const result = manifest({
    agent: agent({ credentialPersistence: "keyring_verified" }),
  });
  const check = result.readiness.find((c) => c.id === "credential_persistence");
  assert.equal(check?.status, "ready");
  assert.equal(check?.detail, "Keyring entry found");
});

test("credential persistence readiness check reflects inline_fallback as ready with detail", () => {
  const result = manifest({
    agent: agent({ credentialPersistence: "inline_fallback" }),
  });
  const check = result.readiness.find((c) => c.id === "credential_persistence");
  assert.equal(check?.status, "ready");
  assert.equal(check?.detail, "Key stored inline (keyring unreachable at last save)");
});

test("credential persistence readiness check reflects missing as attention", () => {
  const result = manifest({
    agent: agent({ credentialPersistence: "missing" }),
  });
  const check = result.readiness.find((c) => c.id === "credential_persistence");
  assert.equal(check?.status, "attention");
  assert.equal(check?.detail, "No key found in keyring or inline storage");
});

test("credential persistence readiness check reflects unavailable as unknown", () => {
  const result = manifest({
    agent: agent({ credentialPersistence: "unavailable" }),
  });
  const check = result.readiness.find((c) => c.id === "credential_persistence");
  assert.equal(check?.status, "unknown");
  assert.equal(check?.detail, "Keyring unavailable — cannot determine persistence");
});

test("credential persistence readiness check defaults to unknown when backend does not report it", () => {
  const result = manifest({
    agent: agent({ credentialPersistence: null }),
  });
  const check = result.readiness.find((c) => c.id === "credential_persistence");
  assert.equal(check?.status, "unknown");
});

test("credential persistence check is positioned after authentication in the readiness array", () => {
  const result = manifest();
  const authIdx = result.readiness.findIndex((c) => c.id === "authentication");
  const credIdx = result.readiness.findIndex((c) => c.id === "credential_persistence");
  assert.equal(credIdx, authIdx + 1);
});

test("lastVerifiedAt reflects observer events only, not catalog or runtime query refreshes", () => {
  const result = manifest({
    catalogObservedAt: "2026-07-26T09:00:00.000Z",
    runtimeObservedAt: "2026-07-26T09:30:00.000Z",
  });
  // Observer events in the default manifest are at 01:01, 01:02, 01:03.
  // Catalog/runtime refreshes are at 09:00 and 09:30 — much later.
  // lastVerifiedAt must be the newest observer event (01:03), not the
  // catalog/runtime refresh, so the card cannot claim "Verified just now"
  // from a query refresh alone.
  assert.equal(result.lastVerifiedAt, "2026-07-26T01:03:00.000Z");
});
