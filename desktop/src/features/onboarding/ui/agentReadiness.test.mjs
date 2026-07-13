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
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// CLI path
// ---------------------------------------------------------------------------

test("resolveAgentReadiness_cli_returns_ready_when_cli_runtime_available_and_logged_in", () => {
  const runtimes = [makeRuntime({ id: "goose", label: "Goose" })];
  const result = resolveAgentReadiness(runtimes, makeConfig());
  assert.deepEqual(result, {
    ready: true,
    reason: "cli",
    runtimeLabel: "Goose",
  });
});

test("resolveAgentReadiness_cli_uses_first_matching_runtime", () => {
  const runtimes = [
    makeRuntime({ id: "claude", label: "Claude" }),
    makeRuntime({ id: "goose", label: "Goose" }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig());
  assert.equal(result.ready, true);
  if (result.ready) {
    assert.equal(result.reason, "cli");
    assert.equal(result.runtimeLabel, "Claude");
  }
});

test("resolveAgentReadiness_cli_skips_logged_out_runtimes", () => {
  const runtimes = [
    makeRuntime({
      id: "goose",
      label: "Goose",
      authStatus: { status: "logged_out" },
    }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig());
  assert.equal(result.ready, false);
});

test("resolveAgentReadiness_cli_ready_for_login_free_harness_with_not_applicable_auth", () => {
  // Goose uses not_applicable because it has no login concept; it should
  // still show green if the runtime is available.
  const runtimes = [
    makeRuntime({
      id: "goose",
      label: "Goose",
      availability: "available",
      authStatus: { status: "not_applicable" },
    }),
  ];
  const result = resolveAgentReadiness(runtimes, makeConfig());
  assert.deepEqual(result, {
    ready: true,
    reason: "cli",
    runtimeLabel: "Goose",
  });
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
  const result = resolveAgentReadiness(runtimes, makeConfig());
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
  const result = resolveAgentReadiness(runtimes, makeConfig());
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
  const result = resolveAgentReadiness(runtimes, makeConfig());
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
  const result = resolveAgentReadiness(runtimes, makeConfig());
  assert.equal(result.ready, false);
});

// ---------------------------------------------------------------------------
// buzz-agent path
// ---------------------------------------------------------------------------

test("resolveAgentReadiness_buzz_agent_ready_when_provider_model_and_key_set", () => {
  // anthropic requires ANTHROPIC_API_KEY
  const config = makeConfig({
    provider: "anthropic",
    model: "claude-3-5-sonnet-latest",
    env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
  });
  const result = resolveAgentReadiness([], config);
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

// ---------------------------------------------------------------------------
// CLI path takes priority over buzz-agent path
// ---------------------------------------------------------------------------

test("resolveAgentReadiness_cli_wins_over_buzz_agent_when_both_ready", () => {
  const runtimes = [makeRuntime({ id: "goose", label: "Goose" })];
  const config = makeConfig({
    provider: "anthropic",
    model: "claude-3-5-sonnet-latest",
    env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
  });
  const result = resolveAgentReadiness(runtimes, config);
  assert.equal(result.ready, true);
  if (result.ready) {
    assert.equal(result.reason, "cli");
  }
});
