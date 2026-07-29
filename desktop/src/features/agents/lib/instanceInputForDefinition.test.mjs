import assert from "node:assert/strict";
import test from "node:test";

import {
  availableRuntimesForStart,
  buildInstanceInputForDefinition,
  resolveCreateRuntimeForDefinition,
  resolveStartRuntimeForDefinition,
} from "./instanceInputForDefinition.ts";

// ── Phase 1B.3.5: the single definition→instance mapping ────────────────────
//
// Every surface that starts an agent from a definition maps through
// buildInstanceInputForDefinition + resolveStartRuntimeForDefinition +
// availableRuntimesForStart. These tests pin the decided rows:
//   row 1: refuse (actionable error) when the configured runtime is missing
//   row 2: harnessOverride = !persona.runtime || persona.runtime === runtime.id
//   row 3: avatar through resolveManagedAgentAvatarUrl (injectable upload)
//   row 4: create input NEVER contains definition env vars
//   row 6: runtime list acquisition is refetch-aware

const gooseRuntime = {
  id: "goose",
  label: "Goose",
  avatarUrl: "https://runtime/goose.png",
  availability: "available",
  command: "goose-cmd",
  binaryPath: "/bin/goose",
  defaultArgs: ["--acp"],
  mcpCommand: "goose-mcp",
  installHint: "",
  installInstructionsUrl: "",
  canAutoInstall: false,
  underlyingCliPath: null,
};

const claudeRuntime = {
  ...gooseRuntime,
  id: "claude",
  label: "Claude",
  command: "claude-cmd",
  mcpCommand: null,
};

const buzzAgentRuntime = {
  ...gooseRuntime,
  id: "buzz-agent",
  label: "Buzz Agent",
  command: "buzz-agent-cmd",
  mcpCommand: null,
};

function persona(overrides = {}) {
  return {
    id: "p-1",
    displayName: "Test Agent",
    systemPrompt: "prompt",
    model: null,
    runtime: "goose",
    avatarUrl: "https://example.com/a.png",
    envVars: { ANTHROPIC_API_KEY: "persona-secret" },
    isBuiltIn: false,
    ...overrides,
  };
}

test("row 4: create input never contains definition env vars", async () => {
  const input = await buildInstanceInputForDefinition(persona(), gooseRuntime);
  assert.equal(
    "envVars" in input,
    false,
    "definition env must never be seeded into the create input — " +
      "record.env_vars is overrides-only and spawn merges the live definition env",
  );
});

test("row 2: harnessOverride follows the backend-aligned formula", async () => {
  const match = await buildInstanceInputForDefinition(
    persona({ runtime: "goose" }),
    gooseRuntime,
  );
  assert.equal(match.harnessOverride, true, "picked == configured → true");

  const noPreference = await buildInstanceInputForDefinition(
    persona({ runtime: undefined }),
    gooseRuntime,
  );
  assert.equal(noPreference.harnessOverride, true, "no preference → true");

  const differs = await buildInstanceInputForDefinition(
    persona({ runtime: "claude" }),
    gooseRuntime,
  );
  assert.equal(
    differs.harnessOverride,
    false,
    "picked != configured → false (definition stays authoritative)",
  );
});

test("row 3: plain avatar URLs pass through; base64 data URIs upload via the injectable", async () => {
  const plain = await buildInstanceInputForDefinition(persona(), gooseRuntime);
  assert.equal(plain.avatarUrl, "https://example.com/a.png");

  const uploads = [];
  const uploaded = await buildInstanceInputForDefinition(
    persona({ avatarUrl: "data:image/png;base64,aGk=" }),
    gooseRuntime,
    async (bytes) => {
      uploads.push(bytes);
      return {
        url: "https://cdn/blob.png",
        sha256: "x",
        size: 2,
        type: "image/png",
        uploaded: 0,
      };
    },
  );
  assert.equal(uploaded.avatarUrl, "https://cdn/blob.png");
  assert.equal(uploads.length, 1, "upload must go through the injected fn");
});

test("row 3: failed persona avatar upload never substitutes the runtime avatar", async () => {
  const input = await buildInstanceInputForDefinition(
    persona({
      id: "builtin:fizz",
      displayName: "Fizz",
      avatarUrl: "data:image/png;base64,aGk=",
    }),
    claudeRuntime,
    async () => {
      throw new Error("upload failed");
    },
  );

  assert.equal(input.avatarUrl, undefined);
  assert.notEqual(input.avatarUrl, claudeRuntime.avatarUrl);
});

