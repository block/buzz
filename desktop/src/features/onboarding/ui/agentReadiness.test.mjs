import assert from "node:assert/strict";
import test from "node:test";

import { resolveAgentReadiness } from "./agentReadiness.ts";

// Minimal stub helpers.
function makeRuntime(overrides = {}) {
  return {
    id: "goose",
    label: "Goose",
    availability: "available",
    authStatus: { status: "logged_in" },
    avatarUrl: "",
    command: "goose",
    binaryPath: "/usr/local/bin/goose",
    defaultArgs: [],
    mcpCommand: null,
    installHint: "",
    installInstructionsUrl: "https://example.com",
    canAutoInstall: false,
    underlyingCliPath: null,
    nodeRequired: false,
    loginHint: null,
    ...overrides,
  };
}

function makeConfig(overrides = {}) {
  return {
    env_vars: {},
    provider: null,
    model: null,
    preferred_runtime: "goose",
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// CLI path
// ---------------------------------------------------------------------------

test("resolveAgentReadiness_cli_returns_ready_when_preferred_cli_runtime_is_logged_in", () => {
  const runtimes = [makeRuntime({ id: "claude", label: "Claude" })];
  const result = resolveAgentReadiness(
    runtimes,
    makeConfig({ preferred_runtime: "claude" }),
  );
  assert.deepEqual(result, {
    ready: true,
    reason: "cli",
    runtimeLabel: "Claude",
  });
});

test("resolveAgentReadiness_uses_only_the_preferred_runtime", () => {
  const runtimes = [
    makeRuntime({ id: "claude", label: "Claude" }),
    makeRuntime({ id: "goose", label: "Goose" }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig(), "preferred");
  assert.equal(result.ready, false);
});

test("resolveAgentReadiness_cli_skips_logged_out_runtimes", () => {
  const runtimes = [
    makeRuntime({
      id: "goose",
      label: "Goose",
      authStatus: { status: "logged_out" },
    }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig(), "preferred");
  assert.equal(result.ready, false);
});

test("resolveAgentReadiness_goose_requires_provider_and_model", () => {
  const runtimes = [
    makeRuntime({
      id: "goose",
      label: "Goose",
      availability: "available",
      authStatus: { status: "not_applicable" },
    }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig(), "preferred");
  assert.equal(result.ready, false);
});

test("resolveAgentReadiness_cli_not_ready_for_unknown_auth_status", () => {
  // unknown means auth state hasn't been determined yet — conservative.
  const runtimes = [
    makeRuntime({
      id: "goose",
      label: "Goose",
      availability: "available",
      authStatus: { status: "unknown" },
    }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig(), "preferred");
  assert.equal(result.ready, false);
});

test("resolveAgentReadiness_cli_not_ready_for_config_invalid_auth_status", () => {
  const runtimes = [
    makeRuntime({
      id: "goose",
      label: "Goose",
      availability: "available",
      authStatus: { status: "config_invalid" },
    }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig(), "preferred");
  assert.equal(result.ready, false);
});

test("resolveAgentReadiness_cli_skips_unavailable_runtimes", () => {
  const runtimes = [
    makeRuntime({
      id: "goose",
      label: "Goose",
      availability: "not_installed",
      authStatus: { status: "logged_in" },
    }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig(), "preferred");
  assert.equal(result.ready, false);
});

test("resolveAgentReadiness_cli_ignores_buzz_agent_runtime", () => {
  // buzz-agent with availability=available and logged_in must NOT trigger the CLI path.
  const runtimes = [
    makeRuntime({
      id: "buzz-agent",
      label: "buzz-agent",
      authStatus: { status: "not_applicable" },
    }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig(), "preferred");
  assert.equal(result.ready, false);
});

// ---------------------------------------------------------------------------
// buzz-agent path
// ---------------------------------------------------------------------------

test("resolveAgentReadiness_buzz_agent_ready_when_provider_model_and_key_set", () => {
  // anthropic requires ANTHROPIC_API_KEY
  const result = resolveAgentReadiness(
    [makeRuntime({ id: "buzz-agent", label: "Buzz Agent" })],
    makeConfig({
      preferred_runtime: "buzz-agent",
      provider: "anthropic",
      model: "claude-3-5-sonnet-latest",
      env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
    }),
  );
  assert.deepEqual(result, { ready: true, reason: "buzz-agent" });
});

test("resolveAgentReadiness_buzz_agent_not_ready_when_missing_required_credential_key", () => {
  const config = makeConfig({
    provider: "anthropic",
    model: "claude-3-5-sonnet-latest",
    env_vars: {},
  });
  const result = resolveAgentReadiness([], config);
  assert.equal(result.ready, false);
});

test("resolveAgentReadiness_buzz_agent_not_ready_when_provider_missing", () => {
  const config = makeConfig({
    provider: null,
    model: "claude-3-5-sonnet-latest",
    env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
  });
  const result = resolveAgentReadiness([], config);
  assert.equal(result.ready, false);
});

test("resolveAgentReadiness_buzz_agent_not_ready_when_model_missing", () => {
  const config = makeConfig({
    provider: "anthropic",
    model: null,
    env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
  });
  const result = resolveAgentReadiness([], config);
  assert.equal(result.ready, false);
});

// ---------------------------------------------------------------------------
// Neither path ready
// ---------------------------------------------------------------------------

test("resolveAgentReadiness_neither_returns_not_ready", () => {
  const result = resolveAgentReadiness([], makeConfig());
  assert.deepEqual(result, { ready: false });
});

test("resolveAgentReadiness_welcome_readiness_uses_ready_cli_without_preference", () => {
  const runtimes = [makeRuntime({ id: "claude", label: "Claude" })];
  const result = resolveAgentReadiness(
    runtimes,
    makeConfig({ preferred_runtime: null }),
  );
  assert.deepEqual(result, {
    ready: true,
    reason: "cli",
    runtimeLabel: "Claude",
  });
});

test("resolveAgentReadiness_legacy_config_without_preference_uses_buzz_agent_fields", () => {
  const runtimes = [makeRuntime({ id: "buzz-agent", label: "Buzz Agent" })];
  const result = resolveAgentReadiness(
    runtimes,
    makeConfig({
      preferred_runtime: null,
      provider: "anthropic",
      model: "claude-3-5-sonnet-latest",
      env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
    }),
  );
  assert.deepEqual(result, { ready: true, reason: "buzz-agent" });
});

test("resolveAgentReadiness_legacy_config_does_not_treat_goose_binary_as_ready", () => {
  const result = resolveAgentReadiness(
    [makeRuntime({ id: "goose", label: "Goose" })],
    makeConfig({ preferred_runtime: null }),
    "preferred",
  );
  assert.deepEqual(result, { ready: false });
});

// ---------------------------------------------------------------------------
// Preferred runtime isolation
// ---------------------------------------------------------------------------

test("resolveAgentReadiness_preferred_goose_does_not_borrow_ready_buzz_agent_config", () => {
  const runtimes = [
    makeRuntime({ id: "goose", label: "Goose" }),
    makeRuntime({ id: "buzz-agent", label: "Buzz Agent" }),
  ];
  const result = resolveAgentReadiness(
    runtimes,
    makeConfig({
      provider: "anthropic",
      model: null,
      env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
    }),
    "preferred",
  );
  assert.equal(result.ready, false);
});
