import assert from "node:assert/strict";
import test from "node:test";

import {
  availableRuntimesForStart,
  buildInstanceInputForDefinition,
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

test("mapping carries the runtime and definition fields", async () => {
  const input = await buildInstanceInputForDefinition(persona(), gooseRuntime);
  assert.equal(input.name, "Test Agent");
  assert.equal(input.acpCommand, "buzz-acp");
  assert.equal(input.agentCommand, "goose-cmd");
  assert.deepEqual(input.agentArgs, ["--acp"]);
  assert.equal(input.mcpCommand, "goose-mcp");
  assert.equal(input.personaId, "p-1");
  assert.equal(input.systemPrompt, "prompt");
  assert.equal(input.model, undefined);
  assert.equal(input.spawnAfterCreate, true);
  assert.equal(input.startOnAppLaunch, true);
  assert.deepEqual(input.backend, { type: "local" });
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
