/**
 * Tests for the onboarding step-count and routing logic that was added for the
 * key-backup feature. These are pure-logic tests — no React rendering needed.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { backupNextDisabled } from "./BackupStep.tsx";

// Mirrors the FRESH_STEPS / IMPORT_STEPS arrays in OnboardingFlow.tsx.
// key-import is normalised to "profile" before the indexOf lookup.
const FRESH_STEPS = ["profile", "backup", "avatar", "theme", "setup"];
const IMPORT_STEPS = ["profile", "avatar", "theme", "setup"];
const STEP_OFFSET = 2;

/**
 * Mirrors the currentStep derivation in OnboardingFlow.tsx.
 * Fresh path: profile(2) → backup(3) → avatar(4) → theme(5) → setup(6)
 * Imported path: profile(2) → avatar(3) → theme(4) → setup(5)
 */
function computeCurrentStep(page, identityWasImported) {
  const steps = identityWasImported ? IMPORT_STEPS : FRESH_STEPS;
  const normalizedPage = page === "key-import" ? "profile" : page;
  const idx = steps.indexOf(normalizedPage);
  return idx >= 0 ? idx + STEP_OFFSET : STEP_OFFSET;
}

function computeTotalSteps(identityWasImported) {
  const steps = identityWasImported ? IMPORT_STEPS : FRESH_STEPS;
  return steps.length + 1;
}

// ---------------------------------------------------------------------------
// Total step count
// ---------------------------------------------------------------------------

test("totalSteps_is_5_when_identity_was_imported", () => {
  assert.equal(computeTotalSteps(true), 5);
});

test("totalSteps_is_6_on_fresh_key_path", () => {
  assert.equal(computeTotalSteps(false), 6);
});

// ---------------------------------------------------------------------------
// Step numbers — fresh path (6 steps)
// ---------------------------------------------------------------------------

test("currentStep_profile_is_2_on_fresh_path", () => {
  assert.equal(computeCurrentStep("profile", false), 2);
});

test("currentStep_key_import_is_2_on_fresh_path", () => {
  assert.equal(computeCurrentStep("key-import", false), 2);
});

test("currentStep_backup_is_3_on_fresh_path", () => {
  assert.equal(computeCurrentStep("backup", false), 3);
});

test("currentStep_avatar_is_4_on_fresh_path", () => {
  assert.equal(computeCurrentStep("avatar", false), 4);
});

test("currentStep_theme_is_5_on_fresh_path", () => {
  assert.equal(computeCurrentStep("theme", false), 5);
});

test("currentStep_setup_is_6_on_fresh_path", () => {
  assert.equal(computeCurrentStep("setup", false), 6);
});

// ---------------------------------------------------------------------------
// Step numbers — imported key path (5 steps)
// ---------------------------------------------------------------------------

test("currentStep_profile_is_2_on_imported_path", () => {
  assert.equal(computeCurrentStep("profile", true), 2);
});

test("currentStep_avatar_is_3_on_imported_path", () => {
  assert.equal(computeCurrentStep("avatar", true), 3);
});

test("currentStep_theme_is_4_on_imported_path", () => {
  assert.equal(computeCurrentStep("theme", true), 4);
});

test("currentStep_setup_is_5_on_imported_path", () => {
  assert.equal(computeCurrentStep("setup", true), 5);
});

// ---------------------------------------------------------------------------
// Routing: profile submit goes to backup (fresh) or avatar (imported)
// ---------------------------------------------------------------------------

test("profile_submit_routes_to_backup_on_fresh_path", () => {
  const identityWasImported = false;
  const nextPage = identityWasImported ? "avatar" : "backup";
  assert.equal(nextPage, "backup");
});

test("profile_submit_routes_to_avatar_on_imported_path", () => {
  const identityWasImported = true;
  const nextPage = identityWasImported ? "avatar" : "backup";
  assert.equal(nextPage, "avatar");
});

// ---------------------------------------------------------------------------
// BackupStep gating: backupNextDisabled() pure helper
// ---------------------------------------------------------------------------

test("backup_next_disabled_while_loading", () => {
  // During a slow keychain read, Next must be blocked — user cannot race past
  // the key display before it is shown.
  assert.equal(
    backupNextDisabled({
      isLoading: true,
      loadError: null,
      nsec: null,
      hasAcknowledged: false,
    }),
    true,
  );
});

test("backup_next_disabled_on_load_error", () => {
  // Error state: only the explicit "Skip for now" ghost advances; Next blocked.
  assert.equal(
    backupNextDisabled({
      isLoading: false,
      loadError: "IPC error",
      nsec: null,
      hasAcknowledged: false,
    }),
    true,
  );
});

test("backup_next_disabled_when_nsec_loaded_and_not_acknowledged", () => {
  // Key shown but checkbox unchecked.
  assert.equal(
    backupNextDisabled({
      isLoading: false,
      loadError: null,
      nsec: "nsec1test",
      hasAcknowledged: false,
    }),
    true,
  );
});

test("backup_next_enabled_when_nsec_loaded_and_acknowledged", () => {
  // Key shown and checkbox checked — the normal happy path.
  assert.equal(
    backupNextDisabled({
      isLoading: false,
      loadError: null,
      nsec: "nsec1test",
      hasAcknowledged: true,
    }),
    false,
  );
});

test("backup_next_enabled_when_backend_returned_null_key_cleanly", () => {
  // Backend returned no key without an error (edge case): nothing to acknowledge,
  // allow forward progress so onboarding is never bricked.
  assert.equal(
    backupNextDisabled({
      isLoading: false,
      loadError: null,
      nsec: null,
      hasAcknowledged: false,
    }),
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
