import assert from "node:assert/strict";
import test from "node:test";

import { runtimeAvailabilityWarning } from "./runtimeAvailabilityWarning.ts";

function entry(overrides) {
  return {
    id: "amp",
    label: "Amp",
    avatarUrl: "",
    availability: "not_installed",
    command: null,
    binaryPath: null,
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    installHint: "Install the amp-acp npm adapter: npm install -g amp-acp.",
    installInstructionsUrl: "https://example.com",
    canAutoInstall: false,
    requiresExternalCli: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: "not_applicable",
    loginHint: null,
    source: "preset",
    definitionEnv: {},
    ...overrides,
  };
}

test("available runtimes produce no warning", () => {
  assert.equal(
    runtimeAvailabilityWarning(entry({ availability: "available" })),
    null,
  );
});

test("not-installed warning includes the install hint", () => {
  // Tyler's Amp hand-test: the hint was carried by the entry but the old
  // render gate (requiresExternalCli, hardcoded false for presets) hid it.
  const warning = runtimeAvailabilityWarning(entry({}));
  assert.equal(
    warning,
    "Amp is not installed. Install the amp-acp npm adapter: npm install -g amp-acp.",
  );
});

test("empty install hint leaves no trailing space", () => {
  // Custom harnesses default installHint to "" — an unguarded template
  // would render "Amp is not installed. ".
  const warning = runtimeAvailabilityWarning(entry({ installHint: "" }));
  assert.equal(warning, "Amp is not installed.");
});

test("adapter-missing warning names the adapter and includes the hint", () => {
  const warning = runtimeAvailabilityWarning(
    entry({ availability: "adapter_missing" }),
  );
  assert.match(
    warning ?? "",
    /CLI is installed but the ACP adapter is missing/,
  );
  assert.match(warning ?? "", /npm install -g amp-acp/);
});

test("cli-missing external-CLI warning keeps the hint", () => {
  const warning = runtimeAvailabilityWarning(
    entry({
      availability: "cli_missing",
      requiresExternalCli: true,
      installHint: "Install Codex from example.com.",
      label: "Codex",
    }),
  );
  assert.equal(
    warning,
    "Codex CLI is missing. Install Codex from example.com.",
  );
});

test("adapter-outdated warning stays hint-free reinstall copy", () => {
  const warning = runtimeAvailabilityWarning(
    entry({ availability: "adapter_outdated" }),
  );
  assert.equal(warning, "Amp ACP adapter is outdated — reinstall to continue.");
});
