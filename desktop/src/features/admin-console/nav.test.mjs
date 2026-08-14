/**
 * Unit tests for the Settings → Moderation nav visibility gate.
 *
 * The gate decides whether ordinary members ever see the Moderation entry.
 * Its two load-bearing rules are:
 *  1. A saved manual origin always shows the entry — the Advanced control that
 *     edits/clears a bad saved URL lives inside the surface, so hiding it would
 *     permanently lock a user out of fixing their own state.
 *  2. An advertised-only origin follows the probe: visible on a plausible
 *     authorization or a transport flake, hidden on a definitive non-admin
 *     verdict. Flake must never silently hide the entry.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { shouldShowModerationNav } from "./nav.ts";

test("no-origin-hides-entry: neither advertised nor saved → hidden", () => {
  assert.equal(
    shouldShowModerationNav({ originSource: "none", probe: null }),
    false,
  );
});

test("saved-origin-authorized-shows-entry: saved manual origin visible", () => {
  assert.equal(
    shouldShowModerationNav({
      originSource: "saved",
      probe: "nip98Authorized",
    }),
    true,
  );
});

test("saved-origin-bad-probe-still-shows-entry: notAdminApi under a saved origin stays visible so the user can fix it", () => {
  for (const probe of ["notAdminApi", "nip98Denied", "tokenMode", "error"]) {
    assert.equal(
      shouldShowModerationNav({ originSource: "saved", probe }),
      true,
      `saved origin must stay visible for probe=${probe}`,
    );
  }
});

test("advertised-authorized-shows-entry: advertised origin that authorizes is visible", () => {
  assert.equal(
    shouldShowModerationNav({
      originSource: "advertised",
      probe: "nip98Authorized",
    }),
    true,
  );
});

test("advertised-disabled-shows-entry: advertised origin in auth-disabled mode is visible", () => {
  assert.equal(
    shouldShowModerationNav({ originSource: "advertised", probe: "disabled" }),
    true,
  );
});

test("advertised-flake-shows-entry: transport flake never silently hides an advertised entry", () => {
  for (const probe of ["networkOrIntercepted", "error"]) {
    assert.equal(
      shouldShowModerationNav({ originSource: "advertised", probe }),
      true,
      `advertised origin must stay visible for flake probe=${probe}`,
    );
  }
});

test("advertised-denied-hides-entry: a definitive non-admin verdict hides an advertised-only entry", () => {
  for (const probe of ["nip98Denied", "tokenMode", "notAdminApi"]) {
    assert.equal(
      shouldShowModerationNav({ originSource: "advertised", probe }),
      false,
      `advertised origin must hide for definitive verdict probe=${probe}`,
    );
  }
});

test("advertised-null-probe-hides-entry: fail-closed on an unresolved probe", () => {
  assert.equal(
    shouldShowModerationNav({ originSource: "advertised", probe: null }),
    false,
  );
});
