import assert from "node:assert/strict";
import test from "node:test";

import {
  getReadyOnboardingRuntimes,
  getVisibleOnboardingRuntimes,
  resolveDefaultOnboardingRuntime,
  runtimeIsReadyForOnboarding,
  runtimeIsVisibleInOnboarding,
} from "./onboardingRuntimeSelection.ts";

function runtime(id, availability, status) {
  return { id, availability, authStatus: { status } };
}

test("all bundled harnesses are visible in onboarding", () => {
  assert.equal(runtimeIsVisibleInOnboarding("opencode"), true);
  assert.equal(runtimeIsVisibleInOnboarding("claude"), true);
  assert.equal(runtimeIsVisibleInOnboarding("codex"), true);
  assert.equal(runtimeIsVisibleInOnboarding("goose"), true);
  assert.equal(runtimeIsVisibleInOnboarding("buzz-agent"), true);
  assert.equal(runtimeIsVisibleInOnboarding("custom"), false);
});

test("visible onboarding runtimes use the product order", () => {
  const runtimes = [
    runtime("buzz-agent", "available", "not_applicable"),
    runtime("codex", "available", "logged_in"),
    runtime("opencode", "available", "not_applicable"),
    runtime("goose", "available", "not_applicable"),
    runtime("claude", "available", "logged_in"),
  ];

  assert.deepEqual(
    getVisibleOnboardingRuntimes(runtimes).map(({ id }) => id),
    ["opencode", "claude", "codex", "goose", "buzz-agent"],
  );
});

test("OpenCode is the recommended onboarding default with Buzz Agent fallback", () => {
  assert.equal(
    resolveDefaultOnboardingRuntime([
      runtime("goose", "available", "not_applicable"),
      runtime("buzz-agent", "available", "not_applicable"),
      runtime("opencode", "available", "not_applicable"),
    ])?.id,
    "opencode",
  );
  assert.equal(
    resolveDefaultOnboardingRuntime([
      runtime("goose", "available", "not_applicable"),
      runtime("buzz-agent", "available", "not_applicable"),
    ])?.id,
    "buzz-agent",
  );
});

test("readiness requires an available and authenticated runtime", () => {
  assert.equal(
    runtimeIsReadyForOnboarding(runtime("claude", "available", "logged_in")),
    true,
  );
  assert.equal(
    runtimeIsReadyForOnboarding(
      runtime("codex", "available", "not_applicable"),
    ),
    true,
  );
  assert.equal(
    runtimeIsReadyForOnboarding(runtime("claude", "available", "logged_out")),
    false,
  );
  assert.equal(
    runtimeIsReadyForOnboarding(runtime("codex", "not_installed", "logged_in")),
    false,
  );
});

test("ready onboarding runtimes exclude unknown and non-ready harnesses", () => {
  const runtimes = [
    runtime("goose", "available", "not_applicable"),
    runtime("opencode", "available", "not_applicable"),
    runtime("codex", "available", "logged_out"),
    runtime("buzz-agent", "available", "not_applicable"),
    runtime("claude", "available", "logged_in"),
    runtime("custom", "available", "not_applicable"),
  ];

  assert.deepEqual(
    getReadyOnboardingRuntimes(runtimes).map(({ id }) => id),
    ["opencode", "claude", "goose", "buzz-agent"],
  );
});
