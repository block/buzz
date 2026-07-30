import assert from "node:assert/strict";
import test from "node:test";

import { runtimeSetupDetailText } from "./runtimeSetupDetailText.ts";

function entry(overrides = {}) {
  return {
    id: "goose",
    label: "Goose",
    source: "builtin",
    availability: "not_installed",
    avatarUrl: "",
    command: null,
    binaryPath: null,
    cliVersion: null,
    minimumCliVersion: null,
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    installHint: "Buzz uses Goose for this runtime.",
    installInstructionsUrl: "",
    canAutoInstall: true,
    requiresExternalCli: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
    ...overrides,
  };
}

test("runtimeSetupDetailText returns installHint for normal setup states", () => {
  assert.equal(
    runtimeSetupDetailText(entry({ availability: "not_installed" })),
    "Buzz uses Goose for this runtime.",
  );
});

test("runtimeSetupDetailText prefers structured min-version copy", () => {
  assert.equal(
    runtimeSetupDetailText(
      entry({
        availability: "cli_outdated",
        cliVersion: "1.43.9",
        installHint:
          "Detected Goose 1.43.9. Buzz requires Goose 1.44.0 or newer. Update Goose to continue.",
        minimumCliVersion: "1.44.0",
      }),
    ),
    "Goose 1.43.9 detected; requires 1.44.0 or newer.",
  );
});

test("runtimeSetupDetailText handles missing detected version", () => {
  assert.equal(
    runtimeSetupDetailText(
      entry({
        availability: "cli_outdated",
        cliVersion: null,
        installHint:
          "Buzz could not verify the Goose version. Buzz requires Goose 1.44.0 or newer.",
        minimumCliVersion: "1.44.0",
      }),
    ),
    "Goose is outdated; requires 1.44.0 or newer.",
  );
});

test("runtimeSetupDetailText falls back to installHint for cli_outdated without version metadata", () => {
  assert.equal(
    runtimeSetupDetailText(
      entry({
        availability: "cli_outdated",
        cliVersion: null,
        installHint: "Update Goose to continue.",
        minimumCliVersion: null,
      }),
    ),
    "Update Goose to continue.",
  );
});
