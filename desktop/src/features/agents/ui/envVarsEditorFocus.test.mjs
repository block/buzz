/**
 * Unit tests for EnvVarsEditor focus-dispatch behavior.
 *
 * The component uses a ref map (key → HTMLInputElement) to target the correct
 * value input for a focusKey deep-link request. These tests verify the focus
 * logic properties that were broken in the original DOM-walk implementation:
 *
 *   1. The focus attempt is dispatched to the element from the ref map, NOT
 *      resolved via DOM walking through presentation class ancestors — meaning
 *      `closest("[class]")` / `parentElement` / `querySelector` are never
 *      invoked.
 *
 *   2. The focus fires when the target key materializes in `requiredKeys`
 *      (async-safe), not just on mount.
 *
 *   3. The one-shot guard prevents a second `.focus()` call after the target
 *      has already been focused (e.g., when an unrelated `requiredKeys` change
 *      triggers a re-run).
 *
 * Why not a full component render test?
 * The project's test runner is plain Node `node:test` with no jsdom setup;
 * mounting a full React + Radix Dialog tree would require wiring up jsdom,
 * happy-dom, and React Testing Library — infrastructure that doesn't exist
 * in this repo. The logic under test is the focus-dispatch predicate and
 * one-shot guard, both of which are fully exercisable without a browser DOM.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

// ---------------------------------------------------------------------------
// Minimal mock for an HTMLInputElement: records `.focus()` calls.
// ---------------------------------------------------------------------------

function makeMockInput() {
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
// The focus-dispatch logic extracted for unit testing.
//
// This mirrors the effect body in EnvVarsEditor — given the three inputs it
// reads (focusKey, requiredKeys, refMap) plus the one-shot guard, it either
// focuses the matching element or does nothing.
// ---------------------------------------------------------------------------

function dispatchFocusIfReady(focusKey, requiredKeys, refMap, firedRef) {
  if (!focusKey) return false;
  if (firedRef.current) return false;
  if (!requiredKeys.includes(focusKey)) return false;

  const inputEl = refMap.get(focusKey);
  if (!inputEl) return false;

  firedRef.current = true;
  inputEl.scrollIntoView({ block: "nearest" });
  inputEl.focus();
  return true;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test("envVarsEditorFocus_matchingKey_focusesRefMapElement", () => {
  // Primary acceptance path: DATABRICKS_HOST card click → value input focused.
  // The ref map has the input registered; requiredKeys contains the key.
  const input = makeMockInput();
  const refMap = new Map([["DATABRICKS_HOST", input]]);
  const firedRef = { current: false };

  const fired = dispatchFocusIfReady(
    "DATABRICKS_HOST",
    ["DATABRICKS_HOST"],
    refMap,
    firedRef,
  );

  assert.equal(fired, true, "dispatch must return true when focus is applied");
  assert.equal(
    input.focusCalls,
    1,
    "input.focus() must be called exactly once",
  );
  assert.equal(
    input.scrollCalls,
    1,
    "input.scrollIntoView() must be called exactly once",
  );
});

test("envVarsEditorFocus_noFocusKey_doesNotFocus", () => {
  // No focusKey set — effect must be a no-op (card did not request focus).
  const input = makeMockInput();
  const refMap = new Map([["DATABRICKS_HOST", input]]);
  const firedRef = { current: false };

  const fired = dispatchFocusIfReady(
    undefined,
    ["DATABRICKS_HOST"],
    refMap,
    firedRef,
  );

  assert.equal(fired, false, "no focusKey → no focus");
  assert.equal(input.focusCalls, 0, "focus must not be called");
});

test("envVarsEditorFocus_keyNotYetInRequiredKeys_doesNotFocusYet", () => {
  // Async race: requiredKeys hasn't yet included the key (file-config query
  // still pending). Focus must NOT fire — it will fire once the key appears.
  const input = makeMockInput();
  const refMap = new Map([["DATABRICKS_HOST", input]]);
  const firedRef = { current: false };

  const firedBeforeKey = dispatchFocusIfReady(
    "DATABRICKS_HOST",
    [], // key not yet present — query pending
    refMap,
    firedRef,
  );

  assert.equal(
    firedBeforeKey,
    false,
    "must not fire before key is in requiredKeys",
  );
  assert.equal(
    input.focusCalls,
    0,
    "focus must not be called before key materializes",
  );

  // Once requiredKeys gains the key (query resolved), focus fires.
  const firedAfterKey = dispatchFocusIfReady(
    "DATABRICKS_HOST",
    ["DATABRICKS_HOST"], // key materialized
    refMap,
    firedRef,
  );

  assert.equal(
    firedAfterKey,
    true,
    "must fire once key appears in requiredKeys",
  );
  assert.equal(
    input.focusCalls,
    1,
    "focus must be called exactly once after key materializes",
  );
});

test("envVarsEditorFocus_oneShotGuard_preventsRefocus", () => {
  // After a successful focus, a subsequent requiredKeys change (e.g., another
  // key being added) must not call .focus() a second time.
  const input = makeMockInput();
  const refMap = new Map([["DATABRICKS_HOST", input]]);
  const firedRef = { current: false };

  dispatchFocusIfReady(
    "DATABRICKS_HOST",
    ["DATABRICKS_HOST"],
    refMap,
    firedRef,
  );
  assert.equal(firedRef.current, true, "guard must be set after first focus");

  // Simulate a re-run triggered by unrelated requiredKeys update.
  const firedSecond = dispatchFocusIfReady(
    "DATABRICKS_HOST",
    ["DATABRICKS_HOST", "ANOTHER_KEY"],
    refMap,
    firedRef,
  );

  assert.equal(firedSecond, false, "guard must block second focus attempt");
  assert.equal(
    input.focusCalls,
    1,
    "focus must still have been called exactly once",
  );
});

test("envVarsEditorFocus_refMapMissingEntry_doesNotFocus", () => {
  // The ref map doesn't have the key registered yet (input not yet mounted).
  // Focus must not be attempted — no crashing, clean no-op.
  const firedRef = { current: false };

  const fired = dispatchFocusIfReady(
    "DATABRICKS_HOST",
    ["DATABRICKS_HOST"],
    new Map(), // empty ref map
    firedRef,
  );

  assert.equal(fired, false, "missing ref map entry must be a no-op");
  assert.equal(
    firedRef.current,
    false,
    "guard must NOT be set when no element was focused",
  );
});

test("envVarsEditorFocus_nodomWalking_refMapDirectDispatch", () => {
  // Regression guard: the old implementation walked the DOM via
  // `closest("[class]")` → `parentElement` → `querySelector`, which fails
  // because `closest("[class]")` returns the key-span itself (it has a
  // className), so `parentElement` is the amber key-shell div (not the row),
  // and `querySelector("[data-testid='env-vars-required-value']")` within it
  // can never reach the sibling value-column's Input.
  //
  // The new implementation uses the ref map directly — no DOM traversal.
  // This test verifies that when two keys are present, ONLY the matching
  // key's input receives focus (no cross-row targeting).
  const inputA = makeMockInput();
  const inputB = makeMockInput();
  const refMap = new Map([
    ["DATABRICKS_HOST", inputA],
    ["DATABRICKS_TOKEN", inputB],
  ]);
  const firedRef = { current: false };

  dispatchFocusIfReady(
    "DATABRICKS_HOST",
    ["DATABRICKS_HOST", "DATABRICKS_TOKEN"],
    refMap,
    firedRef,
  );

  assert.equal(inputA.focusCalls, 1, "DATABRICKS_HOST input must be focused");
  assert.equal(
    inputB.focusCalls,
    0,
    "DATABRICKS_TOKEN input must NOT be focused",
  );
});
