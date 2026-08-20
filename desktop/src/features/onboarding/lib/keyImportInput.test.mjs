/**
 * Pure-logic tests for key-import input classification (nsec vs NIP-49
 * ncryptsec) and submit gating.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { nsecEncode } from "nostr-tools/nip19";
import { generateSecretKey } from "nostr-tools/pure";
import {
  classifyKeyImportInput,
  isPlausibleNcryptsec,
  keyImportSubmitEnabled,
  NCRYPTSEC_ENCODED_LENGTH,
} from "./keyImportInput.ts";

// NIP-49 spec vector — structurally valid encrypted backup.
const NCRYPTSEC =
  "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";

const VALID_NSEC = nsecEncode(generateSecretKey());

test("classify_by_hrp_with_whitespace_tolerance", () => {
  assert.equal(classifyKeyImportInput(`  ${NCRYPTSEC}\n`), "ncryptsec");
  assert.equal(classifyKeyImportInput(VALID_NSEC), "nsec");
  assert.equal(classifyKeyImportInput("npub1whatever"), "unknown");
  assert.equal(classifyKeyImportInput(""), "unknown");
  // nsec must not be shadowed by the longer HRP check.
  assert.equal(classifyKeyImportInput("nsec1"), "nsec");
});

test("uppercase_bech32_encoding_classifies_and_gates_like_lowercase", () => {
  // Bech32 permits an all-uppercase encoding; it must route to the
  // encrypted path (matching Rust) and be submit-plausible.
  const upper = NCRYPTSEC.toUpperCase();
  assert.equal(classifyKeyImportInput(upper), "ncryptsec");
  assert.equal(isPlausibleNcryptsec(upper), true);
  assert.equal(keyImportSubmitEnabled(upper, ""), false);
  assert.equal(keyImportSubmitEnabled(upper, "hunter2hunter2"), true);
  // Mixed case: routed encrypted (Rust reports the accurate error) but
  // never plausible/submittable — mixed-case bech32 cannot decode.
  const mixed = `N${NCRYPTSEC.slice(1)}`;
  assert.equal(classifyKeyImportInput(mixed), "ncryptsec");
  assert.equal(isPlausibleNcryptsec(mixed), false);
  assert.equal(keyImportSubmitEnabled(mixed, "hunter2hunter2"), false);
});

test("plausible_ncryptsec_requires_complete_checksummed_nip49_payload", () => {
  assert.equal(NCRYPTSEC.length, NCRYPTSEC_ENCODED_LENGTH);
  assert.equal(isPlausibleNcryptsec(NCRYPTSEC), true);
  assert.equal(isPlausibleNcryptsec(`  ${NCRYPTSEC}\n`), true);
  assert.equal(isPlausibleNcryptsec(NCRYPTSEC.slice(0, -1)), false);
  assert.equal(isPlausibleNcryptsec(`${NCRYPTSEC}q`), false);
  // Same length and charset, but a changed checksum must not advance the UI.
  assert.equal(isPlausibleNcryptsec(`${NCRYPTSEC.slice(0, -1)}q`), false);
  // '1' and 'b' / 'i' / 'o' are not in the Bech32 data charset.
  assert.equal(isPlausibleNcryptsec("ncryptsec1bio"), false);
  assert.equal(isPlausibleNcryptsec("ncryptsec1"), false);
  assert.equal(isPlausibleNcryptsec("ncryptsec1 with spaces"), false);
});

test("submit_gating_nsec_path_unchanged", () => {
  assert.equal(keyImportSubmitEnabled(VALID_NSEC, ""), true);
  assert.equal(keyImportSubmitEnabled("nsec1garbage", ""), false);
  assert.equal(keyImportSubmitEnabled("", ""), false);
});

test("submit_gating_ncryptsec_requires_passphrase", () => {
  assert.equal(keyImportSubmitEnabled(NCRYPTSEC, ""), false);
  assert.equal(keyImportSubmitEnabled(NCRYPTSEC, "hunter2hunter2"), true);
  // Structurally implausible blob never submits, passphrase or not.
  assert.equal(keyImportSubmitEnabled("ncryptsec1bio", "hunter2"), false);
});
