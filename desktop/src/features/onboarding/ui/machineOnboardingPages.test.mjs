/**
 * Tests for the machine onboarding page graph (onboarding v2): chrome
 * positions on the pagination track and the setup Back target. Pure-logic
 * tests — no React rendering needed.
 *
 * The page names asserted here are a cross-PR contract (the advanced-entry
 * PR builds against `advanced-identity`; the signup/login PR fills `signup`
 * and `login`) — renames must be coordinated in #buzz-onboarding-v2.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  machineOnboardingChromePosition,
  machineSetupBackTarget,
} from "./machineOnboardingPages.ts";

// ---------------------------------------------------------------------------
// Chrome positions
// ---------------------------------------------------------------------------

test("landing_renders_no_chrome", () => {
  assert.equal(machineOnboardingChromePosition("identity"), null);
});

test("account_entry_screens_share_position_1", () => {
  // Signup, login, and the no-email entry are branches of the same flow
  // position — the track counts positions, not branches.
  assert.equal(machineOnboardingChromePosition("signup"), 1);
  assert.equal(machineOnboardingChromePosition("login"), 1);
  assert.equal(machineOnboardingChromePosition("advanced-identity"), 1);
});

test("key_screens_share_position_2", () => {
  assert.equal(machineOnboardingChromePosition("key-import"), 2);
  assert.equal(machineOnboardingChromePosition("backup"), 2);
});

test("setup_is_position_3_and_config_is_4", () => {
  assert.equal(machineOnboardingChromePosition("setup"), 3);
  assert.equal(machineOnboardingChromePosition("config"), 4);
});

// ---------------------------------------------------------------------------
// Setup Back target — import-skips-backup asymmetry
// ---------------------------------------------------------------------------

test("setup_back_returns_to_key_import_for_imported_identities", () => {
  // Imported keys skip the backup step on the way in (the user already has
  // their key saved); Back must never surface backup for them.
  assert.equal(machineSetupBackTarget(true), "key-import");
});

test("setup_back_returns_to_backup_for_fresh_identities", () => {
  assert.equal(machineSetupBackTarget(false), "backup");
});
