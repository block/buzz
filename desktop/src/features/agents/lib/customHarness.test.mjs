import assert from "node:assert/strict";
import test from "node:test";

import {
  CUSTOM_RUNTIME_ID,
  applyCustomHarnessPreference,
  buildCustomAcpRuntime,
  formatAgentArgsInput,
  isCustomRuntimeId,
  parseAgentArgsInput,
  resolvePreferredCustomRuntime,
} from "./customHarness.ts";

test("buildCustomAcpRuntime requires a non-blank command", () => {
  assert.equal(buildCustomAcpRuntime(""), null);
  assert.equal(buildCustomAcpRuntime("   "), null);
});

test("buildCustomAcpRuntime synthesizes an available ACP runtime", () => {
  const runtime = buildCustomAcpRuntime("agent", ["acp", ""]);
  assert.ok(runtime);
  assert.equal(runtime.id, CUSTOM_RUNTIME_ID);
  assert.equal(runtime.command, "agent");
  assert.deepEqual(runtime.defaultArgs, ["acp"]);
  assert.equal(runtime.availability, "available");
  assert.equal(runtime.authStatus.status, "not_applicable");
});

test("parse/format agent args round-trip comma lists", () => {
  assert.deepEqual(parseAgentArgsInput("acp, --flag"), ["acp", "--flag"]);
  assert.equal(formatAgentArgsInput(["acp", "--flag"]), "acp, --flag");
  assert.equal(formatAgentArgsInput(null), "");
});

test("resolvePreferredCustomRuntime only resolves the custom sentinel", () => {
  assert.equal(
    resolvePreferredCustomRuntime({
      preferred_runtime: "claude",
      preferred_agent_command: "agent",
      preferred_agent_args: ["acp"],
    }),
    null,
  );
  const runtime = resolvePreferredCustomRuntime({
    preferred_runtime: "custom",
    preferred_agent_command: "agent",
    preferred_agent_args: ["acp"],
  });
  assert.ok(runtime);
  assert.equal(runtime.command, "agent");
  assert.equal(isCustomRuntimeId(runtime.id), true);
});

test("applyCustomHarnessPreference clears provider/model", () => {
  const next = applyCustomHarnessPreference(
    {
      env_vars: { KEEP: "1" },
      model: "opus",
      preferred_runtime: "claude",
      preferred_agent_command: null,
      preferred_agent_args: null,
      provider: "anthropic",
    },
    { command: " agent ", args: "acp, --x" },
  );
  assert.equal(next.preferred_runtime, "custom");
  assert.equal(next.preferred_agent_command, "agent");
  assert.deepEqual(next.preferred_agent_args, ["acp", "--x"]);
  assert.equal(next.provider, null);
  assert.equal(next.model, null);
  assert.equal(next.env_vars.KEEP, "1");
});
