import assert from "node:assert/strict";
import test from "node:test";

// ── fromRawAcpRuntimeCatalogEntry: custom row API-boundary (B-2) ─────────────
//
// These tests feed real raw custom catalog rows through fromRawAcpRuntimeCatalogEntry
// and verify the Rust→TypeScript mapping boundary: definition_env (snake_case)
// arrives as definitionEnv (camelCase), source "custom" is preserved, and the
// env round-trips end-to-end so a save-then-edit cycle cannot erase env.

const { fromRawAcpRuntimeCatalogEntry } = await import("./tauri.ts");

test("fromRawAcpRuntimeCatalogEntry maps definition_env to definitionEnv", () => {
  const raw = {
    id: "my-harness",
    label: "My Harness",
    availability: "available",
    command: "my-bin",
    source: "custom",
    definition_env: { ANTHROPIC_API_KEY: "sk-test", MODEL: "claude-3" },
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.deepStrictEqual(entry.definitionEnv, {
    ANTHROPIC_API_KEY: "sk-test",
    MODEL: "claude-3",
  });
  assert.equal(entry.source, "custom");
});

test("fromRawAcpRuntimeCatalogEntry defaults definitionEnv to {} when absent", () => {
  // Rust serialization skips empty BTreeMap, so definition_env will be absent
  // for harnesses with no env defined — the mapper must default to {}.
  const raw = {
    id: "no-env-harness",
    label: "No Env",
    availability: "available",
    command: "no-env-bin",
    source: "custom",
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.deepStrictEqual(
    entry.definitionEnv,
    {},
    "absent definition_env must map to empty object, not undefined",
  );
});

test("fromRawAcpRuntimeCatalogEntry preserves source preset", () => {
  const raw = {
    id: "cursor",
    label: "Cursor",
    availability: "available",
    command: "cursor",
    source: "preset",
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.equal(entry.source, "preset");
  assert.deepStrictEqual(entry.definitionEnv, {});
});

test("fromRawAcpRuntimeCatalogEntry env round-trips through edit payload shape", () => {
  // Simulate the full save → re-open cycle: raw entry comes back from Rust
  // with definition_env populated; the edit form reads entry.definitionEnv.
  // Verify the env values are identical before and after the mapper.
  const envValues = { OPENAI_API_KEY: "sk-live-abc", REGION: "us-east-1" };
  const raw = {
    id: "openai-harness",
    label: "OpenAI",
    availability: "not_installed",
    command: "openai-agent",
    source: "custom",
    definition_env: envValues,
    default_args: ["--acp"],
    can_auto_install: false,
    requires_external_cli: true,
    install_hint: "Install the OpenAI CLI",
    install_instructions_url: "https://platform.openai.com/docs",
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  // The edit form reads entry.definitionEnv; it must equal the original env.
  assert.deepStrictEqual(
    entry.definitionEnv,
    envValues,
    "env must round-trip: edit form must see the same values that Rust serialized",
  );
});

// ── max_parallelism → maxParallelism mapping ──────────────────────────────────

test("fromRawAcpRuntimeCatalogEntry maps max_parallelism to maxParallelism when present", () => {
  const raw = {
    id: "openclaw",
    label: "OpenClaw",
    availability: "not_installed",
    command: null,
    source: "preset",
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
    max_parallelism: 5,
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.equal(
    entry.maxParallelism,
    5,
    "max_parallelism: 5 must map to maxParallelism: 5",
  );
});

test("fromRawAcpRuntimeCatalogEntry omits maxParallelism when max_parallelism is absent", () => {
  const raw = {
    id: "goose",
    label: "Goose",
    availability: "available",
    command: "goose",
    source: "builtin",
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
    // No max_parallelism field — uncapped harness.
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.equal(
    entry.maxParallelism,
    undefined,
    "uncapped harness must have maxParallelism: undefined",
  );
});
