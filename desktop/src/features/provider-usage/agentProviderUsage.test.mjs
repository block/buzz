import assert from "node:assert/strict";
import test from "node:test";

import {
  constrainingProviderWindow,
  providerAllowanceLevel,
  resolveAgentProviderUsage,
} from "./agentProviderUsage.ts";

const runtimes = [
  {
    id: "codex",
    label: "Codex",
    command: "/opt/buzz/codex-acp",
    providerUsageId: "codex",
  },
  {
    id: "custom",
    label: "Custom runtime",
    command: "custom-acp",
    providerUsageId: null,
  },
];

test("resolves provider allowance from runtime catalog metadata", () => {
  assert.deepEqual(
    resolveAgentProviderUsage(
      { runtime: "codex", agentCommand: "old-command" },
      runtimes,
    ),
    { providerUsageId: "codex", runtimeLabel: "Codex" },
  );
});

test("falls back to the effective command for inherited legacy records", () => {
  assert.deepEqual(
    resolveAgentProviderUsage(
      { runtime: null, agentCommand: "/opt/buzz/codex-acp" },
      runtimes,
    ),
    { providerUsageId: "codex", runtimeLabel: "Codex" },
  );
  assert.deepEqual(
    resolveAgentProviderUsage(
      { runtime: null, agentCommand: "unlisted-acp" },
      runtimes,
    ),
    { providerUsageId: null, runtimeLabel: "unlisted-acp" },
  );
});

test("labels 80%, 90%, and exhausted allowance thresholds", () => {
  assert.equal(providerAllowanceLevel(21), "healthy");
  assert.equal(providerAllowanceLevel(20), "low");
  assert.equal(providerAllowanceLevel(10), "critical");
  assert.equal(providerAllowanceLevel(0), "exhausted");
});

test("selects the window with the least remaining allowance", () => {
  const windows = [
    {
      id: "weekly",
      label: "Weekly",
      usedPercent: 40,
      remainingPercent: 60,
      resetsAt: null,
      durationMinutes: null,
    },
    {
      id: "five-hour",
      label: "5 hour",
      usedPercent: 88,
      remainingPercent: 12,
      resetsAt: null,
      durationMinutes: 300,
    },
  ];
  assert.equal(constrainingProviderWindow(windows)?.id, "five-hour");
  assert.equal(constrainingProviderWindow([]), null);
});
