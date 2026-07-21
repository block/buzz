/**
 * Contract tests for the canonical agent-config behaviors and disclosure
 * presets (PR 2 flag cleanup).
 *
 * Before PR 2, AgentConfigFields exposed 23 optional props and no test pinned
 * any surface's behavior — a flag could be flipped or reintroduced and the
 * suite would stay green. These tests pin the decided contract:
 *
 *   1. The four canonical behaviors hold (onboarding's values won every call).
 *   2. "full" disclosure shows everything (the evergreen stance).
 *   3. "onboarding-essential" hides escape hatches and power tools, but
 *      NEVER the effort field (onboarding never hid it).
 *
 * If a future change needs a different behavior or preset, it must edit these
 * assertions — which is the conversation the flag cleanup was designed to force.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  CANONICAL_CONFIG_BEHAVIORS,
  resolveDisclosure,
  shouldShowModelStatusMessage,
} from "./AgentConfigFields.tsx";

test("canonical behaviors: onboarding's values are the only behavior", () => {
  assert.deepEqual(CANONICAL_CONFIG_BEHAVIORS, {
    // Changing provider auto-selects a valid model (no dead-end empty model).
    autoSelectModelOnProviderChange: true,
    // Model select stays usable while discovery loads (no flash-disable).
    disableModelSelectDuringDiscovery: false,
    // Switching provider keeps the old provider's typed API key in env_vars.
    preserveCredentialEnvVarsOnProviderChange: true,
    // Model/effort are locked until a provider exists (no saveable invalid state).
    requireProviderForModelAndEffort: true,
  });
});

test("full disclosure shows every field, escape hatch, and description", () => {
  const full = resolveDisclosure("full");
  for (const [key, value] of Object.entries(full)) {
    assert.equal(value, true, `full preset must show ${key}`);
  }
});

test("onboarding-essential hides power tools but never the effort field", () => {
  const essential = resolveDisclosure("onboarding-essential");
  assert.deepEqual(essential, {
    showAdvancedFields: false,
    showCustomModelOption: false,
    showCustomProviderOption: false,
    showDescriptions: false,
    // Effort was never hidden by onboarding; both presets show it.
    showEffortField: true,
    showProviderPlaceholderOption: false,
    showRequiredIndicators: false,
    showUnavailableEffortOptions: false,
  });
});

// ── shouldShowModelStatusMessage ──────────────────────────────────────────────
// The onboarding-essential preset sets showDescriptions=false.  Discovery
// warnings must bypass the preset so first-run failures are never invisible.

test("shouldShowModelStatusMessage_fullDisclosure_nullStatus_showsMessage", () => {
  // Full disclosure always shows the status line regardless of status.
  assert.equal(shouldShowModelStatusMessage(true, null), true);
});

test("shouldShowModelStatusMessage_onboardingPreset_nullStatus_hidesMessage", () => {
  // Happy path: no status → status line hidden in onboarding.
  const { showDescriptions } = resolveDisclosure("onboarding-essential");
  assert.equal(shouldShowModelStatusMessage(showDescriptions, null), false);
});

test("shouldShowModelStatusMessage_onboardingPreset_warningStatus_showsMessage", () => {
  // Discovery failure → status line surfaces even in onboarding-essential.
  const { showDescriptions } = resolveDisclosure("onboarding-essential");
  const warning = {
    message: "Claude Code reported no models.",
    tone: "warning",
  };
  assert.equal(shouldShowModelStatusMessage(showDescriptions, warning), true);
});
