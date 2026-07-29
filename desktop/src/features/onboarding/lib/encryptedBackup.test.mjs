/**
 * Pure-logic tests for the encrypted-backup (NIP-49) creation state model.
 * These drive the same reducer + validation helpers the DownloadKeyStep and
 * settings row use, without a DOM.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  MIN_PASSPHRASE_LEN,
  downloadDisabled,
  isEncrypting,
  passphraseIssue,
  pendingEncryptPassphrase,
  effectivePassphrase,
  encryptedBackupReducer,
  initialEncryptedBackupState,
} from "./encryptedBackup.ts";

function reduce(events, from = initialEncryptedBackupState) {
  return events.reduce(encryptedBackupReducer, from);
}

// ── passphrase validation ────────────────────────────────────────────────────

test("download_disabled_until_passphrase_meets_min_length", () => {
  assert.equal(downloadDisabled(initialEncryptedBackupState), true);

  const short = reduce([{ type: "set-passphrase", value: "short" }]);
  assert.equal(effectivePassphrase(short), null);
  assert.equal(downloadDisabled(short), true);

  const ready = reduce([
    { type: "set-passphrase", value: "regency-dawes-bilal-sit" },
  ]);
  assert.equal(effectivePassphrase(ready), "regency-dawes-bilal-sit");
  assert.equal(downloadDisabled(ready), false);
});

test("passphrase_issue_messages", () => {
  // Empty input: no scolding while the user hasn't typed anything.
  assert.equal(passphraseIssue(""), null);
  assert.match(passphraseIssue("short"), new RegExp(`${MIN_PASSPHRASE_LEN}`));
  assert.equal(passphraseIssue("a".repeat(MIN_PASSPHRASE_LEN)), null);
});

test("min_length_counts_code_points_not_utf16_units", () => {
  // 12 astral-plane emoji = 24 UTF-16 units but 12 code points; mirrors the
  // Rust chars().count() gate so both sides agree on the boundary.
  const emoji = "😀".repeat(MIN_PASSPHRASE_LEN);
  assert.equal(passphraseIssue(emoji), null);
  const ready = reduce([{ type: "set-passphrase", value: emoji }]);
  assert.equal(effectivePassphrase(ready), emoji);
});

test("editing_passphrase_clears_create_error", () => {
  const failed = reduce([
    { type: "set-passphrase", value: "regency-dawes-bilal-sit" },
    { type: "encrypt-started", passphrase: "regency-dawes-bilal-sit" },
    {
      type: "encrypt-failed",
      passphrase: "regency-dawes-bilal-sit",
      message: "keychain unavailable",
    },
  ]);
  assert.equal(failed.createError, "keychain unavailable");
  const edited = reduce(
    [{ type: "set-passphrase", value: "regency-dawes-bilal-sat" }],
    failed,
  );
  assert.equal(edited.createError, null);
});

// ── eager encryption lifecycle ───────────────────────────────────────────────

test("valid_passphrase_requests_background_encryption", () => {
  assert.equal(pendingEncryptPassphrase(initialEncryptedBackupState), null);

  const short = reduce([{ type: "set-passphrase", value: "short" }]);
  assert.equal(pendingEncryptPassphrase(short), null);

  const ready = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
  ]);
  assert.equal(pendingEncryptPassphrase(ready), "one-two-three-four");

  const started = reduce(
    [{ type: "encrypt-started", passphrase: "one-two-three-four" }],
    ready,
  );
  assert.equal(isEncrypting(started), true);
  assert.equal(
    pendingEncryptPassphrase(started),
    null,
    "no duplicate start while the KDF runs",
  );

  const done = reduce(
    [
      {
        type: "encrypt-succeeded",
        passphrase: "one-two-three-four",
        ncryptsec: "ncryptsec1abc",
      },
    ],
    started,
  );
  assert.equal(isEncrypting(done), false);
  assert.equal(pendingEncryptPassphrase(done), null, "result cached");
  assert.equal(done.ncryptsec, null, "nothing commits before Download");
});

test("stale_encryption_results_are_dropped_and_retriggered", () => {
  const editedMidFlight = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", passphrase: "one-two-three-four" },
    { type: "set-passphrase", value: "five-six-seven-eight" },
  ]);
  // The new passphrase needs its own run even while the old one is in flight.
  assert.equal(
    pendingEncryptPassphrase(editedMidFlight),
    "five-six-seven-eight",
  );

  const staleLanded = reduce(
    [
      { type: "encrypt-started", passphrase: "five-six-seven-eight" },
      {
        type: "encrypt-succeeded",
        passphrase: "one-two-three-four",
        ncryptsec: "ncryptsec1stale",
      },
    ],
    editedMidFlight,
  );
  assert.equal(staleLanded.encrypted.passphrase, "one-two-three-four");
  assert.equal(staleLanded.ncryptsec, null, "stale result never commits");
  assert.equal(
    isEncrypting(staleLanded),
    true,
    "current passphrase still encrypting",
  );

  const staleFailed = reduce(
    [
      {
        type: "encrypt-failed",
        passphrase: "one-two-three-four",
        message: "boom",
      },
    ],
    editedMidFlight,
  );
  assert.equal(staleFailed.createError, null, "stale failures are silent");
});

// ── download commit ──────────────────────────────────────────────────────────

test("download_commits_instantly_when_encryption_is_done", () => {
  const state = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", passphrase: "one-two-three-four" },
    {
      type: "encrypt-succeeded",
      passphrase: "one-two-three-four",
      ncryptsec: "ncryptsec1abc",
    },
    { type: "download-clicked" },
  ]);
  assert.equal(state.ncryptsec, "ncryptsec1abc");
  assert.equal(state.downloadPending, false);
});

test("download_during_encryption_queues_then_commits_on_success", () => {
  const queued = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", passphrase: "one-two-three-four" },
    { type: "download-clicked" },
  ]);
  assert.equal(queued.downloadPending, true);
  assert.equal(queued.ncryptsec, null);
  assert.equal(downloadDisabled(queued), true, "no double-click while queued");

  const committed = reduce(
    [
      {
        type: "encrypt-succeeded",
        passphrase: "one-two-three-four",
        ncryptsec: "ncryptsec1abc",
      },
    ],
    queued,
  );
  assert.equal(committed.ncryptsec, "ncryptsec1abc");
  assert.equal(committed.downloadPending, false);
});

test("queued_download_clears_on_failure_so_user_can_retry", () => {
  const failed = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", passphrase: "one-two-three-four" },
    { type: "download-clicked" },
    {
      type: "encrypt-failed",
      passphrase: "one-two-three-four",
      message: "keychain unavailable",
    },
  ]);
  assert.equal(failed.downloadPending, false);
  assert.equal(failed.createError, "keychain unavailable");
  assert.equal(downloadDisabled(failed), false, "retry allowed after failure");
});

test("back_to_password_discards_commit_but_keeps_cached_encryption", () => {
  const back = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "encrypt-started", passphrase: "one-two-three-four" },
    {
      type: "encrypt-succeeded",
      passphrase: "one-two-three-four",
      ncryptsec: "ncryptsec1abc",
    },
    { type: "download-clicked" },
    { type: "back-to-password" },
  ]);
  assert.equal(back.ncryptsec, null, "committed blob discarded");
  assert.equal(back.passphrase, "one-two-three-four", "passphrase kept");
  assert.equal(
    pendingEncryptPassphrase(back),
    null,
    "cached result means no re-encryption",
  );

  // Re-downloading the unchanged passphrase commits instantly from cache.
  const recommitted = reduce([{ type: "download-clicked" }], back);
  assert.equal(recommitted.ncryptsec, "ncryptsec1abc");
});

test("download_click_before_encryption_starts_still_queues", () => {
  // Click lands inside the debounce window: nothing started yet, but the
  // pending flag makes the eventual result commit.
  const queued = reduce([
    { type: "set-passphrase", value: "one-two-three-four" },
    { type: "download-clicked" },
  ]);
  assert.equal(queued.downloadPending, true);
  assert.equal(
    pendingEncryptPassphrase(queued),
    "one-two-three-four",
    "host still needs to start the KDF",
  );
});
