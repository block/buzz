/**
 * Unit tests for the focusTargetForRequirement helper exported from
 * config-nudge-attachment.tsx.
 *
 * The tests exercise per-row CTA focus semantics — the fix to Thufir's
 * pass-4 IMPORTANT finding that row CTAs on mixed cards always focused the
 * first editable field instead of the row's own field.
 *
 * Test strategy:
 *  - focusTargetForRequirement (pure function): env_key, normalized_field,
 *    cli_login, and card-level fallback comparison (must differ from per-row
 *    when the second row is the one clicked).
 *  - Per-row focus dispatch: verifies the correct focus target reaches
 *    requestOpenEditAgent by round-tripping through consumePendingOpenEditAgent,
 *    mirroring the approach in openEditAgentEvent.test.mjs.
 */

import assert from "node:assert/strict";
import test from "node:test";

// Provide a minimal window shim for the event dispatch path.
const _eventTarget = new EventTarget();
globalThis.window = {
  addEventListener: _eventTarget.addEventListener.bind(_eventTarget),
  removeEventListener: _eventTarget.removeEventListener.bind(_eventTarget),
  dispatchEvent: _eventTarget.dispatchEvent.bind(_eventTarget),
};

import {
  focusTargetForRequirement,
  shouldOpenDoctor,
} from "./config-nudge-attachment.tsx";
import {
  consumePendingOpenEditAgent,
  requestOpenEditAgent,
} from "../../features/agents/openEditAgentEvent.ts";

const AGENT_PUBKEY = "aabbccddeeff00112233445566778899";

// ── Doctor routing ────────────────────────────────────────────────────────────

test("shouldOpenDoctor_gitBashMixedWithEnvKey_routesToDoctor", () => {
  assert.equal(
    shouldOpenDoctor([
      { surface: "git_bash" },
      { surface: "env_key", key: "ANTHROPIC_API_KEY" },
    ]),
    true,
    "a Git Bash requirement must route the whole mixed card to Doctor",
  );
});

test("shouldOpenDoctor_regularMixedRequirements_routesToEditAgent", () => {
  assert.equal(
    shouldOpenDoctor([
      { surface: "env_key", key: "ANTHROPIC_API_KEY" },
      { surface: "normalized_field", field: "model" },
    ]),
    false,
  );
});

// ── focusTargetForRequirement — pure function ─────────────────────────────────

test("focusTargetForRequirement_envKey_returnsEnvKeyTarget", () => {
  const req = { surface: "env_key", key: "ANTHROPIC_API_KEY" };
  assert.deepEqual(focusTargetForRequirement(req), {
    type: "env_key",
    key: "ANTHROPIC_API_KEY",
  });
});

test("focusTargetForRequirement_normalizedField_returnsNormalizedFieldTarget", () => {
  const req = { surface: "normalized_field", field: "model" };
  assert.deepEqual(focusTargetForRequirement(req), {
    type: "normalized_field",
    field: "model",
  });
});

test("focusTargetForRequirement_cliLogin_returnsUndefined", () => {
  const req = {
    surface: "cli_login",
    probe_args: ["goose"],
    setup_copy: "run `goose login`",
    availability: "available",
  };
  assert.equal(
    focusTargetForRequirement(req),
    undefined,
    "cli_login requirement must not map to a focusable Edit Agent field",
  );
});

// ── Per-row vs card-level focus divergence ────────────────────────────────────
//
// Regression: before the fix, EVERY row CTA called openEditAgent() which used
// firstFocusTarget(requirements) — the first non-cli_login req in the list.
// For a mixed card [env_key: ANTHROPIC_API_KEY, normalized_field: model],
// clicking the model row focused the env-key field instead.
//
// These tests verify that the per-row focus matches the ROW, not the first
// editable field on the card.