test("mapping carries the runtime and definition fields", async () => {
  const input = await buildInstanceInputForDefinition(persona(), gooseRuntime);
  assert.equal(input.name, "Test Agent");
  assert.equal(input.acpCommand, "buzz-acp");
  assert.equal(input.agentCommand, "goose-cmd");
  // B-5: agentArgs is intentionally empty at create time — spawn reads args
  // live from the definition on every start so definition edits take effect
  // without recreating the agent. Seeding from runtime.defaultArgs here would
  // freeze args at create-time and silently ignore later definition edits.
  assert.deepEqual(input.agentArgs, []);
  assert.equal(input.mcpCommand, "goose-mcp");
  assert.equal(input.personaId, "p-1");
  assert.equal(input.systemPrompt, "prompt");
  assert.equal(input.model, undefined);
  assert.equal(input.provider, undefined);
  assert.equal(input.spawnAfterCreate, true);
  assert.equal(input.startOnAppLaunch, true);
  assert.deepEqual(input.backend, { type: "local" });
});

test("no backend intent is byte-identical to the pre-intent mapping", async () => {
  // The 3 pre-B5 call sites (useManagedAgentActions, usePersonaActions,
  // UserProfilePanel) pass no intent; their output must not move.
  // B-5: agentArgs is [] — args are NOT seeded from the definition at create
  // time. Spawn reads live args from the definition on every start.
  const input = await buildInstanceInputForDefinition(persona(), gooseRuntime);
  assert.deepEqual(input, {
    name: "Test Agent",
    personaId: "p-1",
    systemPrompt: "prompt",
    avatarUrl: "https://example.com/a.png",
    acpCommand: "buzz-acp",
    agentCommand: "goose-cmd",
    agentArgs: [],
    mcpCommand: "goose-mcp",
    harnessOverride: true,
    model: undefined,
    provider: undefined,
    spawnAfterCreate: true,
    startOnAppLaunch: true,
    backend: { type: "local" },
  });
});

test("Buzz shared compute definition carries native provider and auto model", async () => {
  const input = await buildInstanceInputForDefinition(
    persona({
      runtime: "buzz-agent",
      provider: "relay-mesh",
      model: "auto",
    }),
    { ...gooseRuntime, id: "buzz-agent", command: "buzz-agent" },
  );
  assert.equal(input.agentCommand, "buzz-agent");
  assert.equal(input.provider, "relay-mesh");
  assert.equal(input.model, "auto");
  assert.equal(input.spawnAfterCreate, true);
  assert.equal(input.startOnAppLaunch, true);
});

const remoteHarness = {
  id: "goose",
  command: "/opt/host/bin/goose",
  args: ["acp"],
  env: { GOOSE_MODE: "auto" },
};

function providerIntent(overrides = {}) {
  return {
    type: "provider",
    id: "blox",
    config: { region: "us" },
    harness: remoteHarness,
    ...overrides,
  };
}

test("provider intent forces startOnAppLaunch off and spawns no local ACP", async () => {
  const input = await buildInstanceInputForDefinition(
    persona(),
    gooseRuntime,
    undefined,
    providerIntent(),
  );
  assert.deepEqual(input.backend, {
    type: "provider",
    id: "blox",
    config: { region: "us" },
  });
  assert.equal(input.startOnAppLaunch, false, "remote agents never auto-start");
  assert.equal(input.spawnAfterCreate, true);
  // Provider agents spawn no local ACP or MCP sidecar.
  for (const key of ["acpCommand", "mcpCommand", "relayMesh"]) {
    assert.equal(key in input, false, `provider intent must omit ${key}`);
  }
  assert.equal(input.personaId, "p-1", "definition link is kept");
  assert.equal(input.systemPrompt, "prompt");
});

// ── Correction C1: the remote harness pin ───────────────────────────────────
//
// The create-time agentCommand is the ONLY channel by which the harness choice
// reaches the host: `create_time_agent_command_override` drops a divergent
// command when harnessOverride is false, `effective_agent_command` then
// resolves against the LOCAL registry and falls through to
// `default_agent_command()`, and the deploy ships that. So a provider create
// without a true override + remote command silently provisions "buzz-agent".

test("C1: provider intent pins the REMOTE harness command, not the local runtime", async () => {
  const input = await buildInstanceInputForDefinition(
    persona(),
    gooseRuntime,
    undefined,
    providerIntent(),
  );
  assert.equal(input.agentCommand, "/opt/host/bin/goose");
  assert.notEqual(
    input.agentCommand,
    gooseRuntime.command,
    "the local runtime's command describes the wrong machine",
  );
  assert.equal(
    input.harnessOverride,
    true,
    "without the override flag the backend discards the pin and falls back to buzz-agent",
  );
});

test("C1: remote args and env are pinned (nothing re-resolves them)", async () => {
  const input = await buildInstanceInputForDefinition(
    persona(),
    null,
    undefined,
    providerIntent(),
  );
  assert.deepEqual(input.agentArgs, ["acp"]);
  assert.deepEqual(input.envVars, { GOOSE_MODE: "auto" });
});

test("C1: a provider create with no remote harness is refused", async () => {
  await assert.rejects(
    () =>
      buildInstanceInputForDefinition(
        persona(),
        gooseRuntime,
        undefined,
        providerIntent({ harness: undefined }),
      ),
    /remote host/i,
    "must refuse rather than silently deploy the locally-resolved default",
  );
});

