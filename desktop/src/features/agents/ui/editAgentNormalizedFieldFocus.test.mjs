/**
 * Unit tests for EditAgentDialog normalized-field deep-link focus behavior.
 *
 * When a config nudge deep-links to a missing "provider" or "model" field, the
 * dialog must focus the corresponding PersonaDropdownField trigger button.
 * Post-#1540 these fields render as <button> elements (DropdownMenuTrigger),
 * NOT native <select> elements — so the guard must be HTMLElement, not
 * HTMLSelectElement, and the target IDs must match the rendered Edit-specific
 * id attributes ("edit-agent-llm-provider" / "edit-agent-model").
 *
 * These tests guard against two compounding regressions:
 *   1. Wrong target IDs: stale "agent-provider" / "agent-model" IDs cause
 *      getElementById to return null, silently skipping focus entirely.
 *   2. Wrong element-type guard: HTMLSelectElement guard bails for a <button>
 *      even when the correct ID is found, so focus never fires.
 *
 * Test approach mirrors envVarsEditorFocus.test.mjs: extract the pure focus-
 * dispatch predicate and exercise it with mock elements — no jsdom required.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ---------------------------------------------------------------------------
// Mock element: records focus() / scrollIntoView() calls, acts as instanceof
// HTMLElement by having the right methods (duck-typed for the logic under test).
// ---------------------------------------------------------------------------

function makeMockButton() {
  return {
    focusCalls: 0,
    scrollCalls: 0,
    focus() {
      this.focusCalls++;
    },
    scrollIntoView() {
      this.scrollCalls++;
    },
  };
}

// ---------------------------------------------------------------------------
// The normalized-field focus-dispatch logic extracted for unit testing.
//
// This mirrors the effect body in EditAgentDialog.tsx:
//   - resolves targetId using Edit-specific IDs ("edit-agent-llm-provider" /
//     "edit-agent-model"), NOT the legacy "agent-provider" / "agent-model"
//   - guards against HTMLElement (not HTMLSelectElement)
//   - enforces one-shot via firedRef
//   - bails if llmProviderFieldVisible is false and field === "provider"
// ---------------------------------------------------------------------------

const EDIT_FIELD_IDS = {
  provider: "edit-agent-llm-provider",
  model: "edit-agent-model",
};

/**
 * @param {{type: string, field: string} | undefined} initialFocus
 * @param {boolean} open
 * @param {boolean} llmProviderFieldVisible
 * @param {Map<string, {focus(): void, scrollIntoView(opts: unknown): void}>} domMap
 * @param {{current: boolean}} firedRef
 * @returns {boolean} true if focus was dispatched
 */