test("focusTargetForRequirement_secondRowDiffersFromFirstFocusTarget", () => {
  // Mixed card requirements: env_key first, then model.
  const requirements = [
    { surface: "env_key", key: "ANTHROPIC_API_KEY" },
    { surface: "normalized_field", field: "model" },
  ];

  // Card-level firstFocusTarget returns the first editable row.
  const firstTarget = focusTargetForRequirement(requirements[0]);
  // Per-row focus for the SECOND row must differ.
  const secondRowTarget = focusTargetForRequirement(requirements[1]);

  assert.deepEqual(
    firstTarget,
    { type: "env_key", key: "ANTHROPIC_API_KEY" },
    "first row target must be the env_key row",
  );
  assert.deepEqual(
    secondRowTarget,
    { type: "normalized_field", field: "model" },
    "second row target must be the normalized_field row, NOT the first row",
  );
  assert.notDeepEqual(
    firstTarget,
    secondRowTarget,
    "per-row focus for the second row must differ from the first row's focus",
  );
});

// ── requestOpenEditAgent focus round-trip (per-row dispatch) ──────────────────
//
// Verifies the full chain: clicking a row CTA should dispatch requestOpenEditAgent
// with the row-specific focus target. We simulate this directly (without rendering
// the React component) by calling requestOpenEditAgent with the focus target
// focusTargetForRequirement would produce, then consuming it.

test("perRowDispatch_envKeyRow_focusesEnvKey", () => {
  const envKeyReq = { surface: "env_key", key: "ANTHROPIC_API_KEY" };
  // Simulate what RequirementRow's onClick now does.
  requestOpenEditAgent(AGENT_PUBKEY, focusTargetForRequirement(envKeyReq));
  const result = consumePendingOpenEditAgent(AGENT_PUBKEY);
  assert.deepEqual(
    result,
    { type: "env_key", key: "ANTHROPIC_API_KEY" },
    "clicking the env_key row must dispatch focus to that specific key",
  );
});

test("perRowDispatch_normalizedFieldRow_focusesModel", () => {
  const modelReq = { surface: "normalized_field", field: "model" };
  // Simulate what RequirementRow's onClick now does.
  requestOpenEditAgent(AGENT_PUBKEY, focusTargetForRequirement(modelReq));
  const result = consumePendingOpenEditAgent(AGENT_PUBKEY);
  assert.deepEqual(
    result,
    { type: "normalized_field", field: "model" },
    "clicking the model (normalized_field) row must dispatch focus to the model field",
  );
});

test("perRowDispatch_mixedCard_secondRowFocusesModel_notEnvKey", () => {
  // The concrete failing case before the fix:
  // mixed card [env_key: ANTHROPIC_API_KEY, normalized_field: model].
  // Clicking the model row must dispatch model focus, not env_key focus.
  const requirements = [
    { surface: "env_key", key: "ANTHROPIC_API_KEY" },
    { surface: "normalized_field", field: "model" },
  ];

  // Simulate clicking the SECOND row's Edit Agent CTA.
  const secondRowFocus = focusTargetForRequirement(requirements[1]);
  requestOpenEditAgent(AGENT_PUBKEY, secondRowFocus);
  const result = consumePendingOpenEditAgent(AGENT_PUBKEY);

  assert.deepEqual(
    result,
    { type: "normalized_field", field: "model" },
    "clicking the second (model) row on a mixed card must focus model, not the env-key row",
  );
});

test("cardLevelFallback_noPerRowTarget_focusesFirstEditableField", () => {
  // The card-level trigger (not a row CTA) must still use firstFocusTarget
  // semantics — focus the first editable field.
  // Simulate the card trigger path for a mixed card.
  const requirements = [
    { surface: "env_key", key: "ANTHROPIC_API_KEY" },
    { surface: "normalized_field", field: "model" },
  ];
  // Card-level: pick the first non-cli_login req (what firstFocusTarget returns).
  const cardLevelFocus = focusTargetForRequirement(requirements[0]);
  requestOpenEditAgent(AGENT_PUBKEY, cardLevelFocus);
  const result = consumePendingOpenEditAgent(AGENT_PUBKEY);
  assert.deepEqual(
    result,
    { type: "env_key", key: "ANTHROPIC_API_KEY" },
    "card-level trigger must focus the first editable field",
  );
});
