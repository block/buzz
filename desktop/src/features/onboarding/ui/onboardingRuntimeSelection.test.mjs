import assert from "node:assert/strict";
import test from "node:test";

import {
  getDefaultModelConfigRuntimeId,
  getPreferredRuntimeIdForSelection,
  runtimeCanAdvanceOnboarding,
  runtimeCanBeSelected,
  runtimeSelectionNeedsDefaultModelConfig,
  runtimeSelectionNeedsDefaultsStep,
} from "./onboardingRuntimeSelection.ts";

function runtime(id, availability, status) {
  return { id, availability, authStatus: { status } };
}

test("known onboarding harnesses can be selected regardless of setup state", () => {
  for (const id of ["claude", "codex"]) {
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "logged_in")),
      true,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "not_applicable")),
      true,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "logged_out")),
      true,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "config_invalid")),
      true,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "unknown")),
      true,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "not_installed", "logged_out")),
      true,
    );
  }

  for (const id of ["buzz-agent", "goose"]) {
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "not_applicable")),
      true,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "not_installed", "not_applicable")),
      true,
    );
  }
});

test("unknown runtimes are not onboarding choices", () => {
  assert.equal(
    runtimeCanBeSelected(runtime("custom", "available", "logged_in")),
    false,
  );
});

test("selected runtimes can advance only after setup is complete", () => {
  assert.equal(
    runtimeCanAdvanceOnboarding(runtime("claude", "available", "logged_in")),
    true,
  );
  assert.equal(
    runtimeCanAdvanceOnboarding(
      runtime("buzz-agent", "available", "not_applicable"),
    ),
    true,
  );
  assert.equal(
    runtimeCanAdvanceOnboarding(runtime("claude", "available", "logged_out")),
    false,
  );
  assert.equal(
    runtimeCanAdvanceOnboarding(runtime("codex", "not_installed", "unknown")),
    false,
  );
  assert.equal(
    runtimeCanAdvanceOnboarding(
      runtime("claude", "adapter_missing", "unknown"),
    ),
    false,
  );
});

test("provider-backed selections drive the default model config step", () => {
  assert.equal(
    runtimeSelectionNeedsDefaultModelConfig(["claude", "codex"]),
    false,
  );
  assert.equal(
    runtimeSelectionNeedsDefaultModelConfig(["claude", "goose"]),
    true,
  );
  assert.equal(
    getDefaultModelConfigRuntimeId(["claude", "codex", "goose", "buzz-agent"]),
    "buzz-agent",
  );
  assert.equal(
    getPreferredRuntimeIdForSelection(["claude", "codex", "goose"]),
    "goose",
  );
  assert.equal(
    getPreferredRuntimeIdForSelection(["claude", "codex"]),
    "claude",
  );
});

test("any harness selection drives the defaults step", () => {
  assert.equal(runtimeSelectionNeedsDefaultsStep([]), false);
  assert.equal(runtimeSelectionNeedsDefaultsStep(["claude"]), true);
  assert.equal(runtimeSelectionNeedsDefaultsStep(["codex"]), true);
  assert.equal(runtimeSelectionNeedsDefaultsStep(["claude", "codex"]), true);
  assert.equal(runtimeSelectionNeedsDefaultsStep(["goose"]), true);
});
