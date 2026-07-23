/**
 * Tests for the onboarding step-count logic and the BackupStep gating helper
 * (BackupStep now runs in the machine onboarding flow). These are pure-logic
 * tests — no React rendering needed.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { backupNextDisabled } from "./BackupStep.tsx";

// Mirrors the activeSteps array in OnboardingFlow.tsx. The relay-scoped flow
// owns only the community profile: profile → avatar. key-import is normalised
// to "profile" before the indexOf lookup.
const ACTIVE_STEPS = ["profile", "avatar"];
const STEP_OFFSET = 1;

/**
 * Mirrors the currentStep derivation in OnboardingFlow.tsx:
 * profile(1) → avatar(2).
 */
function computeCurrentStep(page) {
  const normalizedPage = page === "key-import" ? "profile" : page;
  const idx = ACTIVE_STEPS.indexOf(normalizedPage);
  return idx >= 0 ? idx + STEP_OFFSET : STEP_OFFSET;
}

function computeTotalSteps() {
  return ACTIVE_STEPS.length;
}

// ---------------------------------------------------------------------------
// Step count and numbering
// ---------------------------------------------------------------------------

test("totalSteps_is_2", () => {
  assert.equal(computeTotalSteps(), 2);
});

test("currentStep_profile_is_1", () => {
  assert.equal(computeCurrentStep("profile"), 1);
});

test("currentStep_key_import_is_1", () => {
  assert.equal(computeCurrentStep("key-import"), 1);
});

test("currentStep_avatar_is_2", () => {
  assert.equal(computeCurrentStep("avatar"), 2);
});

test("currentStep_falls_back_to_1_for_pages_outside_the_step_list", () => {
  assert.equal(computeCurrentStep("membership-denied"), 1);
});

// ---------------------------------------------------------------------------
// BackupStep gating: backupNextDisabled() pure helper
// ---------------------------------------------------------------------------

test("backup_next_disabled_while_loading", () => {
  // During a slow keychain read, Next must be blocked — user cannot race past
  // the key display before it is shown.
  assert.equal(backupNextDisabled({ isLoading: true, loadError: null }), true);
});

test("backup_next_disabled_on_load_error", () => {
  // Error state: only the explicit "Skip for now" ghost advances; Next blocked.
  assert.equal(
    backupNextDisabled({ isLoading: false, loadError: "IPC error" }),
    true,
  );
});

test("backup_next_enabled_after_clean_load", () => {
  // Key shown (or backend cleanly returned none) — user may proceed.
  assert.equal(
    backupNextDisabled({ isLoading: false, loadError: null }),
    false,
  );
});

// ---------------------------------------------------------------------------
// Avatar skip button visibility logic
// ---------------------------------------------------------------------------

test("always_skip_shows_skip_button_when_no_error", () => {
  const showAlwaysSkip = true;
  const errorMessage = null;
  const canSkipForNow = false;
  const showSkip = canSkipForNow || (showAlwaysSkip && errorMessage === null);
  assert.equal(showSkip, true);
});

test("always_skip_hides_skip_button_when_error_is_present", () => {
  // On error, the error-recovery buttons take over (canAdvanceWithoutSaving)
  const showAlwaysSkip = true;
  const errorMessage = "Save failed";
  const canSkipForNow = false;
  const showSkip = canSkipForNow || (showAlwaysSkip && errorMessage === null);
  assert.equal(showSkip, false);
});

test("error_recovery_shows_skip_button_regardless_of_always_skip", () => {
  const showAlwaysSkip = false;
  const errorMessage = null;
  const canSkipForNow = true;
  const showSkip = canSkipForNow || (showAlwaysSkip && errorMessage === null);
  assert.equal(showSkip, true);
});

test("skip_button_hidden_when_no_error_and_always_skip_false", () => {
  const showAlwaysSkip = false;
  const errorMessage = null;
  const canSkipForNow = false;
  const showSkip = canSkipForNow || (showAlwaysSkip && errorMessage === null);
  assert.equal(showSkip, false);
});