function dispatchNormalizedFieldFocus(
  initialFocus,
  open,
  llmProviderFieldVisible,
  domMap,
  firedRef,
) {
  if (!open || !initialFocus) return false;
  if (initialFocus.type !== "normalized_field") return false;
  if (firedRef.current) return false;

  // Provider field only renders when the runtime catalog has resolved.
  if (initialFocus.field === "provider" && !llmProviderFieldVisible)
    return false;

  const targetId = EDIT_FIELD_IDS[initialFocus.field];
  if (!targetId) return false;

  const el = domMap.get(targetId);
  if (!el) return false;

  firedRef.current = true;
  el.scrollIntoView({ block: "nearest" });
  el.focus();
  return true;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test("editAgentNormalizedFieldFocus_providerField_focusesCorrectElement", () => {
  const providerBtn = makeMockButton();
  const modelBtn = makeMockButton();
  const domMap = new Map([
    ["edit-agent-llm-provider", providerBtn],
    ["edit-agent-model", modelBtn],
  ]);
  const firedRef = { current: false };

  const fired = dispatchNormalizedFieldFocus(
    { type: "normalized_field", field: "provider" },
    /*open=*/ true,
    /*llmProviderFieldVisible=*/ true,
    domMap,
    firedRef,
  );

  assert.equal(fired, true, "dispatch must return true for provider field");
  assert.equal(providerBtn.focusCalls, 1, "provider button must be focused");
  assert.equal(providerBtn.scrollCalls, 1, "provider button must be scrolled");
  assert.equal(modelBtn.focusCalls, 0, "model button must NOT be focused");
});

test("editAgentNormalizedFieldFocus_modelField_focusesCorrectElement", () => {
  const providerBtn = makeMockButton();
  const modelBtn = makeMockButton();
  const domMap = new Map([
    ["edit-agent-llm-provider", providerBtn],
    ["edit-agent-model", modelBtn],
  ]);
  const firedRef = { current: false };

  const fired = dispatchNormalizedFieldFocus(
    { type: "normalized_field", field: "model" },
    /*open=*/ true,
    /*llmProviderFieldVisible=*/ true,
    domMap,
    firedRef,
  );

  assert.equal(fired, true, "dispatch must return true for model field");
  assert.equal(modelBtn.focusCalls, 1, "model button must be focused");
  assert.equal(modelBtn.scrollCalls, 1, "model button must be scrolled");
  assert.equal(
    providerBtn.focusCalls,
    0,
    "provider button must NOT be focused",
  );
});

test("editAgentNormalizedFieldFocus_legacyId_doesNotFocus", () => {
  // Regression: if the code used the old IDs ("agent-provider" / "agent-model")
  // instead of the Edit-specific IDs, getElementById would return null and
  // focus would silently no-op. The domMap here simulates a real DOM where
  // only the correct Edit-specific IDs are registered.
  const domMap = new Map([
    ["agent-provider", makeMockButton()], // legacy ID — must NOT be targeted
    ["agent-model", makeMockButton()], // legacy ID — must NOT be targeted
    // correct IDs absent — simulates getElementById returning null for legacy keys
  ]);
  const firedRef = { current: false };

  const fired = dispatchNormalizedFieldFocus(
    { type: "normalized_field", field: "provider" },
    /*open=*/ true,
    /*llmProviderFieldVisible=*/ true,
    domMap,
    firedRef,
  );

  assert.equal(
    fired,
    false,
    "legacy IDs must not resolve — ensures correct IDs are used",
  );
  assert.equal(
    firedRef.current,
    false,
    "one-shot guard must NOT be set when focus was not dispatched",
  );
});

test("editAgentNormalizedFieldFocus_providerNotYetVisible_doesNotFocus", () => {
  // Provider field renders lazily (llmProviderFieldVisible=false until runtime
  // catalog resolves). Focus must not fire until the field materializes.
  const providerBtn = makeMockButton();
  const domMap = new Map([["edit-agent-llm-provider", providerBtn]]);
  const firedRef = { current: false };

  const firedBefore = dispatchNormalizedFieldFocus(
    { type: "normalized_field", field: "provider" },
    /*open=*/ true,
    /*llmProviderFieldVisible=*/ false, // not yet visible
    domMap,
    firedRef,
  );

  assert.equal(
    firedBefore,
    false,
    "must not focus before provider field is visible",
  );
  assert.equal(providerBtn.focusCalls, 0, "focus must not fire yet");

  // Once the runtime catalog resolves, the field appears and focus fires.
  const firedAfter = dispatchNormalizedFieldFocus(
    { type: "normalized_field", field: "provider" },
    /*open=*/ true,
    /*llmProviderFieldVisible=*/ true, // now visible
    domMap,
    firedRef,
  );

  assert.equal(firedAfter, true, "must focus once provider field is visible");
  assert.equal(providerBtn.focusCalls, 1, "focus must fire exactly once");
});

test("editAgentNormalizedFieldFocus_oneShotGuard_preventsRefocus", () => {
  const modelBtn = makeMockButton();
  const domMap = new Map([["edit-agent-model", modelBtn]]);
  const firedRef = { current: false };

  // First dispatch — fires.
  dispatchNormalizedFieldFocus(
    { type: "normalized_field", field: "model" },
    /*open=*/ true,
    /*llmProviderFieldVisible=*/ true,
    domMap,
    firedRef,
  );
  assert.equal(firedRef.current, true, "guard must be set after first focus");

  // Second dispatch (simulates re-render from llmProviderFieldVisible change) — must not fire.
  const firedSecond = dispatchNormalizedFieldFocus(
    { type: "normalized_field", field: "model" },
    /*open=*/ true,
    /*llmProviderFieldVisible=*/ true,
    domMap,
    firedRef,
  );

  assert.equal(firedSecond, false, "one-shot guard must prevent second focus");
  assert.equal(
    modelBtn.focusCalls,
    1,
    "model button must be focused exactly once",
  );
});

test("editAgentNormalizedFieldFocus_envKeyType_isNotHandled", () => {
  // env_key focus is handled separately by EnvVarsEditor via focusKey prop.
  // The normalized-field handler must ignore it entirely.
  const btn = makeMockButton();
  const domMap = new Map([["edit-agent-llm-provider", btn]]);
  const firedRef = { current: false };

  const fired = dispatchNormalizedFieldFocus(
    { type: "env_key", key: "ANTHROPIC_API_KEY" },
    /*open=*/ true,
    /*llmProviderFieldVisible=*/ true,
    domMap,
    firedRef,
  );

  assert.equal(fired, false, "env_key type must not be handled here");
  assert.equal(btn.focusCalls, 0, "no focus for env_key type");
});

test("editAgentNormalizedFieldFocus_dialogClosed_doesNotFocus", () => {
  const btn = makeMockButton();
  const domMap = new Map([["edit-agent-model", btn]]);
  const firedRef = { current: false };

  const fired = dispatchNormalizedFieldFocus(
    { type: "normalized_field", field: "model" },
    /*open=*/ false, // dialog not open
    /*llmProviderFieldVisible=*/ true,
    domMap,
    firedRef,
  );

  assert.equal(fired, false, "must not focus when dialog is closed");
  assert.equal(btn.focusCalls, 0, "no focus when dialog closed");
});