test("C1: a blank remote harness command is refused", async () => {
  await assert.rejects(
    () =>
      buildInstanceInputForDefinition(persona(), gooseRuntime, undefined, {
        ...providerIntent(),
        harness: { ...remoteHarness, command: "   " },
      }),
    /remote host/i,
  );
});

test("provider create needs no locally-installed runtime", () => {
  const { runtime } = resolveCreateRuntimeForDefinition([], "goose", true);
  assert.equal(
    runtime,
    null,
    "the harness lives on the host; the local catalog is a different machine",
  );
});

test("local create still refuses an unavailable runtime", () => {
  assert.throws(
    () => resolveCreateRuntimeForDefinition([], "goose", false),
    /Choose an available runtime/,
  );
});

test("create runtime resolves locally when it happens to exist", () => {
  const { runtime } = resolveCreateRuntimeForDefinition(
    [gooseRuntime, claudeRuntime],
    "claude",
    true,
  );
  assert.equal(runtime?.id, "claude", "used for the avatar fallback");
});

test("a local create with no runtime is refused at the mapping too", async () => {
  await assert.rejects(
    () => buildInstanceInputForDefinition(persona(), null),
    /Choose an available runtime/,
  );
});

test("row 1: refuses when the configured runtime is not available", () => {
  assert.throws(
    () =>
      resolveStartRuntimeForDefinition(persona({ runtime: "missing" }), [
        gooseRuntime,
        claudeRuntime,
      ]),
    /not available|No available runtime/i,
    "configured-but-missing runtime must refuse, never silently fall back",
  );
});

test("row 1: resolves the configured runtime when available", () => {
  const { runtime, warnings } = resolveStartRuntimeForDefinition(
    persona({ runtime: "claude" }),
    [gooseRuntime, claudeRuntime],
  );
  assert.equal(runtime.id, "claude");
  assert.deepEqual(warnings, []);
});

test("row 1: no preference resolves the default with no warnings", () => {
  const { runtime, warnings } = resolveStartRuntimeForDefinition(
    persona({ runtime: undefined }),
    [gooseRuntime, claudeRuntime],
  );
  assert.equal(runtime.id, "goose");
  assert.deepEqual(warnings, []);
});

test("row 1: refuses when no runtimes exist at all", () => {
  assert.throws(
    () => resolveStartRuntimeForDefinition(persona({ runtime: undefined }), []),
    /No available runtime/,
  );
});

test("row 6: fetched query uses cached data without refetching", async () => {
  let refetched = false;
  const runtimes = await availableRuntimesForStart({
    isFetched: true,
    data: [gooseRuntime, { ...claudeRuntime, availability: "missing" }],
    refetch: async () => {
      refetched = true;
      return { data: [] };
    },
  });
  assert.equal(refetched, false);
  assert.deepEqual(
    runtimes.map((r) => r.id),
    ["goose"],
    "unavailable runtimes are filtered out",
  );
});

test("row 6: unfetched query refetches instead of resolving empty", async () => {
  const runtimes = await availableRuntimesForStart({
    isFetched: false,
    data: undefined,
    refetch: async () => ({ data: [claudeRuntime] }),
  });
  assert.deepEqual(
    runtimes.map((r) => r.id),
    ["claude"],
    "an unfetched query must fetch, not spuriously report no runtimes",
  );
});

// ── item-13 regression: buzz-agent-first default runtime ─────────────────────
//
// Before this fix, resolveStartRuntimeForDefinition used runtimes[0] (catalog
// order: goose, claude, codex, buzz-agent), so an installed goose would beat
// the bundled buzz-agent sidecar as the default for runtime-less personas.
// The fix applies the preference order: buzz-agent → goose → first available.

test("item-13: goose+buzz-agent both available — persona with no runtime resolves buzz-agent", () => {
  const { runtime, warnings } = resolveStartRuntimeForDefinition(
    persona({ runtime: undefined }),
    [gooseRuntime, claudeRuntime, buzzAgentRuntime],
  );
  assert.equal(
    runtime.id,
    "buzz-agent",
    "buzz-agent must win over catalog-first goose for runtime-less personas",
  );
  assert.deepEqual(warnings, []);
});

test("item-13: goose-only available — persona with no runtime resolves goose", () => {
  const { runtime, warnings } = resolveStartRuntimeForDefinition(
    persona({ runtime: undefined }),
    [gooseRuntime, claudeRuntime],
  );
  assert.equal(runtime.id, "goose");
  assert.deepEqual(warnings, []);
});

test("item-13: no runtimes available — refuses with actionable error", () => {
  assert.throws(
    () => resolveStartRuntimeForDefinition(persona({ runtime: undefined }), []),
    /No available runtime/,
    "empty runtime list must throw, not silently return null",
  );
});
